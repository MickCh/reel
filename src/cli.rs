use std::fs;
use std::path::PathBuf;

use crate::http::execute;
use crate::model::State;
use crate::session::{save_state, session_path};
use crate::template::apply_interpolation;

pub enum LastView {
    Body,
    Headers,
    All,
}

pub fn show_state(state: &State) {
    println!("Session: {}", session_path().display());
    println!();
    println!("  method  {}", state.method.as_deref().unwrap_or("(not set)"));
    println!("  url     {}", state.url.as_deref().unwrap_or("(not set)"));
    for (k, v) in &state.headers {
        println!("  header  {}: {}", k, v);
    }
    if let Some(body) = &state.body {
        println!("  body    {}", body);
    }
}

pub fn show_last(state: &State, view: LastView) {
    match &state.last {
        Some(last) => match view {
            LastView::Body => print!("{}", last.body),
            LastView::Headers => {
                println!("{}", last.status);
                for (k, v) in &last.headers {
                    println!("{}: {}", k, v);
                }
            }
            LastView::All => println!("{}", serde_json::to_string_pretty(last).unwrap()),
        },
        None => eprintln!("error: no response stored yet (run 'req send' first)"),
    }
}

pub fn print_usage() {
    eprintln!("Usage: req [COMMANDS...]");
    eprintln!();
    eprintln!("Commands (can be combined in a single invocation):");
    eprintln!("  method <METHOD>        set HTTP method (GET, POST, PUT, PATCH, DELETE, HEAD)");
    eprintln!("  url <URL>              set request URL");
    eprintln!("  header <KEY:VALUE>     add a header (KEY:VALUE or KEY VALUE)");
    eprintln!("  header-rm <KEY>        remove a header by name");
    eprintln!("  body <BODY>            set request body");
    eprintln!("  send                   send the current request");
    eprintln!("  then <PATH>            load next request from file (with template interpolation) and send it");
    eprintln!("  show                   print the current session state");
    eprintln!("  last [body|headers]    show last response (default: full JSON)");
    eprintln!("  reset                  clear the session state");
    eprintln!("  load <PATH>            load state from a JSON file");
    eprintln!("  save <PATH>            save current session state to a JSON file");
    eprintln!();
    eprintln!("Template interpolation in files loaded by 'then':");
    eprintln!("  ${{{{ status }}}}          HTTP status code of the previous response");
    eprintln!("  ${{{{ body }}}}            raw body of the previous response");
    eprintln!("  ${{{{ body.field.sub }}}}  dot-path into the JSON body");
    eprintln!("  ${{{{ headers.name }}}}    response header value");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  req method GET url https://httpbin.org/get send");
    eprintln!("  req header \"Authorization: Bearer token\"");
    eprintln!("  req header Content-Type application/json");
    eprintln!("  req body '{{\"key\":\"value\"}}' send");
    eprintln!("  req last body | jq .name");
    eprintln!("  req last headers");
    eprintln!("  req send then step2.json then step3.json");
}

pub struct ParseResult {
    pub do_show: bool,
    pub do_last: Option<LastView>,
}

pub fn parse_and_run(args: &[String], state: &mut State) -> ParseResult {
    let mut modified = false;
    let mut do_show = false;
    let mut do_last: Option<LastView> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "send" => {
                if !execute(state) {
                    std::process::exit(1);
                }
                modified = false;
                i += 1;
            }
            "then" => {
                if i + 1 >= args.len() {
                    eprintln!("error: 'then' requires a file path");
                    std::process::exit(1);
                }
                let path = PathBuf::from(&args[i + 1]);
                let prev_last = state.last.clone();

                let content = match fs::read_to_string(&path) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("error reading '{}': {}", path.display(), e);
                        std::process::exit(1);
                    }
                };
                let mut loaded: State = match serde_json::from_str(&content) {
                    Ok(s) => s,
                    Err(e) => {
                        eprintln!("error parsing '{}': {}", path.display(), e);
                        std::process::exit(1);
                    }
                };

                loaded.last = prev_last;
                *state = loaded;

                if let Some(last) = state.last.clone()
                    && let Err(e) = apply_interpolation(state, &last)
                {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }

                if !execute(state) {
                    std::process::exit(1);
                }
                modified = false;
                i += 2;
            }
            "exec" => {
                eprintln!("error: 'exec' has been renamed to 'send'");
                std::process::exit(1);
            }
            "next" => {
                eprintln!("error: 'next' has been renamed to 'then'");
                std::process::exit(1);
            }
            "show" => {
                do_show = true;
                i += 1;
            }
            "last" => {
                let view = match args.get(i + 1).map(String::as_str) {
                    Some("body") => {
                        i += 2;
                        LastView::Body
                    }
                    Some("headers") => {
                        i += 2;
                        LastView::Headers
                    }
                    _ => {
                        i += 1;
                        LastView::All
                    }
                };
                do_last = Some(view);
            }
            "reset" => {
                *state = State::default();
                modified = true;
                println!("Session cleared.");
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
                    let content = serde_json::to_string_pretty(&state).unwrap();
                    match fs::write(&path, content) {
                        Ok(_) => println!("Session saved to: {}", path.display()),
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
                                println!("Loaded state from: {}", path.display());
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
                eprintln!("Run 'req' with no arguments to see usage.");
                i += 1;
            }
        }
    }

    if modified {
        save_state(state);
    }

    ParseResult { do_show, do_last }
}
