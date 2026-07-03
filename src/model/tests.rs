use super::*;

#[test]
fn state_default_is_empty() {
    let s = State::default();
    assert!(s.method.is_none());
    assert!(s.url.is_none());
    assert!(s.headers.is_empty());
    assert!(s.body.is_none());
    assert!(s.responses.is_empty());
}

#[test]
fn state_round_trips_through_json() {
    let mut s = State::default();
    s.method = Some("POST".to_string());
    s.url = Some("https://example.com".to_string());
    s.headers.insert("X-Foo".to_string(), "bar".to_string());
    s.body = Some(r#"{"key":"val"}"#.to_string());

    let json = serde_json::to_string(&s).unwrap();
    let restored: State = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.method, s.method);
    assert_eq!(restored.url, s.url);
    assert_eq!(restored.headers["X-Foo"], "bar");
    assert_eq!(restored.body, s.body);
    assert!(restored.responses.is_empty());
}

#[test]
fn state_deserializes_native_json_object_body() {
    let json = r#"{"url":"https://x.com","body":{"field":"value","nested":{"n":1}}}"#;
    let s: State = serde_json::from_str(json).unwrap();
    assert_eq!(
        s.body.as_deref(),
        Some(r#"{"field":"value","nested":{"n":1}}"#)
    );
}

#[test]
fn state_deserializes_native_json_array_body() {
    let json = r#"{"url":"https://x.com","body":[1,2,3]}"#;
    let s: State = serde_json::from_str(json).unwrap();
    assert_eq!(s.body.as_deref(), Some("[1,2,3]"));
}

#[test]
fn state_deserializes_plain_string_body() {
    let json = r#"{"url":"https://x.com","body":"hello world"}"#;
    let s: State = serde_json::from_str(json).unwrap();
    assert_eq!(s.body.as_deref(), Some("hello world"));
}

#[test]
fn state_deserializes_escaped_json_string_body() {
    let json = r#"{"url":"https://x.com","body":"{\"field\":\"value\"}"}"#;
    let s: State = serde_json::from_str(json).unwrap();
    assert_eq!(s.body.as_deref(), Some(r#"{"field":"value"}"#));
}

#[test]
fn state_serializes_json_body_as_native_object() {
    let mut s = State::default();
    s.body = Some(r#"{"key":"val"}"#.to_string());
    let json = serde_json::to_string(&s).unwrap();
    assert!(json.contains(r#""body":{"key":"val"}"#));
}

#[test]
fn state_serializes_non_json_body_as_string() {
    let mut s = State::default();
    s.body = Some("plain text".to_string());
    let json = serde_json::to_string(&s).unwrap();
    assert!(json.contains(r#""body":"plain text""#));
}

#[test]
fn state_serializes_json_array_body_as_native_array() {
    let mut s = State::default();
    s.body = Some("[1,2,3]".to_string());
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
    assert!(s.headers.is_empty());
}

#[test]
fn state_old_last_field_is_ignored() {
    let json = r#"{"method":"GET","url":"https://x.com","headers":{},"last":{"status":200,"headers":{},"body":"ok"}}"#;
    let s: State = serde_json::from_str(json).unwrap();
    assert!(s.responses.is_empty());
}

#[test]
fn response_record_round_trips() {
    let mut headers = HashMap::new();
    headers.insert("content-type".to_string(), "application/json".to_string());
    let r = ResponseRecord {
        source: Some("login.json".to_string()),
        status: 200,
        headers,
        body: "ok".to_string(),
    };
    let json = serde_json::to_string(&r).unwrap();
    let restored: ResponseRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.status, 200);
    assert_eq!(restored.body, "ok");
    assert_eq!(restored.source.as_deref(), Some("login.json"));
    assert_eq!(restored.headers["content-type"], "application/json");
}

#[test]
fn response_record_without_source_omits_field() {
    let r = ResponseRecord {
        source: None,
        status: 404,
        headers: HashMap::new(),
        body: "not found".to_string(),
    };
    let json = serde_json::to_string(&r).unwrap();
    assert!(!json.contains("source"));
    let restored: ResponseRecord = serde_json::from_str(&json).unwrap();
    assert!(restored.source.is_none());
    assert_eq!(restored.status, 404);
}
