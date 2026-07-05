use anyhow::{Result, bail};

use crate::http::HttpClient;
use crate::model::State;
use crate::session::{PresetStore, SessionStore};
use crate::template::{Context, apply_interpolation};

use super::commands::{Command, GlobalFlags, ParseResult, ResponseTarget, ResponseView};
use super::display::{format_status, print_dry_run};

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

    let record = http.execute(&state.request, source)?;
    state.requests.push(state.request.clone());
    state.responses.push(record);
    session.save(state)?;

    let last = state.responses.last().unwrap();
    match status_note {
        Some(note) => eprintln!("< {} ({})", format_status(last.status), note),
        None => eprintln!("< {}", format_status(last.status)),
    }
    print!("{}", last.body);
    if !last.body.ends_with('\n') {
        println!();
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
                    print_dry_run(&state.request);
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
                    print_dry_run(&state.request);
                    continue;
                }

                execute_and_record(state, flags, http, session, Some(&path_str), None)?;
                modified = false;
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
                state.request.body = Some(b);
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
