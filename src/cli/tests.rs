use super::commands::{Command, ParseResult, ResponseTarget, ResponseView};
use super::*;
use crate::http::HttpClient;
use crate::model::{Request, ResponseRecord, State};
use crate::session::{PresetStore, SessionStore};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// --- mocks ---

fn response(status: u16, body: &str) -> ResponseRecord {
    ResponseRecord {
        status,
        body: body.to_string(),
        ..Default::default()
    }
}

// Responses are consumed front-to-back; the last one repeats forever.
struct MockHttp {
    responses: RefCell<Vec<Result<ResponseRecord, ()>>>,
    seen: RefCell<Vec<Request>>,
}

impl MockHttp {
    fn sequence(responses: Vec<Result<ResponseRecord, ()>>) -> Self {
        assert!(!responses.is_empty());
        Self {
            responses: RefCell::new(responses),
            seen: RefCell::new(Vec::new()),
        }
    }

    fn ok(status: u16) -> Self {
        Self::sequence(vec![Ok(response(status, "{}"))])
    }

    fn err() -> Self {
        Self::sequence(vec![Err(())])
    }

    fn with_set_cookies(self, cookies: &[&str]) -> Self {
        for r in self.responses.borrow_mut().iter_mut().flatten() {
            r.set_cookies = cookies.iter().map(|s| s.to_string()).collect();
        }
        self
    }
}

impl HttpClient for MockHttp {
    fn execute(&self, request: &Request, source: Option<&str>) -> anyhow::Result<ResponseRecord> {
        self.seen.borrow_mut().push(request.clone());
        let mut responses = self.responses.borrow_mut();
        let response = if responses.len() > 1 {
            responses.remove(0)
        } else {
            responses[0].clone()
        };
        response
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

// In-memory preset files keyed by path.
#[derive(Default)]
struct MockPresets {
    files: HashMap<PathBuf, State>,
    saved: RefCell<Vec<(PathBuf, Request)>>,
}

impl MockPresets {
    fn with(path: &str, state: State) -> Self {
        let mut presets = Self::default();
        presets.files.insert(PathBuf::from(path), state);
        presets
    }
}

impl PresetStore for MockPresets {
    fn load(&self, path: &Path) -> anyhow::Result<State> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("error reading '{}': not found", path.display()))
    }
    fn save(&self, request: &Request, path: &Path) -> anyhow::Result<()> {
        self.saved
            .borrow_mut()
            .push((path.to_path_buf(), request.clone()));
        Ok(())
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
    run_with_presets(input, state, http, session, &MockPresets::default())
}

fn run_with_presets(
    input: &str,
    state: &mut State,
    http: &dyn HttpClient,
    session: &dyn SessionStore,
    presets: &dyn PresetStore,
) -> anyhow::Result<ParseResult> {
    let (cmds, flags) = parse_args(&args(input)).unwrap();
    run_commands(cmds, &flags, state, http, session, presets)
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
    assert!(
        run_commands(
            cmds,
            &flags,
            &mut state,
            &http,
            &session,
            &MockPresets::default()
        )
        .is_err()
    );
}

#[test]
fn fail_flag_on_2xx_succeeds() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

    let (cmds, flags) = parse_args(&args("fail send")).unwrap();
    assert!(
        run_commands(
            cmds,
            &flags,
            &mut state,
            &http,
            &session,
            &MockPresets::default()
        )
        .is_ok()
    );
}

#[test]
fn dry_run_skips_http() {
    let http = MockHttp::err(); // would fail if called
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

    let (cmds, flags) = parse_args(&args("--dry-run send")).unwrap();
    run_commands(
        cmds,
        &flags,
        &mut state,
        &http,
        &session,
        &MockPresets::default(),
    )
    .unwrap();

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
        status: 200,
        body: "old".to_string(),
        ..Default::default()
    });
    state.responses.push(ResponseRecord {
        status: 200,
        body: "old2".to_string(),
        ..Default::default()
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

// --- load / save / then (preset store) ---

fn preset(method: &str, url: &str) -> State {
    let mut state = State::default();
    state.request.method = Some(method.to_string());
    state.request.url = Some(url.to_string());
    state
}

#[test]
fn load_replaces_state_and_saves_session() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let presets = MockPresets::with("p.json", preset("POST", "https://preset.example"));
    let mut state = State::default();
    state.request.url = Some("https://old.example".to_string());

    run_with_presets("load p.json", &mut state, &http, &session, &presets).unwrap();

    assert_eq!(state.request.method, Some("POST".to_string()));
    assert_eq!(
        state.request.url,
        Some("https://preset.example".to_string())
    );
    assert_eq!(session.save_count.get(), 1);
}

#[test]
fn load_missing_preset_is_error() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();

    assert!(run("load missing.json", &mut state, &http, &session).is_err());
}

#[test]
fn save_writes_current_request_to_preset_store() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let presets = MockPresets::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

    run_with_presets("save out.json", &mut state, &http, &session, &presets).unwrap();

    let saved = presets.saved.borrow();
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].0, PathBuf::from("out.json"));
    assert_eq!(saved[0].1.url, Some("https://example.com".to_string()));
    // `save` writes a preset; it does not modify the session.
    assert_eq!(session.save_count.get(), 0);
}

#[test]
fn then_appends_to_history() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let presets = MockPresets::with("step2.json", preset("GET", "https://step2.example"));
    let mut state = State::default();
    state.request.url = Some("https://step1.example".to_string());

    run_with_presets(
        "send then step2.json",
        &mut state,
        &http,
        &session,
        &presets,
    )
    .unwrap();

    assert_eq!(state.requests.len(), 2);
    assert_eq!(state.responses.len(), 2);
    assert_eq!(
        state.requests[1].url,
        Some("https://step2.example".to_string())
    );
    assert_eq!(state.responses[1].source.as_deref(), Some("step2.json"));
}

#[test]
fn then_interpolates_from_previous_response() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let presets = MockPresets::with(
        "step2.json",
        preset("GET", "https://example.com/${{ body.token }}"),
    );
    let mut state = State::default();
    state.responses.push(ResponseRecord {
        status: 200,
        body: r#"{"token":"abc"}"#.to_string(),
        ..Default::default()
    });

    run_with_presets("then step2.json", &mut state, &http, &session, &presets).unwrap();

    // The history snapshot holds the request as actually sent.
    assert_eq!(
        state.requests.last().unwrap().url,
        Some("https://example.com/abc".to_string())
    );
    assert_eq!(state.responses.len(), 2);
}

#[test]
fn then_with_missing_history_is_error() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let presets = MockPresets::with(
        "step2.json",
        preset("GET", "https://example.com/${{ body.token }}"),
    );
    let mut state = State::default();

    assert!(run_with_presets("then step2.json", &mut state, &http, &session, &presets).is_err());
    assert!(state.responses.is_empty());
}

#[test]
fn then_missing_preset_is_error() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();

    assert!(run("then missing.json", &mut state, &http, &session).is_err());
}

// --- expect ---

#[test]
fn parse_expect() {
    let (cmds, _) = parse_args(&[s("expect"), s("status == 200")]).unwrap();
    assert!(matches!(&cmds[0], Command::Expect(c) if c == "status == 200"));
}

#[test]
fn parse_expect_missing_condition_is_error() {
    assert!(parse_args(&args("expect")).is_err());
}

#[test]
fn expect_passes_after_send() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

    let (cmds, flags) = parse_args(&[s("send"), s("expect"), s("status == 200")]).unwrap();
    run_commands(
        cmds,
        &flags,
        &mut state,
        &http,
        &session,
        &MockPresets::default(),
    )
    .unwrap();
}

#[test]
fn expect_failure_aborts_chain() {
    let http = MockHttp::ok(500);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

    // The failing expect must stop the second send from running.
    let (cmds, flags) =
        parse_args(&[s("send"), s("expect"), s("status == 200"), s("send")]).unwrap();
    let err = run_commands(
        cmds,
        &flags,
        &mut state,
        &http,
        &session,
        &MockPresets::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("expect failed"), "{err}");
    assert_eq!(http.seen.borrow().len(), 1);
}

#[test]
fn expect_without_response_is_error() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();

    let (cmds, flags) = parse_args(&[s("expect"), s("status == 200")]).unwrap();
    assert!(
        run_commands(
            cmds,
            &flags,
            &mut state,
            &http,
            &session,
            &MockPresets::default(),
        )
        .is_err()
    );
}

#[test]
fn dry_run_skips_expect() {
    let http = MockHttp::err();
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

    let (cmds, flags) =
        parse_args(&[s("--dry-run"), s("send"), s("expect"), s("status == 200")]).unwrap();
    run_commands(
        cmds,
        &flags,
        &mut state,
        &http,
        &session,
        &MockPresets::default(),
    )
    .unwrap();
}

// --- curl export ---

#[test]
fn parse_curl() {
    let (cmds, _) = parse_args(&args("curl")).unwrap();
    assert!(matches!(cmds[0], Command::Curl));
}

#[test]
fn format_curl_full_request() {
    let mut request = Request {
        method: Some("POST".to_string()),
        url: Some("https://api.example.com/users".to_string()),
        body: Some(r#"{"name":"Alice"}"#.to_string()),
        ..Default::default()
    };
    request
        .headers
        .insert("Content-Type".to_string(), "application/json".to_string());
    request
        .headers
        .insert("Authorization".to_string(), "Bearer tok".to_string());

    let cmd = display::format_curl(&request, false).unwrap();
    assert_eq!(
        cmd,
        r#"curl -X POST 'https://api.example.com/users' -H 'Authorization: Bearer tok' -H 'Content-Type: application/json' --data-binary '{"name":"Alice"}'"#
    );
}

#[test]
fn format_curl_quotes_apostrophes() {
    let request = Request {
        url: Some("https://example.com".to_string()),
        body: Some("it's".to_string()),
        ..Default::default()
    };

    let cmd = display::format_curl(&request, false).unwrap();
    // Body forces an explicit -X GET (curl would otherwise switch to POST).
    assert_eq!(
        cmd,
        r#"curl -X GET 'https://example.com' --data-binary 'it'\''s'"#
    );
}

#[test]
fn format_curl_bare_get_and_insecure() {
    let request = Request {
        url: Some("https://example.com".to_string()),
        ..Default::default()
    };
    assert_eq!(
        display::format_curl(&request, true).unwrap(),
        "curl -k 'https://example.com'"
    );
}

#[test]
fn format_curl_without_url_is_error() {
    assert!(display::format_curl(&Request::default(), false).is_err());
}

#[test]
fn curl_command_does_not_modify_session() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

    run("curl", &mut state, &http, &session).unwrap();

    assert_eq!(session.save_count.get(), 0);
    assert!(http.seen.borrow().is_empty());
}

#[test]
fn curl_without_url_is_error() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();

    assert!(run("curl", &mut state, &http, &session).is_err());
}

// --- verb shortcuts ---

#[test]
fn parse_verb_expands_to_method_url_send() {
    let (cmds, _) = parse_args(&args("get https://example.com")).unwrap();
    assert_eq!(cmds.len(), 3);
    assert!(matches!(&cmds[0], Command::Method(m) if m == "GET"));
    assert!(matches!(&cmds[1], Command::Url(u) if u == "https://example.com"));
    assert!(matches!(cmds[2], Command::Send));
}

#[test]
fn parse_verb_implied_send_comes_after_modifiers() {
    let (cmds, _) = parse_args(&args("post https://example.com body {} header X-A:1")).unwrap();
    // body and header written after the verb still apply before the send.
    assert!(matches!(cmds[0], Command::Method(_)));
    assert!(matches!(cmds[1], Command::Url(_)));
    assert!(matches!(cmds[2], Command::Body(_)));
    assert!(matches!(cmds[3], Command::Header(..)));
    assert!(matches!(cmds[4], Command::Send));
}

#[test]
fn parse_verb_implied_send_precedes_expect() {
    let (cmds, _) = parse_args(&[
        s("get"),
        s("https://example.com"),
        s("expect"),
        s("status == 200"),
    ])
    .unwrap();
    assert!(matches!(cmds[2], Command::Send));
    assert!(matches!(&cmds[3], Command::Expect(_)));
}

#[test]
fn parse_verb_implied_send_stays_after_leading_expect() {
    // An expect written before the verb asserts on the prior session state;
    // the implied send must not jump in front of method/url.
    let (cmds, _) = parse_args(&[
        s("expect"),
        s("status == 200"),
        s("get"),
        s("https://example.com"),
        s("expect"),
        s("body.ok"),
    ])
    .unwrap();
    assert!(matches!(&cmds[0], Command::Expect(_)));
    assert!(matches!(cmds[1], Command::Method(_)));
    assert!(matches!(cmds[2], Command::Url(_)));
    assert!(matches!(cmds[3], Command::Send));
    assert!(matches!(&cmds[4], Command::Expect(_)));
}

#[test]
fn parse_second_verb_shortcut_is_error() {
    let err = parse_args(&args("get https://a.example post https://b.example")).unwrap_err();
    assert!(err.to_string().contains("verb shortcut"), "{err}");
}

#[test]
fn parse_verb_with_explicit_send_adds_no_extra() {
    let (cmds, _) = parse_args(&args("get https://example.com send")).unwrap();
    let sends = cmds.iter().filter(|c| matches!(c, Command::Send)).count();
    assert_eq!(sends, 1);
}

#[test]
fn parse_verb_without_url_is_error() {
    assert!(parse_args(&args("get")).is_err());
    assert!(parse_args(&args("post")).is_err());
}

#[test]
fn verb_shortcut_sends_request() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();

    run(
        "post https://example.com body {\"a\":1}",
        &mut state,
        &http,
        &session,
    )
    .unwrap();

    let seen = http.seen.borrow();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method.as_deref(), Some("POST"));
    assert_eq!(seen[0].url.as_deref(), Some("https://example.com"));
    assert_eq!(seen[0].body.as_deref(), Some("{\"a\":1}"));
}

// --- body @file / stdin ---

#[test]
fn body_at_reads_file() {
    let path = std::env::temp_dir().join(format!("reel_body_{}.json", std::process::id()));
    std::fs::write(&path, "{\"from\":\"file\"}\n").unwrap();

    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();
    run(
        &format!("body @{}", path.display()),
        &mut state,
        &http,
        &session,
    )
    .unwrap();

    // File contents are taken verbatim, trailing newline included.
    assert_eq!(state.request.body.as_deref(), Some("{\"from\":\"file\"}\n"));
    std::fs::remove_file(path).ok();
}

#[test]
fn body_at_missing_file_is_error() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();

    let err = run("body @/nonexistent/reel.json", &mut state, &http, &session).unwrap_err();
    assert!(err.to_string().contains("body file"), "{err}");
}

#[test]
fn body_double_at_is_literal() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();

    run("body @@handle", &mut state, &http, &session).unwrap();

    assert_eq!(state.request.body.as_deref(), Some("@handle"));
}

// --- retry / polling ---

#[test]
fn parse_retry_until_delay_flags() {
    let (cmds, flags) =
        parse_args(&[s("--retry"), s("3"), s("--delay"), s("0"), s("send")]).unwrap();
    assert_eq!(flags.retry, 3);
    assert_eq!(flags.delay_secs, 0);
    assert_eq!(cmds.len(), 1);

    let (_, flags) = parse_args(&[s("--until"), s("status == 200"), s("send")]).unwrap();
    assert_eq!(flags.until.as_deref(), Some("status == 200"));
}

#[test]
fn parse_retry_invalid_value_is_error() {
    assert!(parse_args(&args("--retry x send")).is_err());
    assert!(parse_args(&args("--retry")).is_err());
    assert!(parse_args(&args("--delay x")).is_err());
}

#[test]
fn retry_on_5xx_succeeds_on_second_attempt() {
    let http = MockHttp::sequence(vec![Ok(response(503, "busy")), Ok(response(200, "ok"))]);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

    run("--retry 2 --delay 0 send", &mut state, &http, &session).unwrap();

    assert_eq!(http.seen.borrow().len(), 2);
    // Only the final attempt is recorded.
    assert_eq!(state.responses.len(), 1);
    assert_eq!(state.responses[0].status, 200);
}

#[test]
fn retry_exhausted_records_last_response() {
    let http = MockHttp::ok(503);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

    run("--retry 1 --delay 0 send", &mut state, &http, &session).unwrap();

    assert_eq!(http.seen.borrow().len(), 2);
    assert_eq!(state.responses[0].status, 503);
}

#[test]
fn no_retry_without_flag() {
    let http = MockHttp::sequence(vec![Ok(response(503, "busy")), Ok(response(200, "ok"))]);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

    run("send", &mut state, &http, &session).unwrap();

    assert_eq!(http.seen.borrow().len(), 1);
    assert_eq!(state.responses[0].status, 503);
}

#[test]
fn retry_recovers_from_network_error() {
    let http = MockHttp::sequence(vec![Err(()), Ok(response(200, "ok"))]);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

    run("--retry 1 --delay 0 send", &mut state, &http, &session).unwrap();

    assert_eq!(state.responses[0].status, 200);
}

#[test]
fn network_error_without_retry_budget_is_error() {
    let http = MockHttp::err();
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

    assert!(run("--retry 0 --delay 0 send", &mut state, &http, &session).is_err());
}

#[test]
fn does_not_retry_4xx() {
    let http = MockHttp::sequence(vec![Ok(response(404, "no")), Ok(response(200, "ok"))]);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

    run("--retry 2 --delay 0 send", &mut state, &http, &session).unwrap();

    assert_eq!(http.seen.borrow().len(), 1);
    assert_eq!(state.responses[0].status, 404);
}

#[test]
fn until_polls_until_condition_met() {
    let http = MockHttp::sequence(vec![
        Ok(response(200, r#"{"state":"pending"}"#)),
        Ok(response(200, r#"{"state":"pending"}"#)),
        Ok(response(200, r#"{"state":"ready"}"#)),
    ]);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

    let (cmds, flags) = parse_args(&[
        s("--until"),
        s("body.state == ready"),
        s("--delay"),
        s("0"),
        s("send"),
    ])
    .unwrap();
    run_commands(
        cmds,
        &flags,
        &mut state,
        &http,
        &session,
        &MockPresets::default(),
    )
    .unwrap();

    assert_eq!(http.seen.borrow().len(), 3);
    // Only the response that satisfied the condition is recorded.
    assert_eq!(state.responses.len(), 1);
    assert!(state.responses[0].body.contains("ready"));
}

#[test]
fn until_budget_exhausted_records_and_errors() {
    let http = MockHttp::ok(200); // body "{}" never matches
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

    let (cmds, flags) = parse_args(&[
        s("--until"),
        s("body.state == ready"),
        s("--retry"),
        s("2"),
        s("--delay"),
        s("0"),
        s("send"),
    ])
    .unwrap();
    let err = run_commands(
        cmds,
        &flags,
        &mut state,
        &http,
        &session,
        &MockPresets::default(),
    )
    .unwrap_err();

    assert!(err.to_string().contains("not met after 3"), "{err}");
    assert_eq!(http.seen.borrow().len(), 3);
    // The last response is still recorded and persisted for inspection.
    assert_eq!(state.responses.len(), 1);
    assert_eq!(session.save_count.get(), 1);
}

#[test]
fn until_polling_updates_jar_from_every_attempt() {
    // A cookie set by a pending (retried) response must enter the jar and be
    // sent back on the next attempt.
    let mut pending = response(200, r#"{"state":"pending"}"#);
    pending.set_cookies = vec!["poll=abc; Path=/".to_string()];
    let http = MockHttp::sequence(vec![Ok(pending), Ok(response(200, r#"{"state":"ready"}"#))]);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com/job".to_string());

    let (cmds, flags) = parse_args(&[
        s("--until"),
        s("body.state == ready"),
        s("--delay"),
        s("0"),
        s("send"),
    ])
    .unwrap();
    run_commands(
        cmds,
        &flags,
        &mut state,
        &http,
        &session,
        &MockPresets::default(),
    )
    .unwrap();

    assert_eq!(state.cookies.len(), 1);
    assert_eq!(state.cookies[0].name, "poll");
    let seen = http.seen.borrow();
    assert_eq!(seen[0].headers.get("cookie"), None);
    assert_eq!(seen[1].headers.get("cookie"), Some("poll=abc"));
    // The history snapshot reflects the final request as actually sent.
    assert_eq!(state.requests[0].headers.get("cookie"), Some("poll=abc"));
}

#[test]
fn retry_updates_jar_from_failed_attempt() {
    let mut busy = response(503, "busy");
    busy.set_cookies = vec!["lb=node2; Path=/".to_string()];
    let http = MockHttp::sequence(vec![Ok(busy), Ok(response(200, "ok"))]);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com/".to_string());

    run("--retry 1 --delay 0 send", &mut state, &http, &session).unwrap();

    assert_eq!(state.cookies.len(), 1);
    assert_eq!(
        http.seen.borrow()[1].headers.get("cookie"),
        Some("lb=node2")
    );
}

// --- cookie jar ---

fn jar_cookie(name: &str, value: &str, domain: &str) -> crate::model::Cookie {
    crate::model::Cookie {
        name: name.to_string(),
        value: value.to_string(),
        domain: domain.to_string(),
        path: "/".to_string(),
        secure: false,
        host_only: true,
    }
}

#[test]
fn send_stores_cookies_from_response() {
    let http = MockHttp::ok(200).with_set_cookies(&["session=abc; Path=/"]);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com/login".to_string());

    run("send", &mut state, &http, &session).unwrap();

    assert_eq!(state.cookies.len(), 1);
    assert_eq!(state.cookies[0].name, "session");
    assert_eq!(state.cookies[0].value, "abc");
    assert_eq!(state.cookies[0].domain, "example.com");
    // The jar is part of the persisted session.
    assert_eq!(
        session.last_saved.borrow().as_ref().unwrap().cookies.len(),
        1
    );
}

#[test]
fn send_includes_cookie_header_from_jar() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com/data".to_string());
    state
        .cookies
        .push(jar_cookie("session", "abc", "example.com"));

    run("send", &mut state, &http, &session).unwrap();

    let seen = http.seen.borrow();
    assert_eq!(seen[0].headers.get("Cookie"), Some("session=abc"));
    // The history snapshot reflects the request as actually sent.
    assert_eq!(state.requests[0].headers.get("Cookie"), Some("session=abc"));
}

#[test]
fn user_cookie_header_wins_over_jar() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com/".to_string());
    state
        .request
        .headers
        .insert("Cookie".to_string(), "mine=1".to_string());
    state
        .cookies
        .push(jar_cookie("session", "abc", "example.com"));

    run("send", &mut state, &http, &session).unwrap();

    assert_eq!(http.seen.borrow()[0].headers.get("cookie"), Some("mine=1"));
}

#[test]
fn cookie_not_sent_to_other_domain() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://other.example/".to_string());
    state
        .cookies
        .push(jar_cookie("session", "abc", "example.com"));

    run("send", &mut state, &http, &session).unwrap();

    assert_eq!(http.seen.borrow()[0].headers.get("cookie"), None);
}

#[test]
fn max_age_zero_removes_cookie_from_jar() {
    let http = MockHttp::ok(200).with_set_cookies(&["session=; Path=/; Max-Age=0"]);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com/logout".to_string());
    state
        .cookies
        .push(jar_cookie("session", "abc", "example.com"));

    run("send", &mut state, &http, &session).unwrap();

    assert!(state.cookies.is_empty());
}

#[test]
fn send_keeps_jar_but_clears_history() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com/".to_string());
    state
        .cookies
        .push(jar_cookie("session", "abc", "example.com"));
    state.responses.push(ResponseRecord {
        status: 200,
        body: "old".to_string(),
        ..Default::default()
    });

    run("send", &mut state, &http, &session).unwrap();

    assert_eq!(state.responses.len(), 1);
    assert_eq!(state.cookies.len(), 1);
}

// --- flags in value position ---

#[test]
fn flag_words_consumed_as_values_are_not_flags() {
    // A token consumed as a command's value must never flip a global flag —
    // `header X-Mode insecure` must not disable TLS verification.
    let (cmds, flags) = parse_args(&args("header X-Mode insecure send")).unwrap();
    assert!(!flags.insecure);
    assert!(matches!(&cmds[0], Command::Header(k, v) if k == "X-Mode" && v == "insecure"));

    let (cmds, flags) = parse_args(&args("body fail")).unwrap();
    assert!(!flags.fail_on_error);
    assert!(matches!(&cmds[0], Command::Body(b) if b == "fail"));

    let (cmds, flags) = parse_args(&args("body -h")).unwrap();
    assert!(!flags.help);
    assert!(matches!(&cmds[0], Command::Body(b) if b == "-h"));
}

#[test]
fn help_and_version_are_parser_flags() {
    let (_, flags) = parse_args(&args("--help")).unwrap();
    assert!(flags.help && !flags.version);
    let (_, flags) = parse_args(&args("help")).unwrap();
    assert!(flags.help);
    let (_, flags) = parse_args(&args("send -V")).unwrap();
    assert!(flags.version && !flags.help);
}

// --- retry classification ---

#[test]
fn does_not_retry_501() {
    let http = MockHttp::sequence(vec![Ok(response(501, "no")), Ok(response(200, "ok"))]);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

    run("--retry 2 --delay 0 send", &mut state, &http, &session).unwrap();

    assert_eq!(http.seen.borrow().len(), 1);
    assert_eq!(state.responses[0].status, 501);
}

#[test]
fn permanent_error_is_not_retried() {
    struct PermanentHttp {
        calls: Cell<u32>,
    }
    impl HttpClient for PermanentHttp {
        fn execute(
            &self,
            _request: &Request,
            _source: Option<&str>,
        ) -> anyhow::Result<ResponseRecord> {
            self.calls.set(self.calls.get() + 1);
            Err(anyhow::Error::new(crate::http::PermanentError(
                "error: invalid URL 'nope'".to_string(),
            )))
        }
    }

    let http = PermanentHttp {
        calls: Cell::new(0),
    };
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("nope".to_string());

    let err = run("--retry 3 --delay 0 send", &mut state, &http, &session).unwrap_err();

    assert!(err.to_string().contains("invalid URL"), "{err}");
    assert_eq!(http.calls.get(), 1, "permanent errors must not be retried");
}

#[test]
fn until_condition_sees_candidate_request() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();
    state.request.url = Some("https://example.com".to_string());

    let (cmds, flags) = parse_args(&[
        s("--until"),
        s("request.url == https://example.com"),
        s("--delay"),
        s("0"),
        s("send"),
    ])
    .unwrap();
    run_commands(
        cmds,
        &flags,
        &mut state,
        &http,
        &session,
        &MockPresets::default(),
    )
    .unwrap();

    // The in-flight request is visible to the condition, so it passes on the
    // first attempt instead of polling the whole budget.
    assert_eq!(http.seen.borrow().len(), 1);
}

// --- chain failure persistence ---

#[test]
fn mutations_persist_when_chain_fails() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut state = State::default();

    // `expect status` fails (no previous response) after `url` mutated state.
    let err = run(
        "url https://example.com expect status",
        &mut state,
        &http,
        &session,
    )
    .unwrap_err();

    assert!(err.to_string().contains("no previous response"), "{err}");
    let saved = session.last_saved.borrow();
    let saved = saved.as_ref().expect("mutations should be saved on error");
    assert_eq!(saved.request.url, Some("https://example.com".to_string()));
}

// --- load keeps the cookie jar ---

#[test]
fn load_preserves_cookie_jar() {
    let http = MockHttp::ok(200);
    let session = MockSession::default();
    let mut preset = State::default();
    preset.request.url = Some("https://example.com".to_string());
    let presets = MockPresets::with("p.json", preset);

    let mut state = State::default();
    state
        .cookies
        .push(jar_cookie("session", "abc", "example.com"));

    run_with_presets("load p.json", &mut state, &http, &session, &presets).unwrap();

    assert_eq!(state.request.url, Some("https://example.com".to_string()));
    assert_eq!(state.cookies.len(), 1, "load must not clear the cookie jar");
}

// --- format_curl method quoting ---

#[test]
fn format_curl_quotes_unusual_method() {
    let request = Request {
        method: Some("GET;X".to_string()),
        url: Some("https://example.com".to_string()),
        ..Default::default()
    };
    let cmd = display::format_curl(&request, false).unwrap();
    assert!(cmd.contains("-X 'GET;X'"), "{cmd}");
}

// --- format_size ---

#[test]
fn format_size_scales_units() {
    assert_eq!(display::format_size(0), "0 B");
    assert_eq!(display::format_size(512), "512 B");
    assert_eq!(display::format_size(4200), "4.1 KiB");
    assert_eq!(display::format_size(5 * 1024 * 1024), "5.0 MiB");
    assert_eq!(display::format_size(3 * 1024 * 1024 * 1024), "3.0 GiB");
}
