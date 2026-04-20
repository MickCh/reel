use std::fs;
use std::path::PathBuf;

use crate::http::{build_client, execute};
use crate::model::{ResponseRecord, State};
use crate::session::{delete_session, save_state, session_path};
use crate::template::apply_interpolation;

pub enum ResponseTarget {
    Last,
    Index(usize),
    All,
}

pub enum ResponseView {
    Full,
    Body,
    Headers,
}

pub fn show_state(state: &State) {
    println!("Session: {}", session_path().display());
    println!(
        "  method  {}",
        state.method.as_deref().unwrap_or("(not set)")
    );
    println!("  url     {}", state.url.as_deref().unwrap_or("(not set)"));
    for (k, v) in &state.headers {
        println!("  header  {}: {}", k, v);
    }
    if let Some(body) = &state.body {
        println!("  body    {}", body);
    }
}

fn source_label(r: &ResponseRecord) -> &str {
    r.source.as_deref().unwrap_or("(session)")
}

fn show_single(r: &ResponseRecord, view: &ResponseView) {
    match view {
        ResponseView::Body => print!("{}", r.body),
        ResponseView::Headers => {
            println!("{} — {}", r.status, source_label(r));
            for (k, v) in &r.headers {
                println!("{}: {}", k, v);
            }
        }
        ResponseView::Full => println!("{}", serde_json::to_string_pretty(r).unwrap()),
    }
}

pub fn show_response(state: &State, target: ResponseTarget, view: ResponseView) {
    if state.responses.is_empty() {
        eprintln!("error: no responses stored yet (run 'reel send' first)");
        return;
    }
    match target {
        ResponseTarget::Last => show_single(state.responses.last().unwrap(), &view),
        ResponseTarget::Index(n) => match state.responses.get(n) {
            Some(r) => show_single(r, &view),
            None => eprintln!(
                "error: response index {} out of range (have {})",
                n,
                state.responses.len()
            ),
        },
        ResponseTarget::All => match view {
            ResponseView::Full => {
                println!("{}", serde_json::to_string_pretty(&state.responses).unwrap())
            }
            ResponseView::Body => {
                for (i, r) in state.responses.iter().enumerate() {
                    if state.responses.len() > 1 {
                        let label = source_label(r);
                        eprintln!("[{}] {}", i, label);
                    }
                    print!("{}", r.body);
                    if i + 1 < state.responses.len() {
                        println!();
                    }
                }
            }
            ResponseView::Headers => {
                for (i, r) in state.responses.iter().enumerate() {
                    let label = source_label(r);
                    println!("[{}] {} — {}", i, r.status, label);
                    for (k, v) in &r.headers {
                        println!("  {}: {}", k, v);
                    }
                }
            }
        },
    }
}

pub fn print_usage() {
    eprintln!("Usage: reel [COMMANDS...]");
    eprintln!();
    eprintln!("Commands (can be combined in a single invocation):");
    eprintln!("  method <METHOD>        set HTTP method (GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS, ...)");
    eprintln!("  url <URL>              set request URL");
    eprintln!("  header <KEY:VALUE>     add a header (KEY:VALUE or KEY VALUE)");
    eprintln!("  header-rm <KEY>        remove a header by name");
    eprintln!("  body <BODY>            set request body");
    eprintln!("  send                   send the current request");
    eprintln!(
        "  then <PATH>            load next request from file (with template interpolation) and send it"
    );
    eprintln!("  show                   print the current session state");
    eprintln!("  response [N] [body|headers]   show Nth response (default: last); full JSON, body, or headers");
    eprintln!("  responses              show all responses as a JSON array");
    eprintln!("  reset                  clear the session state");
    eprintln!("  load <PATH>            load state from a JSON file");
    eprintln!("  save <PATH>            save current session state to a JSON file");
    eprintln!("  fail                   exit with code 1 if the HTTP response status is 4xx or 5xx");
    eprintln!("  --insecure             skip TLS certificate verification");
    eprintln!();
    eprintln!("Template interpolation in files loaded by 'then':");
    eprintln!("  ${{{{ status }}}}          HTTP status code of the previous response");
    eprintln!("  ${{{{ body }}}}            raw body of the previous response");
    eprintln!("  ${{{{ body.field.sub }}}}  dot-path into the JSON body");
    eprintln!("  ${{{{ headers.name }}}}    response header value");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  reel method GET url https://httpbin.org/get send");
    eprintln!("  reel header \"Authorization: Bearer token\"");
    eprintln!("  reel header Content-Type application/json");
    eprintln!("  reel body '{{\"key\":\"value\"}}' send");
    eprintln!("  reel response body | jq .name");
    eprintln!("  reel response headers");
    eprintln!("  reel send then step2.json then step3.json");
    eprintln!("  reel responses");
    eprintln!("  reel fail send  # exits 1 on 4xx/5xx");
}

pub struct ParseResult {
    pub do_show: bool,
    pub do_response: Option<(ResponseTarget, ResponseView)>,
}

fn parse_response_view(args: &[String]) -> (ResponseView, usize) {
    match args.first().map(String::as_str) {
        Some("body") => (ResponseView::Body, 1),
        Some("headers") => (ResponseView::Headers, 1),
        _ => (ResponseView::Full, 0),
    }
}

pub fn parse_and_run(args: &[String], state: &mut State) -> Result<ParseResult, ()> {
    let mut modified = false;
    let mut do_show = false;
    let mut do_response: Option<(ResponseTarget, ResponseView)> = None;
    let mut fail_on_error = false;

    let insecure = args.iter().any(|a| a == "--insecure");
    let client = build_client(insecure);

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "send" => {
                state.responses.clear();
                let status = execute(&client, state, None).map_err(|_| ())?;
                if fail_on_error && status >= 400 {
                    return Err(());
                }
                modified = false;
                i += 1;
            }
            "then" => {
                if i + 1 >= args.len() {
                    eprintln!("error: 'then' requires a file path");
                    return Err(());
                }
                let path_str = args[i + 1].clone();
                let path = PathBuf::from(&path_str);
                let prev_responses = state.responses.clone();

                let content = match fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("error reading '{}': {}", path.display(), e);
                        return Err(());
                    }
                };
                let mut loaded: State = match serde_json::from_str(&content) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("error parsing '{}': {}", path.display(), e);
                        return Err(());
                    }
                };

                loaded.responses = prev_responses;
                *state = loaded;

                if let Some(prev) = state.responses.last().cloned()
                    && let Err(e) = apply_interpolation(state, &prev)
                {
                    eprintln!("error: {}", e);
                    return Err(());
                }

                let status = execute(&client, state, Some(&path_str)).map_err(|_| ())?;
                if fail_on_error && status >= 400 {
                    return Err(());
                }
                modified = false;
                i += 2;
            }
            "show" => {
                do_show = true;
                i += 1;
            }
            "response" => {
                let rest = &args[i + 1..];
                let (target, view, consumed) = match rest.first().map(String::as_str) {
                    Some("body") => (ResponseTarget::Last, ResponseView::Body, 1),
                    Some("headers") => (ResponseTarget::Last, ResponseView::Headers, 1),
                    Some("all") => {
                        let (view, extra) = parse_response_view(&args[i + 2..]);
                        (ResponseTarget::All, view, 1 + extra)
                    }
                    Some(token) => {
                        if let Ok(n) = token.parse::<usize>() {
                            let (view, extra) = parse_response_view(&args[i + 2..]);
                            (ResponseTarget::Index(n), view, 1 + extra)
                        } else {
                            (ResponseTarget::Last, ResponseView::Full, 0)
                        }
                    }
                    None => (ResponseTarget::Last, ResponseView::Full, 0),
                };
                do_response = Some((target, view));
                i += 1 + consumed;
            }
            "responses" => {
                do_response = Some((ResponseTarget::All, ResponseView::Full));
                i += 1;
            }
            "reset" => {
                *state = State::default();
                delete_session();
                modified = false;
                eprintln!("Session cleared.");
                i += 1;
            }
            "fail" => {
                fail_on_error = true;
                i += 1;
            }
            "--insecure" => {
                i += 1;
            }
            "method" => {
                if i + 1 < args.len() {
                    state.method = Some(args[i + 1].to_uppercase());
                    modified = true;
                    i += 2;
                } else {
                    eprintln!("error: 'method' requires a value");
                    i += 1;
                }
            }
            "url" => {
                if i + 1 < args.len() {
                    state.url = Some(args[i + 1].clone());
                    modified = true;
                    i += 2;
                } else {
                    eprintln!("error: 'url' requires a value");
                    i += 1;
                }
            }
            "header" => {
                if i + 1 >= args.len() {
                    eprintln!("error: 'header' requires KEY:VALUE or KEY VALUE");
                    i += 1;
                    continue;
                }
                let next = &args[i + 1];
                if let Some(pos) = next.find(':') {
                    let key = next[..pos].trim().to_string();
                    let val = next[pos + 1..].trim().to_string();
                    state.headers.insert(key, val);
                    modified = true;
                    i += 2;
                } else if i + 2 < args.len() {
                    state.headers.insert(next.clone(), args[i + 2].clone());
                    modified = true;
                    i += 3;
                } else {
                    eprintln!("error: 'header' requires KEY:VALUE or KEY VALUE");
                    i += 2;
                }
            }
            "header-rm" => {
                if i + 1 < args.len() {
                    let key = &args[i + 1];
                    if state.headers.remove(key).is_some() {
                        modified = true;
                    } else {
                        eprintln!("warning: header '{}' not found", key);
                    }
                    i += 2;
                } else {
                    eprintln!("error: 'header-rm' requires a header name");
                    i += 1;
                }
            }
            "body" => {
                if i + 1 < args.len() {
                    state.body = Some(args[i + 1].clone());
                    modified = true;
                    i += 2;
                } else {
                    eprintln!("error: 'body' requires a value");
                    i += 1;
                }
            }
            "save" => {
                if i + 1 < args.len() {
                    let path = PathBuf::from(&args[i + 1]);
                    let mut to_save = state.clone();
                    to_save.responses.clear();
                    let content = serde_json::to_string_pretty(&to_save).unwrap();
                    match fs::write(&path, content) {
                        Ok(_) => eprintln!("Request saved to: {}", path.display()),
                        Err(e) => eprintln!("error writing '{}': {}", path.display(), e),
                    }
                    i += 2;
                } else {
                    eprintln!("error: 'save' requires a path");
                    i += 1;
                }
            }
            "load" => {
                if i + 1 < args.len() {
                    let path = PathBuf::from(&args[i + 1]);
                    match fs::read_to_string(&path) {
                        Ok(content) => match serde_json::from_str::<State>(&content) {
                            Ok(loaded) => {
                                *state = loaded;
                                modified = true;
                                eprintln!("Request loaded from: {}", path.display());
                            }
                            Err(e) => eprintln!("error parsing file: {}", e),
                        },
                        Err(e) => eprintln!("error reading '{}': {}", path.display(), e),
                    }
                    i += 2;
                } else {
                    eprintln!("error: 'load' requires a path");
                    i += 1;
                }
            }
            unknown => {
                eprintln!("error: unknown command '{}'", unknown);
                eprintln!("Run 'reel' with no arguments to see usage.");
                i += 1;
            }
        }
    }

    if modified {
        save_state(state);
    }

    Ok(ParseResult { do_show, do_response })
}
