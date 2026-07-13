use super::cookies::SetCookie;
use super::*;

// --- Headers ---

#[test]
fn headers_get_is_case_insensitive() {
    let mut h = Headers::default();
    h.insert("Content-Type".to_string(), "application/json".to_string());
    assert_eq!(h.get("content-type"), Some("application/json"));
    assert_eq!(h.get("CONTENT-TYPE"), Some("application/json"));
    assert_eq!(h.get("x-missing"), None);
}

#[test]
fn headers_insert_dedups_case_insensitively() {
    let mut h = Headers::default();
    h.insert("X-Token".to_string(), "old".to_string());
    h.insert("x-token".to_string(), "new".to_string());
    assert_eq!(h.iter().count(), 1);
    // The most recent casing wins.
    assert_eq!(h.sorted()[0].0, "x-token");
    assert_eq!(h.get("X-Token"), Some("new"));
}

#[test]
fn headers_remove_is_case_insensitive() {
    let mut h = Headers::default();
    h.insert("X-Foo".to_string(), "bar".to_string());
    assert!(h.remove("x-foo"));
    assert!(h.is_empty());
    assert!(!h.remove("x-foo"));
}

#[test]
fn headers_sorted_orders_by_name() {
    let mut h = Headers::default();
    h.insert("b".to_string(), "2".to_string());
    h.insert("a".to_string(), "1".to_string());
    let names: Vec<&str> = h.sorted().into_iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(names, ["a", "b"]);
}

// --- State ---

#[test]
fn state_default_is_empty() {
    let s = State::default();
    assert!(s.request.method.is_none());
    assert!(s.request.url.is_none());
    assert!(s.request.headers.is_empty());
    assert!(s.request.body.is_none());
    assert!(s.responses.is_empty());
}

#[test]
fn state_round_trips_through_json() {
    let mut s = State::default();
    s.request.method = Some("POST".to_string());
    s.request.url = Some("https://example.com".to_string());
    s.request
        .headers
        .insert("X-Foo".to_string(), "bar".to_string());
    s.request.body = Some(r#"{"key":"val"}"#.to_string());

    let json = serde_json::to_string(&s).unwrap();
    let restored: State = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.request.method, s.request.method);
    assert_eq!(restored.request.url, s.request.url);
    assert_eq!(restored.request.headers.get("X-Foo"), Some("bar"));
    assert_eq!(restored.request.body, s.request.body);
    assert!(restored.responses.is_empty());
}

#[test]
fn state_serializes_request_fields_flat() {
    let mut s = State::default();
    s.request.method = Some("GET".to_string());
    let json = serde_json::to_string(&s).unwrap();
    // The current request is flattened — no nested "request" object.
    assert!(json.contains(r#""method":"GET""#));
    assert!(!json.contains(r#""request""#));
}

#[test]
fn state_deserializes_native_json_object_body() {
    let json = r#"{"url":"https://x.com","body":{"field":"value","nested":{"n":1}}}"#;
    let s: State = serde_json::from_str(json).unwrap();
    assert_eq!(
        s.request.body.as_deref(),
        Some(r#"{"field":"value","nested":{"n":1}}"#)
    );
}

#[test]
fn state_deserializes_native_json_array_body() {
    let json = r#"{"url":"https://x.com","body":[1,2,3]}"#;
    let s: State = serde_json::from_str(json).unwrap();
    assert_eq!(s.request.body.as_deref(), Some("[1,2,3]"));
}

#[test]
fn state_deserializes_plain_string_body() {
    let json = r#"{"url":"https://x.com","body":"hello world"}"#;
    let s: State = serde_json::from_str(json).unwrap();
    assert_eq!(s.request.body.as_deref(), Some("hello world"));
}

fn body_round_trip(body: &str) -> Option<String> {
    let mut s = State::default();
    s.request.body = Some(body.to_string());
    let json = serde_json::to_string(&s).unwrap();
    let restored: State = serde_json::from_str(&json).unwrap();
    restored.request.body
}

#[test]
fn body_round_trips_byte_exact() {
    // Bodies whose native-JSON form would come back different must be stored
    // as plain strings: a JSON string literal (would lose its quotes), null
    // (would become no body), non-canonical key order, non-canonical number
    // notation, pretty-printed JSON, and plain text.
    for body in [
        r#""hello""#,
        "null",
        r#"{"b":1,"a":2}"#,
        "1e3",
        "{\n  \"a\": 1\n}",
        "plain text",
    ] {
        assert_eq!(body_round_trip(body).as_deref(), Some(body), "{body:?}");
    }
}

#[test]
fn body_canonical_json_is_stored_natively() {
    let mut s = State::default();
    s.request.body = Some(r#"{"a":2,"b":1}"#.to_string());
    let json = serde_json::to_string(&s).unwrap();
    assert!(
        json.contains(r#""body":{"a":2,"b":1}"#),
        "canonical JSON body should be embedded as a native value: {json}"
    );
    assert_eq!(
        body_round_trip(r#"{"a":2,"b":1}"#).as_deref(),
        Some(r#"{"a":2,"b":1}"#)
    );
}

#[test]
fn state_deserializes_escaped_json_string_body() {
    let json = r#"{"url":"https://x.com","body":"{\"field\":\"value\"}"}"#;
    let s: State = serde_json::from_str(json).unwrap();
    assert_eq!(s.request.body.as_deref(), Some(r#"{"field":"value"}"#));
}

#[test]
fn state_serializes_json_body_as_native_object() {
    let mut s = State::default();
    s.request.body = Some(r#"{"key":"val"}"#.to_string());
    let json = serde_json::to_string(&s).unwrap();
    assert!(json.contains(r#""body":{"key":"val"}"#));
}

#[test]
fn state_serializes_non_json_body_as_string() {
    let mut s = State::default();
    s.request.body = Some("plain text".to_string());
    let json = serde_json::to_string(&s).unwrap();
    assert!(json.contains(r#""body":"plain text""#));
}

#[test]
fn state_serializes_json_array_body_as_native_array() {
    let mut s = State::default();
    s.request.body = Some("[1,2,3]".to_string());
    let json = serde_json::to_string(&s).unwrap();
    assert!(json.contains(r#""body":[1,2,3]"#));
}

#[test]
fn state_omitted_responses_deserializes_as_empty() {
    let json = r#"{"method":"GET","url":"https://x.com","headers":{}}"#;
    let s: State = serde_json::from_str(json).unwrap();
    assert!(s.responses.is_empty());
}

#[test]
fn state_omitted_headers_deserializes_as_empty() {
    let json = r#"{"method":"GET","url":"https://x.com"}"#;
    let s: State = serde_json::from_str(json).unwrap();
    assert!(s.request.headers.is_empty());
}

#[test]
fn state_old_last_field_is_ignored() {
    let json = r#"{"method":"GET","url":"https://x.com","headers":{},"last":{"status":200,"headers":{},"body":"ok"}}"#;
    let s: State = serde_json::from_str(json).unwrap();
    assert!(s.responses.is_empty());
}

// --- ResponseRecord ---

#[test]
fn response_record_round_trips() {
    let mut headers = Headers::default();
    headers.insert("content-type".to_string(), "application/json".to_string());
    let r = ResponseRecord {
        source: Some("login.json".to_string()),
        status: 200,
        headers,
        body: "ok".to_string(),
        ..Default::default()
    };
    let json = serde_json::to_string(&r).unwrap();
    let restored: ResponseRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.status, 200);
    assert_eq!(restored.body, "ok");
    assert_eq!(restored.source.as_deref(), Some("login.json"));
    assert_eq!(
        restored.headers.get("content-type"),
        Some("application/json")
    );
}

#[test]
fn response_record_without_source_omits_field() {
    let r = ResponseRecord {
        status: 404,
        body: "not found".to_string(),
        ..Default::default()
    };
    let json = serde_json::to_string(&r).unwrap();
    assert!(!json.contains("source"));
    let restored: ResponseRecord = serde_json::from_str(&json).unwrap();
    assert!(restored.source.is_none());
    assert_eq!(restored.status, 404);
}

// --- status_reason ---

#[test]
fn status_reason_known_codes() {
    assert_eq!(status_reason(200), Some("OK"));
    assert_eq!(status_reason(404), Some("Not Found"));
    assert_eq!(status_reason(503), Some("Service Unavailable"));
}

#[test]
fn status_reason_unknown_code_is_none() {
    assert_eq!(status_reason(299), None);
    assert_eq!(status_reason(999), None);
}

// --- cookies ---

fn set(update: Option<SetCookie>) -> Cookie {
    match update {
        Some(SetCookie::Set(c)) => c,
        _ => panic!("expected SetCookie::Set"),
    }
}

#[test]
fn parse_set_cookie_basic() {
    let c = set(parse_set_cookie("session=abc123", "example.com", "/login"));
    assert_eq!(c.name, "session");
    assert_eq!(c.value, "abc123");
    assert_eq!(c.domain, "example.com");
    assert!(c.host_only);
    assert!(!c.secure);
}

#[test]
fn parse_set_cookie_default_path_is_request_directory() {
    let c = set(parse_set_cookie("a=1", "example.com", "/api/v1/login"));
    assert_eq!(c.path, "/api/v1");
    let c = set(parse_set_cookie("a=1", "example.com", "/login"));
    assert_eq!(c.path, "/");
    let c = set(parse_set_cookie("a=1", "example.com", "/"));
    assert_eq!(c.path, "/");
}

#[test]
fn parse_set_cookie_attributes() {
    let c = set(parse_set_cookie(
        "sid=x; Domain=.example.com; Path=/api; Secure; HttpOnly",
        "www.example.com",
        "/",
    ));
    assert_eq!(c.domain, "example.com");
    assert!(!c.host_only);
    assert_eq!(c.path, "/api");
    assert!(c.secure);
}

#[test]
fn parse_set_cookie_rejects_unrelated_domain() {
    assert!(parse_set_cookie("sid=x; Domain=evil.com", "example.com", "/").is_none());
    // Suffix without a dot boundary is not a subdomain.
    assert!(parse_set_cookie("sid=x; Domain=ample.com", "example.com", "/").is_none());
}

#[test]
fn parse_set_cookie_ip_host_rejects_suffix_domain() {
    // "0.0.1" is a dot-preceded suffix of "127.0.0.1", but suffix matching
    // must never apply to IP addresses (RFC 6265: host-only for IPs).
    assert!(parse_set_cookie("sid=x; Domain=0.0.1", "127.0.0.1", "/").is_none());
    assert!(parse_set_cookie("sid=x; Domain=:1", "[::1]", "/").is_none());
    // A Domain identical to the IP host is fine.
    let c = set(parse_set_cookie(
        "sid=x; Domain=127.0.0.1",
        "127.0.0.1",
        "/",
    ));
    assert_eq!(c.domain, "127.0.0.1");
}

#[test]
fn cookie_ip_host_matches_exactly_only() {
    // A jar entry with an IP-suffix domain (e.g. from an old session file)
    // must not match a longer IP via the suffix rule.
    let c = Cookie {
        name: "sid".to_string(),
        value: "x".to_string(),
        domain: "0.0.1".to_string(),
        path: "/".to_string(),
        secure: false,
        host_only: false,
    };
    assert!(!c.matches("127.0.0.1", "/", true));

    let exact = set(parse_set_cookie(
        "sid=x; Domain=127.0.0.1",
        "127.0.0.1",
        "/",
    ));
    assert!(exact.matches("127.0.0.1", "/", true));
    assert!(!exact.matches("127.0.0.2", "/", true));
}

#[test]
fn parse_set_cookie_max_age_zero_is_delete() {
    match parse_set_cookie("sid=; Path=/; Max-Age=0", "example.com", "/") {
        Some(SetCookie::Delete { name, domain, path }) => {
            assert_eq!(name, "sid");
            assert_eq!(domain, "example.com");
            assert_eq!(path, "/");
        }
        _ => panic!("expected SetCookie::Delete"),
    }
}

#[test]
fn parse_set_cookie_malformed_is_none() {
    assert!(parse_set_cookie("no-equals-sign", "example.com", "/").is_none());
    assert!(parse_set_cookie("=value-only", "example.com", "/").is_none());
}

#[test]
fn cookie_domain_matching() {
    let host_only = set(parse_set_cookie("a=1; Path=/", "example.com", "/"));
    assert!(host_only.matches("example.com", "/", true));
    assert!(!host_only.matches("www.example.com", "/", true));

    let domain_wide = set(parse_set_cookie(
        "a=1; Path=/; Domain=example.com",
        "example.com",
        "/",
    ));
    assert!(domain_wide.matches("example.com", "/", true));
    assert!(domain_wide.matches("api.example.com", "/", true));
    assert!(!domain_wide.matches("notexample.com", "/", true));
}

#[test]
fn cookie_path_matching() {
    let c = set(parse_set_cookie("a=1; Path=/api", "example.com", "/"));
    assert!(c.matches("example.com", "/api", true));
    assert!(c.matches("example.com", "/api/users", true));
    assert!(!c.matches("example.com", "/apiary", true));
    assert!(!c.matches("example.com", "/", true));
}

#[test]
fn secure_cookie_requires_https() {
    let c = set(parse_set_cookie("a=1; Path=/; Secure", "example.com", "/"));
    assert!(c.matches("example.com", "/", true));
    assert!(!c.matches("example.com", "/", false));
}

#[test]
fn update_jar_replaces_and_deletes() {
    let mut jar = Vec::new();
    update_jar(
        &mut jar,
        parse_set_cookie("sid=old; Path=/", "example.com", "/").unwrap(),
    );
    update_jar(
        &mut jar,
        parse_set_cookie("sid=new; Path=/", "example.com", "/").unwrap(),
    );
    assert_eq!(jar.len(), 1);
    assert_eq!(jar[0].value, "new");

    update_jar(
        &mut jar,
        parse_set_cookie("sid=; Path=/; Max-Age=0", "example.com", "/").unwrap(),
    );
    assert!(jar.is_empty());
}

#[test]
fn cookie_header_joins_matching_cookies() {
    let mut jar = Vec::new();
    update_jar(
        &mut jar,
        parse_set_cookie("b=2; Path=/", "example.com", "/").unwrap(),
    );
    update_jar(
        &mut jar,
        parse_set_cookie("a=1; Path=/api", "example.com", "/").unwrap(),
    );
    update_jar(
        &mut jar,
        parse_set_cookie("c=3; Path=/", "other.com", "/").unwrap(),
    );
    // Longest path first; the other.com cookie does not match.
    assert_eq!(
        cookie_header(&jar, "example.com", "/api/x", true).as_deref(),
        Some("a=1; b=2")
    );
    assert_eq!(
        cookie_header(&jar, "example.com", "/", true).as_deref(),
        Some("b=2")
    );
    assert_eq!(cookie_header(&jar, "unrelated.com", "/", true), None);
}

#[test]
fn state_with_cookies_round_trips() {
    let mut state = State::default();
    state.cookies.push(Cookie {
        name: "sid".to_string(),
        value: "abc".to_string(),
        domain: "example.com".to_string(),
        path: "/".to_string(),
        secure: true,
        host_only: true,
    });
    let json = serde_json::to_string(&state).unwrap();
    let restored: State = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.cookies, state.cookies);
}

#[test]
fn state_without_cookies_omits_field() {
    let state = State::default();
    let json = serde_json::to_string(&state).unwrap();
    assert!(!json.contains("cookies"));
}

#[test]
fn state_with_vars_round_trips() {
    let mut state = State::default();
    state.vars.insert("token".to_string(), "abc123".to_string());
    let json = serde_json::to_string(&state).unwrap();
    let restored: State = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.vars, state.vars);
}

#[test]
fn state_without_vars_omits_field() {
    // Session files written by older versions have no vars field — and files
    // written now must not grow one until a var is actually set.
    let state = State::default();
    let json = serde_json::to_string(&state).unwrap();
    assert!(!json.contains("vars"));
    let restored: State = serde_json::from_str("{}").unwrap();
    assert!(restored.vars.is_empty());
}

#[test]
fn response_record_without_elapsed_defaults_to_zero() {
    // Session files written by older versions have no elapsed_ms field.
    let restored: ResponseRecord =
        serde_json::from_str(r#"{"status":200,"headers":{},"body":"ok"}"#).unwrap();
    assert_eq!(restored.elapsed_ms, 0);
}
