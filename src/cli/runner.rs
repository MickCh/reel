use anyhow::{Result, bail};

use crate::http::HttpClient;
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

// Shared execution path of `send` and `then`: run the request, record it in
// the history, persist the session, print the result, and honour `fail`.
// `session.save()` must come before any stdout write — broken-pipe safety.
fn execute_and_record(
    state: &mut State,
    flags: &GlobalFlags,
    http: &dyn HttpClient,
    session: &dyn SessionStore,
    source: Option<&str>,
    status_note: Option<&str>,
) -> Result<()> {
    if state.request.body.is_some() && state.request.headers.get("content-type").is_none() {
        eprintln!("warning: body is set but Content-Type header is missing");
    }

    // Attempt budget: --retry N gives N extra attempts; polling with --until
    // alone gets a default budget of 10 attempts.
    let max_attempts = match (&flags.until, flags.retry) {
        (Some(_), 0) => 10,
        (_, retry) => retry + 1,
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
                // one — servers rotate session cookies mid-poll.
                if let Some((host, path, _)) = url_parts(request.url.as_deref()) {
                    for raw in &record.set_cookies {
                        if let Some(update) = parse_set_cookie(raw, &host, &path) {
                            update_jar(&mut state.cookies, update);
                        }
                    }
                }
                if let Some(condition) = &flags.until {
                    // Evaluate against the history as it would look with this
                    // response appended.
                    let mut responses = state.responses.clone();
                    responses.push(record.clone());
                    let ctx = Context {
                        requests: &state.requests,
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
                    "condition not met"
                } else if record.status >= 500 && attempt < max_attempts {
                    "server error"
                } else {
                    break (request, record);
                }
            }
            Err(e) => {
                if attempt >= max_attempts {
                    return Err(e);
                }
                "network error"
            }
        };
        attempt += 1;
        eprintln!(
            "{}; retrying in {}s (attempt {}/{})",
            reason, flags.delay_secs, attempt, max_attempts
        );
        std::thread::sleep(std::time::Duration::from_secs(flags.delay_secs));
    };
    state.requests.push(request);
    state.responses.push(record);
    session.save(state)?;

    let last = state.responses.last().unwrap();
    let mut notes = vec![
        format!("{} ms", last.elapsed_ms),
        format_size(last.body.len()),
    ];
    notes.extend(status_note.map(str::to_string));
    eprintln!("< {} ({})", format_status(last.status), notes.join(", "));
    print_body(&last.body, true);
    if let Some(e) = until_error {
        return Err(e);
    }
    if flags.fail_on_error && last.status >= 400 {
        bail!("error: request failed with status {}", last.status);
    }
    Ok(())
}

pub fn run_commands(
    commands: Vec<Command>,
    flags: &GlobalFlags,
    state: &mut State,
    http: &dyn HttpClient,
    session: &dyn SessionStore,
    presets: &dyn PresetStore,
) -> Result<ParseResult> {
    let mut modified = false;
    let mut do_show = false;
    let mut do_response: Option<(ResponseTarget, ResponseView)> = None;

    for cmd in commands {
        match cmd {
            Command::Send => {
                if flags.dry_run {
                    print_dry_run(&with_session_cookies(&state.request, &state.cookies));
                    continue;
                }
                let had_responses = !state.responses.is_empty();
                state.requests.clear();
                state.responses.clear();
                let note = had_responses.then_some("previous responses cleared");
                execute_and_record(state, flags, http, session, None, note)?;
                modified = false;
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

                execute_and_record(state, flags, http, session, Some(&path_str), None)?;
                modified = false;
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
                do_show = true;
            }
            Command::Response(target, view) => {
                do_response = Some((target, view));
            }
            Command::Reset => {
                *state = State::default();
                session.delete();
                modified = false;
                eprintln!("Session cleared.");
            }
            Command::Method(m) => {
                state.request.method = Some(m);
                modified = true;
            }
            Command::Url(u) => {
                if u.is_empty() {
                    bail!("error: URL cannot be empty");
                }
                state.request.url = Some(u);
                modified = true;
            }
            Command::Header(key, val) => {
                state.request.headers.insert(key, val);
                modified = true;
            }
            Command::HeaderRm(key) => {
                if state.request.headers.remove(&key) {
                    modified = true;
                } else {
                    eprintln!("warning: header '{}' not found", key);
                }
            }
            Command::HeaderRmAll => {
                state.request.headers.clear();
                modified = true;
            }
            Command::Body(b) => {
                state.request.body = Some(resolve_body(&b)?);
                modified = true;
            }
            Command::Save(path) => {
                presets.save(&state.request, &path)?;
                eprintln!("Request saved to: {}", path.display());
            }
            Command::Load(path) => {
                *state = presets.load(&path)?;
                modified = true;
                eprintln!("Request loaded from: {}", path.display());
            }
        }
    }

    if modified {
        session.save(state)?;
    }

    Ok(ParseResult {
        do_show,
        do_response,
    })
}
