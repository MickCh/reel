use super::*;
use crate::model::RequestRecord;
use std::collections::HashMap;

fn record(status: u16, body: &str, headers: &[(&str, &str)]) -> ResponseRecord {
    ResponseRecord {
        source: None,
        status,
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body: body.to_string(),
    }
}

fn request(method: &str, url: &str, body: Option<&str>, headers: &[(&str, &str)]) -> RequestRecord {
    RequestRecord {
        method: Some(method.to_string()),
        url: Some(url.to_string()),
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body: body.map(str::to_string),
    }
}

// Interpolate against a response-only history (no requests).
fn interp(text: &str, responses: &[ResponseRecord]) -> Result<String> {
    interpolate(
        text,
        &Context {
            requests: &[],
            responses,
        },
    )
}

#[test]
fn no_placeholders() {
    let r = record(200, "hello", &[]);
    assert_eq!(interp("plain text", &[r]).unwrap(), "plain text");
}

#[test]
fn status_placeholder() {
    let r = record(404, "", &[]);
    assert_eq!(interp("code=${{ status }}", &[r]).unwrap(), "code=404");
}

#[test]
fn body_placeholder() {
    let r = record(200, "raw body", &[]);
    assert_eq!(interp("x=${{ body }}", &[r]).unwrap(), "x=raw body");
}

#[test]
fn header_placeholder() {
    let r = record(200, "", &[("content-type", "application/json")]);
    assert_eq!(
        interp("ct=${{ headers.content-type }}", &[r]).unwrap(),
        "ct=application/json"
    );
}

#[test]
fn header_missing_is_error() {
    let r = record(200, "", &[]);
    assert!(interp("${{ headers.x-missing }}", &[r]).is_err());
}

#[test]
fn body_dot_path() {
    let r = record(200, r#"{"token":"abc123"}"#, &[]);
    assert_eq!(interp("${{ body.token }}", &[r]).unwrap(), "abc123");
}

#[test]
fn body_nested_path() {
    let r = record(200, r#"{"access":{"token":"t42"}}"#, &[]);
    assert_eq!(interp("${{ body.access.token }}", &[r]).unwrap(), "t42");
}

#[test]
fn body_array_index() {
    let r = record(200, r#"{"items":[{"id":"first"},{"id":"second"}]}"#, &[]);
    assert_eq!(interp("${{ body.items.1.id }}", &[r]).unwrap(), "second");
}

#[test]
fn body_numeric_value_stringified() {
    let r = record(200, r#"{"count":42}"#, &[]);
    assert_eq!(interp("${{ body.count }}", &[r]).unwrap(), "42");
}

#[test]
fn body_path_missing_is_error() {
    let r = record(200, r#"{"a":1}"#, &[]);
    assert!(interp("${{ body.b }}", &[r]).is_err());
}

#[test]
fn body_not_json_with_path_is_error() {
    let r = record(200, "not json", &[]);
    assert!(interp("${{ body.field }}", &[r]).is_err());
}

#[test]
fn unclosed_placeholder_is_error() {
    let r = record(200, "", &[]);
    assert!(interp("${{ status", &[r]).is_err());
}

#[test]
fn unknown_expression_is_error() {
    let r = record(200, "", &[]);
    assert!(interp("${{ unknown }}", &[r]).is_err());
}

#[test]
fn multiple_placeholders() {
    let r = record(201, r#"{"id":"99"}"#, &[]);
    assert_eq!(
        interp("status=${{ status }} id=${{ body.id }}", &[r]).unwrap(),
        "status=201 id=99"
    );
}

#[test]
fn apply_interpolation_replaces_url_and_headers() {
    let r = record(200, r#"{"tok":"xyz"}"#, &[]);
    let mut state = State {
        url: Some("https://example.com/${{ body.tok }}".to_string()),
        headers: HashMap::from([("X-Token".to_string(), "${{ body.tok }}".to_string())]),
        ..Default::default()
    };
    apply_interpolation(
        &mut state,
        &Context {
            requests: &[],
            responses: &[r],
        },
    )
    .unwrap();
    assert_eq!(state.url.unwrap(), "https://example.com/xyz");
    assert_eq!(state.headers["X-Token"], "xyz");
}

// --- indexed response[N] syntax ---

#[test]
fn indexed_response_body_path() {
    let r1 = record(200, r#"{"token":"first"}"#, &[]);
    let r2 = record(200, r#"{"token":"second"}"#, &[]);
    assert_eq!(
        interp("${{ response[1].body.token }}", &[r1, r2]).unwrap(),
        "first"
    );
}

#[test]
fn indexed_response_last_implicit() {
    let r1 = record(200, r#"{"token":"first"}"#, &[]);
    let r2 = record(201, r#"{"token":"second"}"#, &[]);
    // bare ${{ body.token }} still refers to the last response
    assert_eq!(interp("${{ body.token }}", &[r1, r2]).unwrap(), "second");
}

#[test]
fn indexed_response_status() {
    let r1 = record(201, "", &[]);
    let r2 = record(200, "", &[]);
    let responses = vec![r1, r2];
    assert_eq!(
        interp("${{ response[1].status }}", &responses).unwrap(),
        "201"
    );
    assert_eq!(
        interp("${{ response[2].status }}", &responses).unwrap(),
        "200"
    );
}

#[test]
fn indexed_response_header() {
    let r1 = record(200, "", &[("x-token", "tok1")]);
    let r2 = record(200, "", &[("x-token", "tok2")]);
    assert_eq!(
        interp("${{ response[1].headers.x-token }}", &[r1, r2]).unwrap(),
        "tok1"
    );
}

#[test]
fn indexed_response_out_of_range_is_error() {
    let r1 = record(200, "", &[]);
    assert!(interp("${{ response[2].body }}", &[r1]).is_err());
}

#[test]
fn indexed_response_zero_is_error() {
    let r1 = record(200, "", &[]);
    assert!(interp("${{ response[0].body }}", &[r1]).is_err());
}

#[test]
fn indexed_response_no_field_is_error() {
    let r1 = record(200, "", &[]);
    assert!(interp("${{ response[1] }}", &[r1]).is_err());
}

#[test]
fn mix_indexed_and_implicit() {
    let r1 = record(200, r#"{"id":"first"}"#, &[]);
    let r2 = record(201, r#"{"id":"second"}"#, &[]);
    assert_eq!(
        interp(
            "from1=${{ response[1].body.id }} from2=${{ body.id }}",
            &[r1, r2]
        )
        .unwrap(),
        "from1=first from2=second"
    );
}

// --- environment variables ---

fn interp_full(
    text: &str,
    requests: &[RequestRecord],
    responses: &[ResponseRecord],
) -> Result<String> {
    interpolate(
        text,
        &Context {
            requests,
            responses,
        },
    )
}

#[test]
fn env_var_resolves() {
    // SAFETY: single-threaded test; no other thread reads the environment here.
    unsafe { std::env::set_var("REEL_TEST_TOKEN", "s3cret") };
    assert_eq!(
        interp_full("Bearer ${{ env.REEL_TEST_TOKEN }}", &[], &[]).unwrap(),
        "Bearer s3cret"
    );
}

#[test]
fn env_var_works_without_history() {
    unsafe { std::env::set_var("REEL_TEST_HOST", "example.com") };
    // No requests and no responses at all — env must still resolve.
    assert_eq!(
        interp_full("https://${{ env.REEL_TEST_HOST }}/x", &[], &[]).unwrap(),
        "https://example.com/x"
    );
}

#[test]
fn env_var_missing_is_error() {
    assert!(interp_full("${{ env.REEL_DEFINITELY_UNSET_VAR }}", &[], &[]).is_err());
}

// --- request history ---

#[test]
fn request_url_and_method() {
    let rq = request("POST", "https://api.example.com/login", None, &[]);
    assert_eq!(
        interp_full("${{ request.method }} ${{ request.url }}", &[rq], &[]).unwrap(),
        "POST https://api.example.com/login"
    );
}

#[test]
fn request_body_dot_path() {
    let rq = request("POST", "https://x", Some(r#"{"user":{"id":"u7"}}"#), &[]);
    assert_eq!(
        interp_full("${{ request.body.user.id }}", &[rq], &[]).unwrap(),
        "u7"
    );
}

#[test]
fn request_header_case_insensitive() {
    let rq = request("GET", "https://x", None, &[("X-Trace-Id", "abc")]);
    assert_eq!(
        interp_full("${{ request.headers.x-trace-id }}", &[rq], &[]).unwrap(),
        "abc"
    );
}

#[test]
fn request_indexed() {
    let rq1 = request("GET", "https://one", None, &[]);
    let rq2 = request("GET", "https://two", None, &[]);
    assert_eq!(
        interp_full("${{ request[1].url }}", &[rq1, rq2], &[]).unwrap(),
        "https://one"
    );
}

#[test]
fn request_last_implicit() {
    let rq1 = request("GET", "https://one", None, &[]);
    let rq2 = request("GET", "https://two", None, &[]);
    // bare request.* refers to the most recent request
    assert_eq!(
        interp_full("${{ request.url }}", &[rq1, rq2], &[]).unwrap(),
        "https://two"
    );
}

#[test]
fn request_with_no_history_is_error() {
    assert!(interp_full("${{ request.url }}", &[], &[]).is_err());
}

#[test]
fn request_unknown_field_is_error() {
    let rq = request("GET", "https://x", None, &[]);
    assert!(interp_full("${{ request.bogus }}", &[rq], &[]).is_err());
}
