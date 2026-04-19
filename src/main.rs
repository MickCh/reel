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

// Navigate a dot-separated path through a JSON value.
// Array indices are supported as numeric path segments (e.g. "items.0.id").
fn traverse_json(value: &serde_json::Value, path: &str) -> Option<String> {
    let mut current = value;
    for key in path.split('.') {
        current = match current {
            serde_json::Value::Object(map) => map.get(key)?,
            serde_json::Value::Array(arr) => {
                let idx: usize = key.parse().ok()?;
                arr.get(idx)?
            }
            _ => return None,
        };
    }
    Some(match current {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

// Evaluate a single template expression against the last response.
//
// Supported forms:
//   status              → HTTP status code as a string
//   body                → raw response body
//   body.<path>         → dot-path into the JSON body (e.g. body.access.token)
//   headers.<name>      → response header value (e.g. headers.content-type)
fn eval_expr(expr: &str, last: &LastResponse) -> Result<String, String> {
    if expr == "status" {
        return Ok(last.status.to_string());
    }
    if expr == "body" {
        return Ok(last.body.clone());
    }
    if let Some(path) = expr.strip_prefix("headers.") {
        return last
            .headers
            .get(path)
            .cloned()
            .ok_or_else(|| format!("header '{}' not found in last response", path));
    }
    if let Some(path) = expr.strip_prefix("body.") {
        let json: serde_json::Value = serde_json::from_str(&last.body)
            .map_err(|e| format!("response body is not valid JSON: {}", e))?;
        return traverse_json(&json, path)
            .ok_or_else(|| format!("path '{}' not found in response body", path));
    }
    Err(format!("unknown template expression '${{{{ {} }}}}'", expr))
}

// Replace all ${{ expr }} placeholders in `text` using values from `last`.
fn interpolate(text: &str, last: &LastResponse) -> Result<String, String> {
    let mut result = String::new();
    let mut remaining = text;
    while let Some(start) = remaining.find("${{") {
        result.push_str(&remaining[..start]);
        remaining = &remaining[start + 3..];
        let end = remaining
            .find("}}")
            .ok_or_else(|| "unclosed '${{' in template".to_string())?;
        let expr = remaining[..end].trim();
        remaining = &remaining[end + 2..];
        result.push_str(&eval_expr(expr, last)?);
    }
    result.push_str(remaining);
    Ok(result)
}

// Apply interpolation to all string fields of state (url, body, header values).
fn apply_interpolation(state: &mut State, last: &LastResponse) -> Result<(), String> {
    if let Some(url) = &state.url.clone() {
        state.url = Some(interpolate(url, last)?);
    }
    if let Some(body) = &state.body.clone() {
        state.body = Some(interpolate(body, last)?);
    }
    let keys: Vec<String> = state.headers.keys().cloned().collect();
    for key in keys {
        let val = state.headers[&key].clone();
        state.headers.insert(key, interpolate(&val, last)?);
    }
    Ok(())
}

// Execute the request, store the response in state.last, and save the session.
// Returns false on failure so the caller can abort a chain.
fn execute(state: &mut State) -> bool {
    let url = match &state.url {
        Some(u) => u.clone(),
        None => {
            eprintln!("error: URL not set (use: req url <URL>)");
            return false;
        }
    };

    let method = state.method.as_deref().unwrap_or("GET").to_uppercase();

    let client = reqwest::blocking::Client::new();

    let req_builder = match method.as_str() {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "PATCH" => client.patch(&url),
        "DELETE" => client.delete(&url),
        "HEAD" => client.head(&url),
        other => {
            eprintln!("error: unknown method '{}'", other);
            return false;
        }
    };

    let req_builder = state
        .headers
        .iter()
        .fold(req_builder, |b, (k, v)| b.header(k, v));

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
                .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
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
                    true
                }
                Err(e) => {
                    eprintln!("error reading response: {}", e);
                    false
                }
            }
        }
        Err(e) => {
            eprintln!("error: {}", e);
            false
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
    eprintln!("  send                   send the current request");
    eprintln!("  then <PATH>            load next request from file (with template interpolation) and send it");
    eprintln!("  show                   print the current session state");
    eprintln!("  last [body|headers]    show last response (default: full JSON)");
    eprintln!("  reset                  clear the session state");
    eprintln!("  file <PATH>            load state from a JSON file");
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

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() {
        print_usage();
        return;
    }

    let mut state = load_state();
    let mut modified = false;
    let mut do_show = false;
    let mut do_last: Option<LastView> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "send" => {
                // execute() calls save_state internally, so no separate save needed here.
                if !execute(&mut state) {
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

                // Remember the last response before replacing state.
                let prev_last = state.last.clone();

                // Load the next request file.
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

                // Carry the previous response forward so templates can reference it.
                loaded.last = prev_last;
                state = loaded;

                // Substitute ${{ … }} placeholders using the previous response.
                if let Some(last) = state.last.clone()
                    && let Err(e) = apply_interpolation(&mut state, &last)
                {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }

                // Execute the newly loaded (and interpolated) state.
                if !execute(&mut state) {
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
                // Peek at the next token: consume it only if it's a known subcommand.
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

    if let Some(view) = do_last {
        match &state.last {
            Some(last) => show_last(last, view),
            None => eprintln!("error: no response stored yet (run 'req exec' first)"),
        }
    }
}
