use std::path::PathBuf;

use crate::http::{build_client, execute};
use crate::model::{ResponseRecord, State};
use crate::session::{delete_session, load_preset, save_preset, save_state, session_path};
use crate::template::apply_interpolation;

fn format_status(code: u16) -> String {
    let reason = match code {
        100 => "Continue",
        101 => "Switching Protocols",
        102 => "Processing",
        103 => "Early Hints",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        203 => "Non-Authoritative Information",
        204 => "No Content",
        205 => "Reset Content",
        206 => "Partial Content",
        207 => "Multi-Status",
        208 => "Already Reported",
        226 => "IM Used",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        305 => "Use Proxy",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        412 => "Precondition Failed",
        413 => "Content Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        417 => "Expectation Failed",
        418 => "I'm a Teapot",
        421 => "Misdirected Request",
        422 => "Unprocessable Content",
        423 => "Locked",
        424 => "Failed Dependency",
        425 => "Too Early",
        426 => "Upgrade Required",
        428 => "Precondition Required",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        451 => "Unavailable For Legal Reasons",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        506 => "Variant Also Negotiates",
        507 => "Insufficient Storage",
        508 => "Loop Detected",
        510 => "Not Extended",
        511 => "Network Authentication Required",
        _ => "",
    };
    if reason.is_empty() {
        format!("{}", code)
    } else {
        format!("{} {}", code, reason)
    }
}

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
    eprintln!("Session: {}", session_path().display());
    eprintln!(
        "  method  {}",
        state.method.as_deref().unwrap_or("(not set)")
    );
    eprintln!("  url     {}", state.url.as_deref().unwrap_or("(not set)"));
    let mut headers: Vec<_> = state.headers.iter().collect();
    headers.sort_by_key(|(k, _)| k.as_str());
    for (k, v) in headers {
        eprintln!("  header  {}: {}", k, v);
    }
    if let Some(body) = &state.body {
        eprintln!("  body    {}", body);
    }
    if !state.responses.is_empty() {
        eprintln!("  responses  {} stored", state.responses.len());
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
            let mut headers: Vec<_> = r.headers.iter().collect();
            headers.sort_by_key(|(k, _)| k.as_str());
            for (k, v) in headers {
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
                n + 1,
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
                        eprintln!("[{}] {}", i + 1, label);
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
                    println!("[{}] {} — {}", i + 1, r.status, label);
                    let mut headers: Vec<_> = r.headers.iter().collect();
                    headers.sort_by_key(|(k, _)| k.as_str());
                    for (k, v) in headers {
                        println!("  {}: {}", k, v);
                    }
                }
            }
        },
    }
}

pub fn print_usage() {
    eprintln!("reel {}", env!("CARGO_PKG_VERSION"));
    eprintln!("Usage: reel [COMMANDS...]");
    eprintln!();
    eprintln!("Commands (can be combined in a single invocation):");
    eprintln!("  method <METHOD>        set HTTP method (GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS, ...)");
    eprintln!("  url <URL>              set request URL");
    eprintln!("  header <KEY:VALUE>     add a header (KEY:VALUE or KEY VALUE)");
    eprintln!("  header-rm <KEY>        remove a header by name");
    eprintln!("  header-rm-all          remove all headers");
    eprintln!("  body <BODY>            set request body");
    eprintln!("  send                   send the current request");
    eprintln!("  --dry-run              print the request that would be sent, without sending it (position-independent)");
    eprintln!(
        "  then <PATH>            load next request from file (with template interpolation) and send it"
    );
    eprintln!("  show                   print the current session state");
    eprintln!("  response [N|all] [body|headers]   show Nth response (1-based, default: last), or all; full JSON, body, or headers");
    eprintln!("  reset                  clear the session state");
    eprintln!("  load <PATH>            load state from a JSON file");
    eprintln!("  save <PATH>            save current session state to a JSON file");
    eprintln!("  fail                   exit with code 1 if any response is 4xx/5xx (position-independent)");
    eprintln!("  insecure / --insecure  skip TLS certificate verification (position-independent)");
    eprintln!("  -h / --help            show this help");
    eprintln!("  -V / --version         show version");
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
    eprintln!("  reel response all");
    eprintln!("  reel fail send  # exits 1 on 4xx/5xx");
    eprintln!();
    eprintln!("Note: status messages and confirmations are written to stderr; response body goes to stdout.");
    eprintln!("      This means 'reel send | jq .' works correctly even when confirmations are visible.");
}

fn print_dry_run(state: &State) {
    eprintln!("> {} {}", state.method.as_deref().unwrap_or("GET"), state.url.as_deref().unwrap_or("(not set)"));
    let mut headers: Vec<_> = state.headers.iter().collect();
    headers.sort_by_key(|(k, _)| k.as_str());
    for (k, v) in headers {
        eprintln!(">   {}: {}", k, v);
    }
    if let Some(body) = &state.body {
        eprintln!(">   {}", body);
    }
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

fn parse_response_target(args: &[String]) -> Result<(ResponseTarget, usize), ()> {
    match args.first().map(String::as_str) {
        Some("all") => Ok((ResponseTarget::All, 1)),
        Some(token) if token != "body" && token != "headers" => {
            if let Ok(n) = token.parse::<usize>() {
                if n == 0 {
                    eprintln!("error: response indices start at 1");
                    return Err(());
                }
                Ok((ResponseTarget::Index(n - 1), 1))
            } else {
                Ok((ResponseTarget::Last, 0))
            }
        }
        _ => Ok((ResponseTarget::Last, 0)),
    }
}

pub fn parse_and_run(args: &[String], state: &mut State) -> Result<ParseResult, ()> {
    let mut modified = false;
    let mut do_show = false;
    let mut do_response: Option<(ResponseTarget, ResponseView)> = None;

    let insecure = args.iter().any(|a| a == "--insecure" || a == "insecure");
    let fail_on_error = args.iter().any(|a| a == "fail");
    let dry_run = args.iter().any(|a| a == "--dry-run" || a == "dry-run");
    let mut client: Option<reqwest::blocking::Client> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "send" => {
                if dry_run {
                    print_dry_run(state);
                    i += 1;
                    continue;
                }
                let had_responses = !state.responses.is_empty();
                state.responses.clear();
                let c = client.get_or_insert_with(|| build_client(insecure));
                let record = execute(c, state, None)?;
                state.responses.push(record);
                save_state(state);
                let last = state.responses.last().unwrap();
                if had_responses {
                    eprintln!("< {} (previous responses cleared)", format_status(last.status));
                } else {
                    eprintln!("< {}", format_status(last.status));
                }
                print!("{}", last.body);
                if fail_on_error && last.status >= 400 {
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

                let mut loaded = load_preset(&path)?;
                loaded.responses = prev_responses;
                *state = loaded;

                if let Some(prev) = state.responses.last().cloned()
                    && let Err(e) = apply_interpolation(state, &prev)
                {
                    eprintln!("error in '{}': {}", path_str, e);
                    return Err(());
                }

                if dry_run {
                    eprintln!("(dry-run: {})", path_str);
                    print_dry_run(state);
                    i += 2;
                    continue;
                }

                let c = client.get_or_insert_with(|| build_client(insecure));
                let record = execute(c, state, Some(&path_str))?;
                state.responses.push(record);
                save_state(state);
                let last = state.responses.last().unwrap();
                eprintln!("< {}", format_status(last.status));
                print!("{}", last.body);
                if fail_on_error && last.status >= 400 {
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
                let (target, target_consumed) = parse_response_target(rest)?;
                let (view, view_consumed) = parse_response_view(&rest[target_consumed..]);
                do_response = Some((target, view));
                i += 1 + target_consumed + view_consumed;
            }
            "reset" => {
                *state = State::default();
                delete_session();
                modified = false;
                eprintln!("Session cleared.");
                i += 1;
            }
            "fail" => {
                i += 1;
            }
            "--insecure" | "insecure" => {
                i += 1;
            }
            "--dry-run" | "dry-run" => {
                i += 1;
            }
            "method" => {
                if i + 1 < args.len() {
                    state.method = Some(args[i + 1].to_uppercase());
                    modified = true;
                    i += 2;
                } else {
                    eprintln!("error: 'method' requires a value");
                    return Err(());
                }
            }
            "url" => {
                if i + 1 < args.len() {
                    state.url = Some(args[i + 1].clone());
                    modified = true;
                    i += 2;
                } else {
                    eprintln!("error: 'url' requires a value");
                    return Err(());
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
                    if key.is_empty() {
                        eprintln!("error: header key cannot be empty (use KEY:VALUE or KEY VALUE)");
                        return Err(());
                    }
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
                    return Err(());
                }
            }
            "header-rm-all" => {
                state.headers.clear();
                modified = true;
                i += 1;
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
                    return Err(());
                }
            }
            "body" => {
                if i + 1 < args.len() {
                    state.body = Some(args[i + 1].clone());
                    modified = true;
                    i += 2;
                } else {
                    eprintln!("error: 'body' requires a value");
                    return Err(());
                }
            }
            "save" => {
                if i + 1 < args.len() {
                    let path = PathBuf::from(&args[i + 1]);
                    if save_preset(state, &path).is_ok() {
                        eprintln!("Request saved to: {}", path.display());
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
                    *state = load_preset(&path)?;
                    modified = true;
                    eprintln!("Request loaded from: {}", path.display());
                    i += 2;
                } else {
                    eprintln!("error: 'load' requires a path");
                    return Err(());
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
