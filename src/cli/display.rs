use crate::model::{ResponseRecord, State};
use crate::session::SessionStore;

use super::commands::{ResponseTarget, ResponseView};

pub(super) fn format_status(code: u16) -> String {
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
        code.to_string()
    } else {
        format!("{} {}", code, reason)
    }
}

fn source_label(r: &ResponseRecord) -> &str {
    r.source.as_deref().unwrap_or("(session)")
}

fn response_display_value(r: &ResponseRecord) -> serde_json::Value {
    let body_val = serde_json::from_str::<serde_json::Value>(&r.body)
        .unwrap_or_else(|_| serde_json::Value::String(r.body.clone()));
    serde_json::json!({
        "source": r.source,
        "status": r.status,
        "headers": r.headers,
        "body": body_val,
    })
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
        ResponseView::Full => {
            println!(
                "{}",
                serde_json::to_string_pretty(&response_display_value(r)).unwrap()
            )
        }
    }
}

pub fn show_state(state: &State, session: &dyn SessionStore) {
    eprintln!("Session: {}", session.path().display());
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
        let formatted = serde_json::from_str::<serde_json::Value>(body)
            .map(|v| serde_json::to_string_pretty(&v).unwrap())
            .unwrap_or_else(|_| body.clone());
        eprintln!("  body    {}", formatted.replace('\n', "\n          "));
    }
    if !state.responses.is_empty() {
        eprintln!("  responses  {} stored", state.responses.len());
    }
}

pub fn show_response(state: &State, target: ResponseTarget, view: ResponseView) -> bool {
    if state.responses.is_empty() {
        eprintln!("error: no responses stored yet (run 'reel send' first)");
        return false;
    }
    match target {
        ResponseTarget::Last => show_single(state.responses.last().unwrap(), &view),
        ResponseTarget::Index(n) => match state.responses.get(n) {
            Some(r) => show_single(r, &view),
            None => {
                eprintln!(
                    "error: response index {} out of range (have {})",
                    n + 1,
                    state.responses.len()
                );
                return false;
            }
        },
        ResponseTarget::All => match view {
            ResponseView::Full => {
                let display: Vec<_> = state.responses.iter().map(response_display_value).collect();
                println!("{}", serde_json::to_string_pretty(&display).unwrap())
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
    true
}

pub fn print_usage() {
    eprintln!("reel {}", env!("CARGO_PKG_VERSION"));
    eprintln!("Usage: reel [COMMANDS...]");
    eprintln!();
    eprintln!("Commands (can be combined in a single invocation):");
    eprintln!(
        "  method <METHOD>        set HTTP method (GET, POST, PUT, PATCH, DELETE, HEAD, OPTIONS, ...)"
    );
    eprintln!("  url <URL>              set request URL");
    eprintln!("  header <KEY:VALUE>     add a header (KEY:VALUE or KEY VALUE)");
    eprintln!("  header-rm <KEY>        remove a header by name");
    eprintln!("  header-rm-all          remove all headers");
    eprintln!("  body <BODY>            set request body");
    eprintln!("  send                   send the current request");
    eprintln!(
        "  --dry-run              print the request that would be sent, without sending it (position-independent)"
    );
    eprintln!(
        "  then <PATH>            load next request from file (with template interpolation) and send it"
    );
    eprintln!("  show                   print the current session state");
    eprintln!(
        "  response [N|all] [body|headers]   show Nth response (1-based, default: last), or all; full JSON, body, or headers"
    );
    eprintln!("  reset                  clear the session state");
    eprintln!("  load <PATH>            load state from a JSON file");
    eprintln!("  save <PATH>            save current session state to a JSON file");
    eprintln!(
        "  fail                   exit with code 1 if any response is 4xx/5xx (position-independent)"
    );
    eprintln!("  insecure / --insecure  skip TLS certificate verification (position-independent)");
    eprintln!("  -h / --help            show this help");
    eprintln!("  -V / --version         show version");
    eprintln!();
    eprintln!("Template interpolation in files loaded by 'then':");
    eprintln!("  ${{{{ status }}}}                  HTTP status code of the last response");
    eprintln!("  ${{{{ body }}}}                    raw body of the last response");
    eprintln!("  ${{{{ body.field.sub }}}}          dot-path into the last response body");
    eprintln!("  ${{{{ headers.name }}}}            response header value from the last response");
    eprintln!("  ${{{{ response[N].status }}}}      status of the Nth response (1-based)");
    eprintln!("  ${{{{ response[N].body }}}}        raw body of the Nth response");
    eprintln!("  ${{{{ response[N].body.field }}}}  dot-path into the Nth response body");
    eprintln!("  ${{{{ response[N].headers.name }}}} header from the Nth response");
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
    eprintln!(
        "Note: status messages and confirmations are written to stderr; response body goes to stdout."
    );
    eprintln!(
        "      This means 'reel send | jq .' works correctly even when confirmations are visible."
    );
}

pub(super) fn print_dry_run(state: &State) {
    eprintln!(
        "> {} {}",
        state.method.as_deref().unwrap_or("GET"),
        state.url.as_deref().unwrap_or("(not set)")
    );
    let mut headers: Vec<_> = state.headers.iter().collect();
    headers.sort_by_key(|(k, _)| k.as_str());
    for (k, v) in headers {
        eprintln!(">   {}: {}", k, v);
    }
    if let Some(body) = &state.body {
        eprintln!(">   {}", body);
    }
}
