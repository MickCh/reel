use anyhow::{Result, bail};

use crate::http::{HttpClient, PermanentError};
use crate::model::{Cookie, Request, State, cookie_header, parse_set_cookie, update_jar};
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
    // alone gets a default budget of 10 attempts.
    let max_attempts = match (&flags.until, flags.retry) {
        (Some(_), 0) => 10,
        (_, retry) => retry.saturating_add(1),
    };
    let mut attempt = 1u32;
    // Set when --until exhausts its budget: the last response is still
    // recorded and printed, but the chain aborts afterwards.
    let mut until_error = None;

    let (request, record) = loop {
        // Rebuilt each attempt: a Set-Cookie received on a retried attempt
        // must be reflected in the next one.
        let request = with_session_cookies(&state.request, &state.cookies);
        let reason = match http.execute(&request, source) {
            Ok(record) => {
                // Every attempt's cookies enter the jar, not just the final
                // one — servers rotate session cookies mid-poll. Marking the
                // state modified lets the end-of-run best-effort save keep
                // them even when a later attempt fails the whole chain.
                if let Some((host, path, _)) = url_parts(request.url.as_deref()) {
                    for raw in &record.set_cookies {
                        if let Some(update) = parse_set_cookie(raw, &host, &path) {
                            update_jar(&mut state.cookies, update);
                            chain.modified = true;
                        }
                    }
                }
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
                };
                check_condition(&condition, &ctx).map_err(|e| anyhow::anyhow!("error: {}", e))?;
                eprintln!("expect ok: {}", condition);
            }
            Command::Curl => {
                // The command reproduces what reel would send, session
                // cookies included. Stdout: it is data, not a diagnostic.
                let request = with_session_cookies(&state.request, &state.cookies);
                println!("{}", format_curl(&request, flags.insecure)?);
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
            Command::Save(path) => {
                presets.save(&state.request, &path)?;
                eprintln!("Request saved to: {}", path.display());
            }
            Command::Load(path) => {
                let mut loaded = presets.load(&path)?;
                // The cookie jar belongs to the session, not the loaded file —
                // it survives `load` just like it survives `send` (only
                // `reset` clears it). A file that explicitly carries cookies
                // still wins.
                if loaded.cookies.is_empty() {
                    loaded.cookies = std::mem::take(&mut state.cookies);
                }
                *state = loaded;
                chain.modified = true;
                eprintln!("Request loaded from: {}", path.display());
            }
        }
    }

    Ok(())
}
