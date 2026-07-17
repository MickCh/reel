use anyhow::{Result, bail};

use crate::http::{HttpClient, PermanentError, permanent};
use crate::model::{
    Cookie, Request, ResponseRecord, State, cookie_header, parse_set_cookie, update_jar,
};
use crate::session::{PresetStore, SessionStore};
use crate::template::{Context, apply_interpolation, check_condition};

use super::commands::{Command, GlobalFlags, ParseResult, ResponseTarget, ResponseView};
use super::display::{format_curl, format_size, format_status, print_body, print_dry_run};

// Host, path, and https-ness of the request URL — the context needed for
// cookie matching. None when the URL is unset or unparseable (in which case
// cookie handling is skipped; the HTTP layer reports the bad URL itself).
fn url_parts(url: Option<&str>) -> Option<(String, String, bool)> {
    let parsed = url::Url::parse(url?).ok()?;
    let host = parsed.host_str()?.to_lowercase();
    Some((host, parsed.path().to_string(), parsed.scheme() == "https"))
}

// The request as it will actually be sent: the user's request plus the Cookie
// header assembled from the session jar. A user-set Cookie header always wins.
fn with_session_cookies(request: &Request, jar: &[Cookie]) -> Request {
    let mut request = request.clone();
    if request.headers.get("cookie").is_none()
        && let Some((host, path, https)) = url_parts(request.url.as_deref())
        && let Some(header) = cookie_header(jar, &host, &path, https)
    {
        request.headers.insert("Cookie".to_string(), header);
    }
    request
}

// Resolve the value of a `body` command: '-' reads stdin, '@path' reads a
// file (verbatim, like curl's --data-binary), '@@...' escapes a literal body
// starting with '@'. Resolution happens here rather than in parse_args, which
// must stay I/O-free.
fn resolve_body(value: &str) -> Result<String> {
    if value == "-" {
        let mut buf = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut buf)
            .map_err(|e| anyhow::anyhow!("error reading body from stdin: {}", e))?;
        return Ok(buf);
    }
    if let Some(rest) = value.strip_prefix('@') {
        if rest.starts_with('@') {
            return Ok(rest.to_string());
        }
        return std::fs::read_to_string(rest)
            .map_err(|e| anyhow::anyhow!("error reading body file '{}': {}", rest, e));
    }
    Ok(value.to_string())
}

// 5xx statuses worth retrying. 501/505/506/510 describe a permanent property
// of the server or the request, so a retry cannot change the outcome.
fn transient_5xx(status: u16) -> bool {
    status >= 500 && !matches!(status, 501 | 505 | 506 | 510)
}

// Redirect-hop cap for --follow (reqwest's own default).
const MAX_REDIRECTS: u32 = 10;

// Parse the raw Set-Cookie values of a response into the session jar. Every
// hop and every retry attempt goes through here — servers rotate session
// cookies mid-chain. Marking the state modified lets the end-of-run
// best-effort save keep the cookies even when the chain later aborts.
fn absorb_cookies(state: &mut State, chain: &mut ChainState, url: Option<&str>, raw: &[String]) {
    if let Some((host, path, _)) = url_parts(url) {
        for value in raw {
            if let Some(update) = parse_set_cookie(value, &host, &path) {
                update_jar(&mut state.cookies, update);
                chain.modified = true;
            }
        }
    }
}

// Given a redirect response, derive the next request from the current one
// (curl -L semantics), or None when the response is not a followable
// redirect. 303 switches to GET (a HEAD stays HEAD); 301/302 do so only for
// POST; 307/308 preserve method and body. When the body is dropped, its
// Content-Type goes with it. On a cross-host redirect the user-set
// Authorization and Cookie headers are stripped — credentials must not ride
// to a different host (the session jar re-scopes itself per hop anyway).
fn redirect_target(current: &Request, status: u16, location: Option<&str>) -> Option<Request> {
    if !matches!(status, 301 | 302 | 303 | 307 | 308) {
        return None;
    }
    let location = location?;
    let base = url::Url::parse(current.url.as_deref()?).ok()?;
    let next_url = base.join(location).ok()?;

    let mut next = current.clone();
    let method = current.method.as_deref().unwrap_or("GET");
    let to_get = (status == 303 && !method.eq_ignore_ascii_case("HEAD"))
        || (matches!(status, 301 | 302) && method.eq_ignore_ascii_case("POST"));
    if to_get {
        next.method = Some("GET".to_string());
        next.body = None;
        next.headers.remove("content-type");
    }
    if base.host_str() != next_url.host_str() {
        next.headers.remove("authorization");
        next.headers.remove("cookie");
    }
    next.url = Some(next_url.to_string());
    Some(next)
}

// One logical exchange: execute the request and, under --follow, chase 3xx
// redirects. Cookies from every hop enter the jar; only the final hop's
// request/response pair is returned (and later recorded) — like curl -L.
fn execute_following(
    state: &mut State,
    flags: &GlobalFlags,
    http: &dyn HttpClient,
    source: Option<&str>,
    chain: &mut ChainState,
) -> Result<(Request, ResponseRecord)> {
    // `base` holds the user-visible request for the current hop, without the
    // jar-injected Cookie header — that header is re-derived per hop so each
    // redirect target gets exactly the cookies scoped to it.
    let mut base = state.request.clone();
    let mut hops = 0u32;
    loop {
        let request = with_session_cookies(&base, &state.cookies);
        let record = http.execute(&request, source)?;
        absorb_cookies(state, chain, request.url.as_deref(), &record.set_cookies);
        if !flags.follow {
            return Ok((request, record));
        }
        let location = record.headers.get("location").map(str::to_string);
        match redirect_target(&base, record.status, location.as_deref()) {
            Some(next) => {
                hops += 1;
                if hops > MAX_REDIRECTS {
                    // Deterministic loop — retrying cannot help.
                    return Err(permanent(format!(
                        "error: too many redirects (more than {})",
                        MAX_REDIRECTS
                    )));
                }
                eprintln!(
                    "following redirect -> {} ({})",
                    next.url.as_deref().unwrap_or("?"),
                    format_status(record.status)
                );
                base = next;
            }
            None => return Ok((request, record)),
        }
    }
}

// Shared execution path of `send` and `then`: run the request, record it in
// the history, persist the session, and print the result. `fresh_history` is
// the `send` semantics — the previous history is replaced, but only once an
// attempt actually produced a response, so a failed send cannot wipe it.
// A 4xx/5xx result is remembered in `chain` for the deferred `fail` verdict.
// `session.save()` must come before any stdout write — broken-pipe safety.
fn execute_and_record(
    state: &mut State,
    flags: &GlobalFlags,
    http: &dyn HttpClient,
    session: &dyn SessionStore,
    source: Option<&str>,
    fresh_history: bool,
    chain: &mut ChainState,
) -> Result<()> {
    if state.request.body.is_some() && state.request.headers.get("content-type").is_none() {
        eprintln!("warning: body is set but Content-Type header is missing");
    }
    let cleared_note =
        (fresh_history && !state.responses.is_empty()).then_some("previous responses cleared");

    // Attempt budget: --retry N gives N extra attempts; polling with --until
    // alone gets a default budget of 10 attempts. An explicit `--retry 0`
    // (unlike an absent flag) caps the poll at a single attempt.
    let max_attempts = match (&flags.until, flags.retry) {
        (Some(_), None) => 10,
        (_, retry) => retry.unwrap_or(0).saturating_add(1),
    };
    let mut attempt = 1u32;
    // Set when --until exhausts its budget: the last response is still
    // recorded and printed, but the chain aborts afterwards.
    let mut until_error = None;

    let (request, record) = loop {
        // The request is rebuilt each attempt (inside execute_following): a
        // Set-Cookie received on a retried attempt must be reflected in the
        // next one.
        let reason = match execute_following(state, flags, http, source, chain) {
            Ok((request, record)) => {
                if let Some(condition) = &flags.until {
                    // Evaluate against the history as it would look with this
                    // attempt appended — both vectors, so `request.*` refers
                    // to the in-flight request and indices stay aligned. A
                    // send starts a fresh history, so only the candidate is
                    // visible to the condition.
                    let (mut requests, mut responses) = if fresh_history {
                        (Vec::new(), Vec::new())
                    } else {
                        (state.requests.clone(), state.responses.clone())
                    };
                    requests.push(request.clone());
                    responses.push(record.clone());
                    let ctx = Context {
                        requests: &requests,
                        responses: &responses,
                        vars: &state.vars,
                    };
                    if check_condition(condition, &ctx).is_ok() {
                        break (request, record);
                    }
                    if attempt >= max_attempts {
                        until_error = Some(anyhow::anyhow!(
                            "error: condition '{}' not met after {} attempt(s)",
                            condition,
                            max_attempts
                        ));
                        break (request, record);
                    }
                    "condition not met".to_string()
                } else if transient_5xx(record.status) && attempt < max_attempts {
                    format!("server error ({})", format_status(record.status))
                } else {
                    break (request, record);
                }
            }
            Err(e) => {
                // A permanent error (bad URL, method, or header) cannot be
                // fixed by retrying — fail fast instead of burning attempts.
                if attempt >= max_attempts || e.is::<PermanentError>() {
                    return Err(e);
                }
                format!("network error ({})", e)
            }
        };
        attempt += 1;
        eprintln!(
            "{}; retrying in {}s (attempt {}/{})",
            reason, flags.delay_secs, attempt, max_attempts
        );
        std::thread::sleep(std::time::Duration::from_secs(flags.delay_secs));
    };
    if fresh_history {
        state.requests.clear();
        state.responses.clear();
    }
    state.requests.push(request);
    state.responses.push(record);
    session.save(state)?;

    let last = state.responses.last().unwrap();
    let mut notes = vec![
        format!("{} ms", last.elapsed_ms),
        format_size(last.body.len()),
    ];
    notes.extend(cleared_note.map(str::to_string));
    eprintln!("< {} ({})", format_status(last.status), notes.join(", "));
    print_body(&last.body, true);
    if last.status >= 400 {
        // The `fail` verdict is delivered after the whole chain has run —
        // remember the first failing status until then.
        chain.http_failure.get_or_insert(last.status);
    }
    if let Some(e) = until_error {
        return Err(e);
    }
    Ok(())
}

// Bookkeeping threaded through the command loop: whether the state carries
// unsaved mutations, plus the deferred show/response requests.
#[derive(Default)]
struct ChainState {
    modified: bool,
    do_show: bool,
    do_response: Option<(ResponseTarget, ResponseView)>,
    // First 4xx/5xx status received by a send/then — `fail` turns it into a
    // failing exit after the whole chain has completed.
    http_failure: Option<u16>,
}

pub fn run_commands(
    commands: Vec<Command>,
    flags: &GlobalFlags,
    state: &mut State,
    http: &dyn HttpClient,
    session: &dyn SessionStore,
    presets: &dyn PresetStore,
) -> Result<ParseResult> {
    let mut chain = ChainState::default();
    let outcome = run_all(commands, flags, state, http, session, presets, &mut chain);
    if chain.modified {
        if outcome.is_err() {
            // Mutations made before the failing command must survive the
            // abort (`reel url X send` should not lose the url on a network
            // error) — best-effort, so a save failure cannot mask the chain
            // error the user needs to see.
            let _ = session.save(state);
        } else {
            session.save(state)?;
        }
    }
    outcome?;
    // `fail` verdict comes last: every command has run and the session is
    // saved — only the exit code changes.
    if flags.fail_on_error
        && let Some(status) = chain.http_failure
    {
        bail!("error: request failed with status {}", status);
    }
    Ok(ParseResult {
        do_show: chain.do_show,
        do_response: chain.do_response,
    })
}

fn run_all(
    commands: Vec<Command>,
    flags: &GlobalFlags,
    state: &mut State,
    http: &dyn HttpClient,
    session: &dyn SessionStore,
    presets: &dyn PresetStore,
    chain: &mut ChainState,
) -> Result<()> {
    for cmd in commands {
        match cmd {
            Command::Send => {
                if flags.dry_run {
                    print_dry_run(&with_session_cookies(&state.request, &state.cookies));
                    continue;
                }
                execute_and_record(state, flags, http, session, None, true, chain)?;
                chain.modified = false;
            }
            Command::Then(path) => {
                let path_str = path.to_string_lossy().into_owned();

                // Replace the current request from the preset; the history
                // stays in place so templates can reference it.
                state.request = presets.load(&path)?.request;

                // Interpolate against the prior request/response history and the
                // environment. Placeholders referencing missing history (or a
                // missing env var) abort the chain here.
                let ctx = Context {
                    requests: &state.requests,
                    responses: &state.responses,
                    vars: &state.vars,
                };
                apply_interpolation(&mut state.request, &ctx)
                    .map_err(|e| anyhow::anyhow!("error in '{}': {}", path_str, e))?;

                if flags.dry_run {
                    eprintln!("(dry-run: {})", path_str);
                    print_dry_run(&with_session_cookies(&state.request, &state.cookies));
                    continue;
                }

                execute_and_record(state, flags, http, session, Some(&path_str), false, chain)?;
                chain.modified = false;
            }
            Command::Expect(condition) => {
                if flags.dry_run {
                    eprintln!("(dry-run: skipping expect {})", condition);
                    continue;
                }
                let ctx = Context {
                    requests: &state.requests,
                    responses: &state.responses,
                    vars: &state.vars,
                };
                check_condition(&condition, &ctx).map_err(|e| anyhow::anyhow!("error: {}", e))?;
                eprintln!("expect ok: {}", condition);
            }
            Command::Curl => {
                // The command reproduces what reel would send, session
                // cookies included. Stdout: it is data, not a diagnostic.
                let request = with_session_cookies(&state.request, &state.cookies);
                println!("{}", format_curl(&request, flags.insecure, flags.follow)?);
            }
            Command::Show => {
                chain.do_show = true;
            }
            Command::Response(target, view) => {
                chain.do_response = Some((target, view));
            }
            Command::Reset => {
                *state = State::default();
                session.delete();
                chain.modified = false;
                eprintln!("Session cleared.");
            }
            Command::Method(m) => {
                state.request.method = Some(m);
                chain.modified = true;
            }
            Command::Url(u) => {
                if u.is_empty() {
                    bail!("error: URL cannot be empty");
                }
                state.request.url = Some(u);
                chain.modified = true;
            }
            Command::Header(key, val) => {
                state.request.headers.insert(key, val);
                chain.modified = true;
            }
            Command::HeaderRm(key) => {
                if state.request.headers.remove(&key) {
                    chain.modified = true;
                } else {
                    eprintln!("warning: header '{}' not found", key);
                }
            }
            Command::HeaderRmAll => {
                state.request.headers.clear();
                chain.modified = true;
            }
            Command::Body(b) => {
                state.request.body = Some(resolve_body(&b)?);
                chain.modified = true;
            }
            Command::BodyRm => {
                if state.request.body.take().is_some() {
                    chain.modified = true;
                } else {
                    eprintln!("warning: body not set");
                }
            }
            Command::CookieRm(name) => {
                let before = state.cookies.len();
                state.cookies.retain(|c| c.name != name);
                if state.cookies.len() != before {
                    chain.modified = true;
                } else {
                    eprintln!("warning: cookie '{}' not found", name);
                }
            }
            Command::Timeout(secs) => {
                state.request.timeout_secs = Some(secs);
                chain.modified = true;
            }
            Command::Var(name, value) => {
                state.vars.insert(name, value);
                chain.modified = true;
            }
            Command::VarRm(name) => {
                if state.vars.remove(&name).is_some() {
                    chain.modified = true;
                } else {
                    eprintln!("warning: variable '{}' not found", name);
                }
            }
            Command::Save(path) => {
                presets.save(&state.request, &path)?;
                eprintln!("Request saved to: {}", path.display());
            }
            Command::Load(path) => {
                let mut loaded = presets.load(&path)?;
                // The cookie jar and session variables belong to the session,
                // not the loaded file — they survive `load` just like they
                // survive `send` (only `reset` clears them). A file that
                // explicitly carries cookies or vars still wins.
                if loaded.cookies.is_empty() {
                    loaded.cookies = std::mem::take(&mut state.cookies);
                }
                if loaded.vars.is_empty() {
                    loaded.vars = std::mem::take(&mut state.vars);
                }
                *state = loaded;
                chain.modified = true;
                eprintln!("Request loaded from: {}", path.display());
            }
        }
    }

    Ok(())
}
