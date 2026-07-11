use std::io::IsTerminal;

use crate::model::{Request, ResponseRecord, State, status_reason};
use crate::session::SessionStore;

use super::commands::{ResponseTarget, ResponseView};

// Print a response body to stdout. On a terminal, valid JSON is pretty-printed
// for readability; when piped, the raw bytes are written untouched so
// `reel send | jq .` sees exactly what the server sent. `pad_newline` appends
// a trailing newline in the raw path when the body lacks one (send/then
// output; `response body` stays byte-exact). Returns true when the body was
// pretty-printed — the output then already ends with a newline.
pub(super) fn print_body(body: &str, pad_newline: bool) -> bool {
    if std::io::stdout().is_terminal()
        && let Ok(v) = serde_json::from_str::<serde_json::Value>(body)
    {
        println!("{}", serde_json::to_string_pretty(&v).unwrap());
        return true;
    }
    print!("{}", body);
    if pad_newline && !body.ends_with('\n') {
        println!();
    }
    false
}

pub(super) fn format_status(code: u16) -> String {
    match status_reason(code) {
        Some(reason) => format!("{} {}", code, reason),
        None => code.to_string(),
    }
}

// Human-readable byte count (1024-based binary units, one decimal above bytes).
pub(super) fn format_size(bytes: usize) -> String {
    const KIB: f64 = 1024.0;
    let b = bytes as f64;
    if b < KIB {
        format!("{} B", bytes)
    } else if b < KIB * KIB {
        format!("{:.1} KiB", b / KIB)
    } else if b < KIB * KIB * KIB {
        format!("{:.1} MiB", b / (KIB * KIB))
    } else {
        format!("{:.1} GiB", b / (KIB * KIB * KIB))
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
        "elapsed_ms": r.elapsed_ms,
        "headers": r.headers,
        "body": body_val,
    })
}

fn show_single(r: &ResponseRecord, view: &ResponseView) {
    match view {
        ResponseView::Body => {
            print_body(&r.body, false);
        }
        ResponseView::Headers => {
            println!("{} — {}", r.status, source_label(r));
            for (k, v) in r.headers.sorted() {
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
        state.request.method.as_deref().unwrap_or("(not set)")
    );
    eprintln!(
        "  url     {}",
        state.request.url.as_deref().unwrap_or("(not set)")
    );
    for (k, v) in state.request.headers.sorted() {
        eprintln!("  header  {}: {}", k, v);
    }
    if let Some(body) = &state.request.body {
        let formatted = serde_json::from_str::<serde_json::Value>(body)
            .map(|v| serde_json::to_string_pretty(&v).unwrap())
            .unwrap_or_else(|_| body.clone());
        eprintln!("  body    {}", formatted.replace('\n', "\n          "));
    }
    for c in &state.cookies {
        eprintln!("  cookie  {}={}  ({}{})", c.name, c.value, c.domain, c.path);
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
                    // Pretty-printed output already ends with a newline; a
                    // separator on top of it would leave a blank line.
                    let pretty = print_body(&r.body, false);
                    if !pretty && i + 1 < state.responses.len() {
                        println!();
                    }
                }
            }
            ResponseView::Headers => {
                for (i, r) in state.responses.iter().enumerate() {
                    let label = source_label(r);
                    println!("[{}] {} — {}", i + 1, r.status, label);
                    for (k, v) in r.headers.sorted() {
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
    eprintln!(
        "  body @<PATH> / body -  set request body from a file / from stdin (@@ escapes a literal @)"
    );
    eprintln!("  send                   send the current request");
    eprintln!(
        "  get/post/put/patch/delete/head/options <URL>   shortcut: set method + URL and send"
    );
    eprintln!(
        "                         (the send runs after all other commands, so trailing body/header apply)"
    );
    eprintln!(
        "  --dry-run              print the request that would be sent, without sending it (position-independent)"
    );
    eprintln!(
        "  then <PATH>            load next request from file (with template interpolation) and send it"
    );
    eprintln!(
        "  expect <CONDITION>     assert on the last response; abort with exit code 1 on failure"
    );
    eprintln!(
        "                         forms: <expr>, <expr> == <v>, <expr> != <v>, <expr> contains <v>"
    );
    eprintln!("  show                   print the current session state");
    eprintln!("  curl                   print the current request as an equivalent curl command");
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
    eprintln!(
        "  --retry <N>            retry send/then up to N extra times on network error or 5xx"
    );
    eprintln!(
        "  --until <CONDITION>    poll: repeat send/then until the condition passes (see expect);"
    );
    eprintln!(
        "                         attempt budget is --retry + 1, or 10 when --retry is not given"
    );
    eprintln!("  --delay <SECONDS>      sleep between attempts (default: 1)");
    eprintln!("  -h / --help            show this help");
    eprintln!("  -V / --version         show version");
    eprintln!();
    eprintln!("Template interpolation in files loaded by 'then':");
    eprintln!("  ${{{{ status }}}}                  HTTP status code of the last response");
    eprintln!("  ${{{{ elapsed }}}}                 duration of the last request in milliseconds");
    eprintln!("  ${{{{ body }}}}                    raw body of the last response");
    eprintln!("  ${{{{ body.field.sub }}}}          dot-path into the last response body");
    eprintln!("  ${{{{ headers.name }}}}            response header value from the last response");
    eprintln!("  ${{{{ response[N].status }}}}      status of the Nth response (1-based)");
    eprintln!("  ${{{{ response[N].body }}}}        raw body of the Nth response");
    eprintln!("  ${{{{ response[N].body.field }}}}  dot-path into the Nth response body");
    eprintln!("  ${{{{ response[N].headers.name }}}} header from the Nth response");
    eprintln!("  ${{{{ request.url }}}}             URL of the last request (also .method, .body)");
    eprintln!("  ${{{{ request.body.field }}}}      dot-path into the last request body");
    eprintln!("  ${{{{ request.headers.name }}}}    header value from the last request");
    eprintln!("  ${{{{ request[N].url }}}}          field of the Nth request (1-based)");
    eprintln!("  ${{{{ env.NAME }}}}                value of environment variable NAME");
    eprintln!("  ${{{{ uuid() }}}}                  random v4 UUID");
    eprintln!(
        "  ${{{{ now() }}}} / ${{{{ now(+N) }}}}     Unix timestamp, optionally shifted by N seconds"
    );
    eprintln!("  ${{{{ base64(arg) }}}}             base64 of a 'literal' or nested expression");
    eprintln!(
        "  ${{{{ expr | default: value }}}}   fallback used when the expression cannot be resolved"
    );
    eprintln!();
    eprintln!("Examples:");
    eprintln!("  reel get https://httpbin.org/get");
    eprintln!("  reel post https://api.example.com/users body '{{\"name\":\"Alice\"}}'");
    eprintln!("  reel method GET url https://httpbin.org/get send");
    eprintln!("  reel header \"Authorization: Bearer token\"");
    eprintln!("  reel header Content-Type application/json");
    eprintln!("  reel body '{{\"key\":\"value\"}}' send");
    eprintln!("  reel response body | jq .name");
    eprintln!("  reel response headers");
    eprintln!("  reel send then step2.json then step3.json");
    eprintln!("  reel response all");
    eprintln!("  reel fail send  # exits 1 on 4xx/5xx");
    eprintln!("  reel send expect 'status == 200' expect 'body.token'");
    eprintln!();
    eprintln!(
        "Cookies: Set-Cookie responses are stored in the session and sent back automatically"
    );
    eprintln!(
        "         on matching requests. A manually set Cookie header always takes precedence."
    );
    eprintln!();
    eprintln!("Redirects: not followed (like curl without -L) — a 3xx response is shown as-is.");
    eprintln!();
    eprintln!(
        "Note: status messages and confirmations are written to stderr; response body goes to stdout."
    );
    eprintln!(
        "      This means 'reel send | jq .' works correctly even when confirmations are visible."
    );
}

// POSIX single-quote: wrap in ', escaping embedded ' as '\''.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', r"'\''"))
}

// Methods are normally plain tokens; quote only when copy-pasting the value
// into a shell would otherwise split or interpret it.
fn quote_method(method: &str) -> String {
    if !method.is_empty()
        && method
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        method.to_string()
    } else {
        shell_quote(method)
    }
}

// The current request as an equivalent curl command — for sharing in issues,
// docs, or with people who don't have reel.
pub(super) fn format_curl(request: &Request, insecure: bool) -> anyhow::Result<String> {
    let url = request
        .url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("error: URL not set (use: reel url <URL>)"))?;

    let mut parts = vec!["curl".to_string()];
    // reel defaults to GET even with a body; curl would switch to POST on
    // --data, so spell the method out whenever a body is present.
    match (&request.method, &request.body) {
        (Some(m), _) => parts.push(format!("-X {}", quote_method(m))),
        (None, Some(_)) => parts.push("-X GET".to_string()),
        (None, None) => {}
    }
    if insecure {
        parts.push("-k".to_string());
    }
    parts.push(shell_quote(url));
    for (name, value) in request.headers.sorted() {
        parts.push(format!(
            "-H {}",
            shell_quote(&format!("{}: {}", name, value))
        ));
    }
    if let Some(body) = &request.body {
        parts.push(format!("--data-binary {}", shell_quote(body)));
    }
    Ok(parts.join(" "))
}

pub(super) fn print_dry_run(request: &Request) {
    eprintln!(
        "> {} {}",
        request.method.as_deref().unwrap_or("GET"),
        request.url.as_deref().unwrap_or("(not set)")
    );
    for (k, v) in request.headers.sorted() {
        eprintln!(">   {}: {}", k, v);
    }
    if let Some(body) = &request.body {
        eprintln!(">   {}", body);
    }
}
