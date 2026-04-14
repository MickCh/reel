use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
struct LastResponse {
    status: u16,
    headers: HashMap<String, String>,
    body: String,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
struct State {
    method: Option<String>,
    url: Option<String>,
    headers: HashMap<String, String>,
    body: Option<String>,
    last: Option<LastResponse>,
}

// Identify the session by the parent shell's PID (PPID of this process).
// Each terminal window runs its own shell, so sessions are naturally isolated.
fn get_ppid() -> u32 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("PPid:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|p| p.parse().ok())
        })
        .unwrap_or(0)
}

fn session_path() -> PathBuf {
    dirs::home_dir()
        .expect("cannot determine home directory")
        .join(".req")
        .join("sessions")
        .join(format!("{}.json", get_ppid()))
}

fn load_state() -> State {
    let path = session_path();
    if path.exists() {
        let content = fs::read_to_string(&path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        State::default()
    }
}

fn save_state(state: &State) {
    let path = session_path();
    fs::create_dir_all(path.parent().unwrap()).expect("cannot create session directory");
    let content = serde_json::to_string_pretty(state).unwrap();
    fs::write(&path, content).expect("cannot write session file");
}

fn show_state(state: &State) {
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

fn show_last(last: &LastResponse, view: LastView) {
    match view {
        LastView::Body => {
            print!("{}", last.body);
        }
        LastView::Headers => {
            println!("{}", last.status);
            for (k, v) in &last.headers {
                println!("{}: {}", k, v);
            }
        }
        LastView::All => {
            println!("{}", serde_json::to_string_pretty(last).unwrap());
        }
    }
}

enum LastView {
    Body,
    Headers,
    All,
}

// Execute the request, store the response in state.last, and save the session.
fn execute(state: &mut State) {
    let url = match &state.url {
        Some(u) => u.clone(),
        None => {
            eprintln!("error: URL not set (use: req url <URL>)");
            std::process::exit(1);
        }
    };

    let method = state.method.as_deref().unwrap_or("GET").to_uppercase();

    let client = reqwest::blocking::Client::new();

    let req_builder = match method.as_str() {
        "GET"    => client.get(&url),
        "POST"   => client.post(&url),
        "PUT"    => client.put(&url),
        "PATCH"  => client.patch(&url),
        "DELETE" => client.delete(&url),
        "HEAD"   => client.head(&url),
        other => {
            eprintln!("error: unknown method '{}'", other);
            std::process::exit(1);
        }
    };

    let req_builder = state.headers.iter().fold(req_builder, |b, (k, v)| b.header(k, v));

    let req_builder = match &state.body {
        Some(b) => req_builder.body(b.clone()),
        None => req_builder,
    };

    match req_builder.send() {
        Ok(resp) => {
            let status = resp.status();
            eprintln!("{} {}", status.as_u16(), status.canonical_reason().unwrap_or(""));

            let resp_headers: HashMap<String, String> = resp
                .headers()
                .iter()
                .filter_map(|(k, v)| {
                    v.to_str().ok().map(|v| (k.to_string(), v.to_string()))
                })
                .collect();

            match resp.text() {
                Ok(body) => {
                    state.last = Some(LastResponse {
                        status: status.as_u16(),
                        headers: resp_headers,
                        body: body.clone(),
                    });
                    save_state(state);
                    print!("{}", body);
                }
                Err(e) => eprintln!("error reading response: {}", e),
            }
        }
        Err(e) => {
            eprintln!("error: {}", e);
            std::process::exit(1);
        }
    }
}

fn print_usage() {
    eprintln!("Usage: req [COMMANDS...]");
    eprintln!();
    eprintln!("Commands (can be combined in a single invocation):");
    eprintln!("  method <METHOD>        set HTTP method (GET, POST, PUT, PATCH, DELETE, HEAD)");
    eprintln!("  url <URL>              set request URL");
    eprintln!("  header <KEY:VALUE>     add a header (KEY:VALUE or KEY VALUE)");
    eprintln!("  header-rm <KEY>        remove a header by name");
    eprintln!("  body <BODY>            set request body");
    eprintln!("  exec                   send the request");
    eprintln!("  show                   print the current session state");
    eprintln!("  last [body|headers]    show last response (default: full JSON)");
    eprintln!("  reset                  clear the session state");
    eprintln!("  file <PATH>            load state from a JSON file");
    eprintln!("  save <PATH>            save current session state to a JSON file");
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  req method GET url https://httpbin.org/get exec");
    eprintln!("  req header \"Authorization: Bearer token\"");
    eprintln!("  req header Content-Type application/json");
    eprintln!("  req body '{{\"key\":\"value\"}}' exec");
    eprintln!("  req last body | jq .name");
    eprintln!("  req last headers");
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        print_usage();
        return;
    }

    let mut state = load_state();
    let mut modified = false;
    let mut do_exec = false;
    let mut do_show = false;
    let mut do_last: Option<LastView> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "exec" => {
                do_exec = true;
                i += 1;
            }
            "show" => {
                do_show = true;
                i += 1;
            }
            "last" => {
                // Peek at the next token: consume it only if it's a known subcommand.
                let view = match args.get(i + 1).map(String::as_str) {
                    Some("body")    => { i += 2; LastView::Body }
                    Some("headers") => { i += 2; LastView::Headers }
                    _               => { i += 1; LastView::All }
                };
                do_last = Some(view);
            }
            "reset" => {
                state = State::default();
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
                    // "Key: Value" or "Key:Value"
                    let key = next[..pos].trim().to_string();
                    let val = next[pos + 1..].trim().to_string();
                    state.headers.insert(key, val);
                    modified = true;
                    i += 2;
                } else if i + 2 < args.len() {
                    // KEY VALUE as two separate arguments
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
            "file" => {
                if i + 1 < args.len() {
                    let path = PathBuf::from(&args[i + 1]);
                    match fs::read_to_string(&path) {
                        Ok(content) => match serde_json::from_str::<State>(&content) {
                            Ok(loaded) => {
                                state = loaded;
                                modified = true;
                                println!("Loaded state from: {}", path.display());
                            }
                            Err(e) => eprintln!("error parsing file: {}", e),
                        },
                        Err(e) => eprintln!("error reading '{}': {}", path.display(), e),
                    }
                    i += 2;
                } else {
                    eprintln!("error: 'file' requires a path");
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
        save_state(&state);
    }

    if do_show {
        show_state(&state);
    }

    // exec saves the session itself (it updates state.last)
    if do_exec {
        execute(&mut state);
    }

    if let Some(view) = do_last {
        match &state.last {
            Some(last) => show_last(last, view),
            None => eprintln!("error: no response stored yet (run 'req exec' first)"),
        }
    }
}
