use anyhow::{Result, bail};

use crate::http::HttpClient;
use crate::model::State;
use crate::session::{SessionStore, load_preset, save_preset};
use crate::template::apply_interpolation;

use super::commands::{Command, GlobalFlags, ParseResult, ResponseTarget, ResponseView};
use super::display::{format_status, print_dry_run};

pub fn run_commands(
    commands: Vec<Command>,
    flags: &GlobalFlags,
    state: &mut State,
    http: &dyn HttpClient,
    session: &dyn SessionStore,
) -> Result<ParseResult> {
    let mut modified = false;
    let mut do_show = false;
    let mut do_response: Option<(ResponseTarget, ResponseView)> = None;

    for cmd in commands {
        match cmd {
            Command::Send => {
                if flags.dry_run {
                    print_dry_run(state);
                    continue;
                }
                let had_responses = !state.responses.is_empty();
                state.responses.clear();
                let record = http.execute(state, None)?;
                state.responses.push(record);
                session.save(state);
                let last = state.responses.last().unwrap();
                if had_responses {
                    eprintln!(
                        "< {} (previous responses cleared)",
                        format_status(last.status)
                    );
                } else {
                    eprintln!("< {}", format_status(last.status));
                }
                print!("{}", last.body);
                if !last.body.ends_with('\n') {
                    println!();
                }
                if flags.fail_on_error && last.status >= 400 {
                    bail!("error: request failed with status {}", last.status);
                }
                modified = false;
            }
            Command::Then(path) => {
                let path_str = path.to_string_lossy().into_owned();
                let prev_responses = state.responses.clone();

                let mut loaded = load_preset(&path)?;
                loaded.responses = prev_responses;
                *state = loaded;

                if state.responses.is_empty() {
                    let has_template = state.url.as_deref().is_some_and(|u| u.contains("${{"))
                        || state.body.as_deref().is_some_and(|b| b.contains("${{"))
                        || state.headers.values().any(|v| v.contains("${{"));
                    if has_template {
                        bail!(
                            "error: '{}' uses template expressions but there is no previous response",
                            path_str
                        );
                    }
                } else {
                    let responses = state.responses.clone();
                    apply_interpolation(state, &responses)
                        .map_err(|e| anyhow::anyhow!("error in '{}': {}", path_str, e))?;
                }

                if flags.dry_run {
                    eprintln!("(dry-run: {})", path_str);
                    print_dry_run(state);
                    continue;
                }

                let record = http.execute(state, Some(&path_str))?;
                state.responses.push(record);
                session.save(state);
                let last = state.responses.last().unwrap();
                eprintln!("< {}", format_status(last.status));
                print!("{}", last.body);
                if !last.body.ends_with('\n') {
                    println!();
                }
                if flags.fail_on_error && last.status >= 400 {
                    bail!("error: request failed with status {}", last.status);
                }
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
                state.method = Some(m);
                modified = true;
            }
            Command::Url(u) => {
                if u.is_empty() {
                    bail!("error: URL cannot be empty");
                }
                state.url = Some(u);
                modified = true;
            }
            Command::Header(key, val) => {
                // Case-insensitive dedup: remove existing key with the same name before inserting.
                let existing = state
                    .headers
                    .keys()
                    .find(|k| k.to_lowercase() == key.to_lowercase())
                    .cloned();
                if let Some(old) = existing {
                    state.headers.remove(&old);
                }
                state.headers.insert(key, val);
                modified = true;
            }
            Command::HeaderRm(key) => {
                // Case-insensitive: scan for a matching key regardless of casing in stored state.
                let found = state
                    .headers
                    .keys()
                    .find(|k| k.to_lowercase() == key)
                    .cloned();
                if let Some(k) = found {
                    state.headers.remove(&k);
                    modified = true;
                } else {
                    eprintln!("warning: header '{}' not found", key);
                }
            }
            Command::HeaderRmAll => {
                state.headers.clear();
                modified = true;
            }
            Command::Body(b) => {
                state.body = Some(b);
                modified = true;
            }
            Command::Save(path) => {
                if save_preset(state, &path).is_ok() {
                    eprintln!("Request saved to: {}", path.display());
                }
            }
            Command::Load(path) => {
                *state = load_preset(&path)?;
                modified = true;
                eprintln!("Request loaded from: {}", path.display());
            }
        }
    }

    if modified {
        session.save(state);
    }

    Ok(ParseResult {
        do_show,
        do_response,
    })
}
