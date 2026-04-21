use std::path::PathBuf;

use crate::http::HttpClient;
use crate::model::{ResponseRecord, State};
use crate::session::{SessionStore, load_preset, save_preset};
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
        code.to_string()
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

pub enum Command {
    Method(String),
    Url(String),
    Header(String, String),
    HeaderRm(String),
    HeaderRmAll,
    Body(String),
    Send,
    Then(PathBuf),
    Show,
    Response(ResponseTarget, ResponseView),
    Reset,
    Save(PathBuf),
    Load(PathBuf),
}

pub struct GlobalFlags {
    pub insecure: bool,
    pub fail_on_error: bool,
    pub dry_run: bool,
}

pub struct ParseResult {
    pub do_show: bool,
    pub do_response: Option<(ResponseTarget, ResponseView)>,
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
        eprintln!("  body    {}", body);
    }
    if !state.responses.is_empty() {
        eprintln!("  responses  {} stored", state.responses.len());
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
            println!("{}", serde_json::to_string_pretty(&response_display_value(r)).unwrap())
        }
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

pub fn parse_args(args: &[String]) -> Result<(Vec<Command>, GlobalFlags), ()> {
    let insecure = args.iter().any(|a| a == "--insecure" || a == "insecure");
    let fail_on_error = args.iter().any(|a| a == "fail");
    let dry_run = args.iter().any(|a| a == "--dry-run" || a == "dry-run");
    let flags = GlobalFlags { insecure, fail_on_error, dry_run };

    let mut commands = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "send" => {
                commands.push(Command::Send);
                i += 1;
            }
            "then" => {
                if i + 1 >= args.len() {
                    eprintln!("error: 'then' requires a file path");
                    return Err(());
                }
                commands.push(Command::Then(PathBuf::from(&args[i + 1])));
                i += 2;
            }
            "show" => {
                commands.push(Command::Show);
                i += 1;
            }
            "response" => {
                let rest = &args[i + 1..];
                let (target, target_consumed) = parse_response_target(rest)?;
                let (view, view_consumed) = parse_response_view(&rest[target_consumed..]);
                commands.push(Command::Response(target, view));
                i += 1 + target_consumed + view_consumed;
            }
            "reset" => {
                commands.push(Command::Reset);
                i += 1;
            }
            "fail" | "--insecure" | "insecure" | "--dry-run" | "dry-run" => {
                i += 1;
            }
            "method" => {
                if i + 1 < args.len() {
                    commands.push(Command::Method(args[i + 1].to_uppercase()));
                    i += 2;
                } else {
                    eprintln!("error: 'method' requires a value");
                    return Err(());
                }
            }
            "url" => {
                if i + 1 < args.len() {
                    commands.push(Command::Url(args[i + 1].clone()));
                    i += 2;
                } else {
                    eprintln!("error: 'url' requires a value");
                    return Err(());
                }
            }
            "header" => {
                if i + 1 >= args.len() {
                    eprintln!("error: 'header' requires KEY:VALUE or KEY VALUE");
                    return Err(());
                }
                let next = &args[i + 1];
                if let Some(pos) = next.find(':') {
                    let key = next[..pos].trim().to_lowercase();
                    if key.is_empty() {
                        eprintln!("error: header key cannot be empty (use KEY:VALUE or KEY VALUE)");
                        return Err(());
                    }
                    let val = next[pos + 1..].trim().to_string();
                    commands.push(Command::Header(key, val));
                    i += 2;
                } else if i + 2 < args.len() {
                    commands.push(Command::Header(next.to_lowercase(), args[i + 2].clone()));
                    i += 3;
                } else {
                    eprintln!("error: 'header' requires KEY:VALUE or KEY VALUE");
                    return Err(());
                }
            }
            "header-rm-all" => {
                commands.push(Command::HeaderRmAll);
                i += 1;
            }
            "header-rm" => {
                if i + 1 < args.len() {
                    commands.push(Command::HeaderRm(args[i + 1].to_lowercase()));
                    i += 2;
                } else {
                    eprintln!("error: 'header-rm' requires a header name");
                    return Err(());
                }
            }
            "body" => {
                if i + 1 < args.len() {
                    commands.push(Command::Body(args[i + 1].clone()));
                    i += 2;
                } else {
                    eprintln!("error: 'body' requires a value");
                    return Err(());
                }
            }
            "save" => {
                if i + 1 < args.len() {
                    commands.push(Command::Save(PathBuf::from(&args[i + 1])));
                    i += 2;
                } else {
                    eprintln!("error: 'save' requires a path");
                    return Err(());
                }
            }
            "load" => {
                if i + 1 < args.len() {
                    commands.push(Command::Load(PathBuf::from(&args[i + 1])));
                    i += 2;
                } else {
                    eprintln!("error: 'load' requires a path");
                    return Err(());
                }
            }
            unknown => {
                eprintln!("error: unknown command '{}'", unknown);
                eprintln!("Run 'reel' with no arguments to see usage.");
                return Err(());
            }
        }
    }
    Ok((commands, flags))
}

pub fn run_commands(
    commands: Vec<Command>,
    flags: &GlobalFlags,
    state: &mut State,
    http: &dyn HttpClient,
    session: &dyn SessionStore,
) -> Result<ParseResult, ()> {
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
                    eprintln!("< {} (previous responses cleared)", format_status(last.status));
                } else {
                    eprintln!("< {}", format_status(last.status));
                }
                print!("{}", last.body);
                if !last.body.ends_with('\n') {
                    println!();
                }
                if flags.fail_on_error && last.status >= 400 {
                    return Err(());
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
                        eprintln!(
                            "error: '{}' uses template expressions but there is no previous response",
                            path_str
                        );
                        return Err(());
                    }
                } else {
                    let prev = state.responses.last().cloned().unwrap();
                    if let Err(e) = apply_interpolation(state, &prev) {
                        eprintln!("error in '{}': {}", path_str, e);
                        return Err(());
                    }
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
                    return Err(());
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
                    eprintln!("error: URL cannot be empty");
                    return Err(());
                }
                state.url = Some(u);
                modified = true;
            }
            Command::Header(key, val) => {
                state.headers.insert(key, val);
                modified = true;
            }
            Command::HeaderRm(key) => {
                // Case-insensitive: scan for a matching key regardless of casing in stored state.
                let found = state.headers.keys().find(|k| k.to_lowercase() == key).cloned();
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

    Ok(ParseResult { do_show, do_response })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::HttpClient;
    use crate::model::{ResponseRecord, State};
    use crate::session::SessionStore;
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::path::Path;

    // --- mocks ---

    struct MockHttp {
        response: Result<ResponseRecord, ()>,
    }

    impl MockHttp {
        fn ok(status: u16) -> Self {
            Self {
                response: Ok(ResponseRecord {
                    source: None,
                    status,
                    headers: HashMap::new(),
                    body: "{}".to_string(),
                }),
            }
        }

        fn err() -> Self {
            Self { response: Err(()) }
        }
    }

    impl HttpClient for MockHttp {
        fn execute(&self, _state: &State, source: Option<&str>) -> Result<ResponseRecord, ()> {
            self.response.clone().map(|mut r| {
                r.source = source.map(str::to_string);
                r
            })
        }
    }

    #[derive(Default)]
    struct MockSession {
        save_count: Cell<u32>,
        delete_count: Cell<u32>,
        last_saved: RefCell<Option<State>>,
    }

    impl SessionStore for MockSession {
        fn load(&self) -> State {
            State::default()
        }
        fn save(&self, state: &State) {
            self.save_count.set(self.save_count.get() + 1);
            *self.last_saved.borrow_mut() = Some(state.clone());
        }
        fn delete(&self) {
            self.delete_count.set(self.delete_count.get() + 1);
        }
        fn path(&self) -> &Path {
            Path::new("/tmp/mock-session.json")
        }
    }

    fn s(v: &str) -> String {
        v.to_string()
    }

    fn args(input: &str) -> Vec<String> {
        input.split_whitespace().map(str::to_string).collect()
    }

    // --- parse_args ---

    #[test]
    fn parse_method_and_url() {
        let (cmds, _) = parse_args(&args("method POST url https://example.com")).unwrap();
        assert_eq!(cmds.len(), 2);
        assert!(matches!(&cmds[0], Command::Method(m) if m == "POST"));
        assert!(matches!(&cmds[1], Command::Url(u) if u == "https://example.com"));
    }

    #[test]
    fn parse_method_is_uppercased() {
        let (cmds, _) = parse_args(&args("method get")).unwrap();
        assert!(matches!(&cmds[0], Command::Method(m) if m == "GET"));
    }

    #[test]
    fn parse_header_colon_form() {
        let (cmds, _) = parse_args(&[s("header"), s("Content-Type:application/json")]).unwrap();
        assert!(matches!(&cmds[0], Command::Header(k, v) if k == "content-type" && v == "application/json"));
    }

    #[test]
    fn parse_header_colon_trims_whitespace() {
        let (cmds, _) = parse_args(&[s("header"), s("X-Foo: bar")]).unwrap();
        assert!(matches!(&cmds[0], Command::Header(k, v) if k == "x-foo" && v == "bar"));
    }

    #[test]
    fn parse_header_two_token_form() {
        let (cmds, _) = parse_args(&args("header X-Foo bar")).unwrap();
        assert!(matches!(&cmds[0], Command::Header(k, v) if k == "x-foo" && v == "bar"));
    }

    #[test]
    fn parse_header_empty_key_is_error() {
        assert!(parse_args(&[s("header"), s(":value")]).is_err());
    }

    #[test]
    fn parse_header_missing_value_is_error() {
        assert!(parse_args(&args("header")).is_err());
    }

    #[test]
    fn parse_header_rm() {
        let (cmds, _) = parse_args(&args("header-rm X-Foo")).unwrap();
        assert!(matches!(&cmds[0], Command::HeaderRm(k) if k == "x-foo"));
    }

    #[test]
    fn parse_header_rm_all() {
        let (cmds, _) = parse_args(&args("header-rm-all")).unwrap();
        assert!(matches!(cmds[0], Command::HeaderRmAll));
    }

    #[test]
    fn parse_header_rm_missing_arg_is_error() {
        assert!(parse_args(&args("header-rm")).is_err());
    }

    #[test]
    fn parse_then_missing_path_is_error() {
        assert!(parse_args(&args("then")).is_err());
    }

    #[test]
    fn parse_method_missing_value_is_error() {
        assert!(parse_args(&args("method")).is_err());
    }

    #[test]
    fn parse_url_missing_value_is_error() {
        assert!(parse_args(&args("url")).is_err());
    }

    #[test]
    fn parse_body_missing_value_is_error() {
        assert!(parse_args(&args("body")).is_err());
    }

    #[test]
    fn parse_save_missing_path_is_error() {
        assert!(parse_args(&args("save")).is_err());
    }

    #[test]
    fn parse_load_missing_path_is_error() {
        assert!(parse_args(&args("load")).is_err());
    }

    #[test]
    fn parse_unknown_command_is_error() {
        assert!(parse_args(&args("frobnicate")).is_err());
    }

    #[test]
    fn parse_response_defaults_to_last_full() {
        let (cmds, _) = parse_args(&args("response")).unwrap();
        assert!(matches!(cmds[0], Command::Response(ResponseTarget::Last, ResponseView::Full)));
    }

    #[test]
    fn parse_response_with_index() {
        let (cmds, _) = parse_args(&args("response 2")).unwrap();
        assert!(matches!(cmds[0], Command::Response(ResponseTarget::Index(1), ResponseView::Full)));
    }

    #[test]
    fn parse_response_index_zero_is_error() {
        assert!(parse_args(&args("response 0")).is_err());
    }

    #[test]
    fn parse_response_all_body() {
        let (cmds, _) = parse_args(&args("response all body")).unwrap();
        assert!(matches!(cmds[0], Command::Response(ResponseTarget::All, ResponseView::Body)));
    }

    #[test]
    fn parse_response_last_headers() {
        let (cmds, _) = parse_args(&args("response headers")).unwrap();
        assert!(matches!(cmds[0], Command::Response(ResponseTarget::Last, ResponseView::Headers)));
    }

    #[test]
    fn parse_insecure_flag() {
        let (_, flags) = parse_args(&args("send --insecure")).unwrap();
        assert!(flags.insecure);
    }

    #[test]
    fn parse_insecure_keyword() {
        let (_, flags) = parse_args(&args("insecure send")).unwrap();
        assert!(flags.insecure);
    }

    #[test]
    fn parse_fail_flag() {
        let (_, flags) = parse_args(&args("fail send")).unwrap();
        assert!(flags.fail_on_error);
    }

    #[test]
    fn parse_dry_run_flag() {
        let (_, flags) = parse_args(&args("--dry-run send")).unwrap();
        assert!(flags.dry_run);
    }

    #[test]
    fn parse_global_flags_not_added_to_commands() {
        let (cmds, flags) = parse_args(&args("fail --insecure --dry-run send")).unwrap();
        assert!(flags.fail_on_error && flags.insecure && flags.dry_run);
        // Only Command::Send should remain, not the flags
        assert_eq!(cmds.len(), 1);
        assert!(matches!(cmds[0], Command::Send));
    }

    // --- run_commands ---

    fn run(input: &str, state: &mut State, http: &dyn HttpClient, session: &dyn SessionStore) -> Result<ParseResult, ()> {
        let (cmds, flags) = parse_args(&args(input)).unwrap();
        run_commands(cmds, &flags, state, http, session)
    }

    #[test]
    fn method_url_header_saves_session() {
        let http = MockHttp::ok(200);
        let session = MockSession::default();
        let mut state = State::default();

        run("method GET url https://example.com header X-Foo:bar", &mut state, &http, &session).unwrap();

        assert_eq!(state.method, Some("GET".to_string()));
        assert_eq!(state.url, Some("https://example.com".to_string()));
        assert_eq!(state.headers.get("x-foo").map(String::as_str), Some("bar"));
        assert_eq!(session.save_count.get(), 1);
    }

    #[test]
    fn send_calls_http_and_saves() {
        let http = MockHttp::ok(200);
        let session = MockSession::default();
        let mut state = State::default();
        state.url = Some("https://example.com".to_string());

        run("send", &mut state, &http, &session).unwrap();

        assert_eq!(state.responses.len(), 1);
        assert_eq!(state.responses[0].status, 200);
        assert_eq!(session.save_count.get(), 1);
    }

    #[test]
    fn send_http_failure_returns_err() {
        let http = MockHttp::err();
        let session = MockSession::default();
        let mut state = State::default();
        state.url = Some("https://example.com".to_string());

        assert!(run("send", &mut state, &http, &session).is_err());
    }

    #[test]
    fn fail_flag_on_4xx_returns_err() {
        let http = MockHttp::ok(404);
        let session = MockSession::default();
        let mut state = State::default();
        state.url = Some("https://example.com".to_string());

        let (cmds, flags) = parse_args(&args("fail send")).unwrap();
        assert!(run_commands(cmds, &flags, &mut state, &http, &session).is_err());
    }

    #[test]
    fn fail_flag_on_2xx_succeeds() {
        let http = MockHttp::ok(200);
        let session = MockSession::default();
        let mut state = State::default();
        state.url = Some("https://example.com".to_string());

        let (cmds, flags) = parse_args(&args("fail send")).unwrap();
        assert!(run_commands(cmds, &flags, &mut state, &http, &session).is_ok());
    }

    #[test]
    fn dry_run_skips_http() {
        let http = MockHttp::err(); // would fail if called
        let session = MockSession::default();
        let mut state = State::default();
        state.url = Some("https://example.com".to_string());

        let (cmds, flags) = parse_args(&args("--dry-run send")).unwrap();
        run_commands(cmds, &flags, &mut state, &http, &session).unwrap();

        assert!(state.responses.is_empty());
        assert_eq!(session.save_count.get(), 0);
    }

    #[test]
    fn reset_clears_state_and_deletes_session() {
        let http = MockHttp::ok(200);
        let session = MockSession::default();
        let mut state = State::default();
        state.method = Some("POST".to_string());
        state.url = Some("https://example.com".to_string());

        run("reset", &mut state, &http, &session).unwrap();

        assert!(state.method.is_none());
        assert!(state.url.is_none());
        assert_eq!(session.delete_count.get(), 1);
        assert_eq!(session.save_count.get(), 0);
    }

    #[test]
    fn header_rm_removes_existing() {
        let http = MockHttp::ok(200);
        let session = MockSession::default();
        let mut state = State::default();
        state.headers.insert("x-foo".to_string(), "bar".to_string());

        run("header-rm x-foo", &mut state, &http, &session).unwrap();

        assert!(!state.headers.contains_key("x-foo"));
        assert_eq!(session.save_count.get(), 1);
    }

    #[test]
    fn header_rm_all_clears_all_headers() {
        let http = MockHttp::ok(200);
        let session = MockSession::default();
        let mut state = State::default();
        state.headers.insert("x-a".to_string(), "1".to_string());
        state.headers.insert("x-b".to_string(), "2".to_string());

        run("header-rm-all", &mut state, &http, &session).unwrap();

        assert!(state.headers.is_empty());
        assert_eq!(session.save_count.get(), 1);
    }

    #[test]
    fn header_rm_missing_does_not_save() {
        let http = MockHttp::ok(200);
        let session = MockSession::default();
        let mut state = State::default();

        run("header-rm x-nonexistent", &mut state, &http, &session).unwrap();

        assert_eq!(session.save_count.get(), 0);
    }

    #[test]
    fn send_clears_previous_responses() {
        let http = MockHttp::ok(201);
        let session = MockSession::default();
        let mut state = State::default();
        state.url = Some("https://example.com".to_string());
        // Pre-load two fake responses
        state.responses.push(ResponseRecord { source: None, status: 200, headers: HashMap::new(), body: "old".to_string() });
        state.responses.push(ResponseRecord { source: None, status: 200, headers: HashMap::new(), body: "old2".to_string() });

        run("send", &mut state, &http, &session).unwrap();

        assert_eq!(state.responses.len(), 1);
        assert_eq!(state.responses[0].status, 201);
    }

    #[test]
    fn show_command_sets_do_show() {
        let http = MockHttp::ok(200);
        let session = MockSession::default();
        let mut state = State::default();

        let result = run("show", &mut state, &http, &session).unwrap();
        assert!(result.do_show);
    }

    #[test]
    fn response_command_sets_do_response() {
        let http = MockHttp::ok(200);
        let session = MockSession::default();
        let mut state = State::default();

        let result = run("response body", &mut state, &http, &session).unwrap();
        assert!(result.do_response.is_some());
    }

    #[test]
    fn no_mutation_does_not_save_session() {
        let http = MockHttp::ok(200);
        let session = MockSession::default();
        let mut state = State::default();

        run("show", &mut state, &http, &session).unwrap();

        assert_eq!(session.save_count.get(), 0);
    }
}
