use super::commands::{Command, ParseResult, ResponseTarget, ResponseView};
use super::*;
use crate::http::HttpClient;
use crate::model::{Request, ResponseRecord, State};
use crate::session::SessionStore;
use std::cell::{Cell, RefCell};
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
                headers: Default::default(),
                body: "{}".to_string(),
            }),
        }
    }

    fn err() -> Self {
        Self { response: Err(()) }
    }
}

impl HttpClient for MockHttp {
    fn execute(&self, _request: &Request, source: Option<&str>) -> anyhow::Result<ResponseRecord> {
        self.response
            .clone()
            .map(|mut r| {
                r.source = source.map(str::to_string);
                r
            })
            .map_err(|()| anyhow::anyhow!("mock http error"))
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
    fn save(&self, state: &State) -> anyhow::Result<()> {
        self.save_count.set(self.save_count.get() + 1);
        *self.last_saved.borrow_mut() = Some(state.clone());
        Ok(())
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
    assert!(
        matches!(&cmds[0], Command::Header(k, v) if k == "Content-Type" && v == "application/json")
    );
}

#[test]
fn parse_header_colon_trims_whitespace() {
    let (cmds, _) = parse_args(&[s("header"), s("X-Foo: bar")]).unwrap();
    assert!(matches!(&cmds[0], Command::Header(k, v) if k == "X-Foo" && v == "bar"));
}

#[test]
fn parse_header_two_token_form() {
    let (cmds, _) = parse_args(&args("header X-Foo bar")).unwrap();
    assert!(matches!(&cmds[0], Command::Header(k, v) if k == "X-Foo" && v == "bar"));
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
    assert!(matches!(
        cmds[0],
        Command::Response(ResponseTarget::Last, ResponseView::Full)
    ));
}

#[test]
fn parse_response_with_index() {
    let (cmds, _) = parse_args(&args("response 2")).unwrap();
    assert!(matches!(
        cmds[0],
        Command::Response(ResponseTarget::Index(1), ResponseView::Full)
    ));
}

#[test]
fn parse_response_index_zero_is_error() {
    assert!(parse_args(&args("response 0")).is_err());
}

#[test]
fn parse_response_all_body() {
    let (cmds, _) = parse_args(&args("response all body")).unwrap();
    assert!(matches!(
        cmds[0],
        Command::Response(ResponseTarget::All, ResponseView::Body)
    ));
}

#[test]
fn parse_response_last_headers() {
    let (cmds, _) = parse_args(&args("response headers")).unwrap();
    assert!(matches!(
        cmds[0],
        Command::Response(ResponseTarget::Last, ResponseView::Headers)
    ));
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

fn run(
    input: &str,
    state: &mut State,
    http: &dyn HttpClient,
    session: &dyn SessionStore,
) -> anyhow::Result<ParseResult> {
    let (cmds, flags) = parse_args(&args(input)).unwrap();
    run_commands(cmds, &flags, state, http, session)
}

#[test]
fn method_url_header_saves_session() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();

    run(
        "method GET url https://example.com header X-Foo:bar",
        &mut state,
        &http,
        &session,
    )
    .unwrap();

    assert_eq!(state.request.method, Some("GET".to_string()));
    assert_eq!(state.request.url, Some("https://example.com".to_string()));
    assert_eq!(state.request.headers.get("X-Foo"), Some("bar"));
    assert_eq!(session.save_count.get(), 1);
}

#[test]
fn send_calls_http_and_saves() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

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
    state.request.url = Some("https://example.com".to_string());

    assert!(run("send", &mut state, &http, &session).is_err());
}

#[test]
fn fail_flag_on_4xx_returns_err() {
    let http = MockHttp::ok(404);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

    let (cmds, flags) = parse_args(&args("fail send")).unwrap();
    assert!(run_commands(cmds, &flags, &mut state, &http, &session).is_err());
}

#[test]
fn fail_flag_on_2xx_succeeds() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

    let (cmds, flags) = parse_args(&args("fail send")).unwrap();
    assert!(run_commands(cmds, &flags, &mut state, &http, &session).is_ok());
}

#[test]
fn dry_run_skips_http() {
    let http = MockHttp::err(); // would fail if called
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

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
    state.request.method = Some("POST".to_string());
    state.request.url = Some("https://example.com".to_string());

    run("reset", &mut state, &http, &session).unwrap();

    assert!(state.request.method.is_none());
    assert!(state.request.url.is_none());
    assert_eq!(session.delete_count.get(), 1);
    assert_eq!(session.save_count.get(), 0);
}

#[test]
fn header_rm_removes_existing() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();
    state
        .request
        .headers
        .insert("x-foo".to_string(), "bar".to_string());

    run("header-rm x-foo", &mut state, &http, &session).unwrap();

    assert!(state.request.headers.get("x-foo").is_none());
    assert_eq!(session.save_count.get(), 1);
}

#[test]
fn header_rm_all_clears_all_headers() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();
    state
        .request
        .headers
        .insert("x-a".to_string(), "1".to_string());
    state
        .request
        .headers
        .insert("x-b".to_string(), "2".to_string());

    run("header-rm-all", &mut state, &http, &session).unwrap();

    assert!(state.request.headers.is_empty());
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
    state.request.url = Some("https://example.com".to_string());
    // Pre-load two fake responses
    state.responses.push(ResponseRecord {
        source: None,
        status: 200,
        headers: Default::default(),
        body: "old".to_string(),
    });
    state.responses.push(ResponseRecord {
        source: None,
        status: 200,
        headers: Default::default(),
        body: "old2".to_string(),
    });

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
