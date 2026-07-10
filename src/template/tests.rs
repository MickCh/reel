use super::*;

fn record(status: u16, body: &str, headers: &[(&str, &str)]) -> ResponseRecord {
    ResponseRecord {
        status,
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        body: body.to_string(),
        ..Default::default()
    }
}

fn request(method: &str, url: &str, body: Option<&str>, headers: &[(&str, &str)]) -> Request {
    Request {
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
    let mut request = Request {
        url: Some("https://example.com/${{ body.tok }}".to_string()),
        headers: [("X-Token".to_string(), "${{ body.tok }}".to_string())]
            .into_iter()
            .collect(),
        ..Default::default()
    };
    apply_interpolation(
        &mut request,
        &Context {
            requests: &[],
            responses: &[r],
        },
    )
    .unwrap();
    assert_eq!(request.url.unwrap(), "https://example.com/xyz");
    assert_eq!(request.headers.get("X-Token"), Some("xyz"));
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

fn interp_full(text: &str, requests: &[Request], responses: &[ResponseRecord]) -> Result<String> {
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

// --- check_condition ---

#[test]
fn condition_existence_passes_when_resolvable() {
    let responses = [record(200, r#"{"token":"abc"}"#, &[])];
    let ctx = Context {
        requests: &[],
        responses: &responses,
    };
    assert!(check_condition("body.token", &ctx).is_ok());
    assert!(check_condition("status", &ctx).is_ok());
}

#[test]
fn condition_existence_fails_when_missing() {
    let responses = [record(200, r#"{"token":"abc"}"#, &[])];
    let ctx = Context {
        requests: &[],
        responses: &responses,
    };
    assert!(check_condition("body.missing", &ctx).is_err());
}

#[test]
fn condition_equality() {
    let responses = [record(201, r#"{"id":42}"#, &[])];
    let ctx = Context {
        requests: &[],
        responses: &responses,
    };
    assert!(check_condition("status == 201", &ctx).is_ok());
    assert!(check_condition("body.id == 42", &ctx).is_ok());
    let err = check_condition("status == 200", &ctx)
        .unwrap_err()
        .to_string();
    assert!(err.contains("actual: 201"), "{err}");
}

#[test]
fn condition_inequality() {
    let responses = [record(200, "{}", &[])];
    let ctx = Context {
        requests: &[],
        responses: &responses,
    };
    assert!(check_condition("status != 404", &ctx).is_ok());
    assert!(check_condition("status != 200", &ctx).is_err());
}

#[test]
fn condition_contains() {
    let responses = [record(200, r#"{"msg":"hello world"}"#, &[])];
    let ctx = Context {
        requests: &[],
        responses: &responses,
    };
    assert!(check_condition("body contains hello", &ctx).is_ok());
    assert!(check_condition("body.msg contains hello world", &ctx).is_ok());
    assert!(check_condition("body contains goodbye", &ctx).is_err());
}

#[test]
fn condition_unknown_operator_is_error() {
    let responses = [record(200, "{}", &[])];
    let ctx = Context {
        requests: &[],
        responses: &responses,
    };
    let err = check_condition("status >= 200", &ctx)
        .unwrap_err()
        .to_string();
    assert!(err.contains("unknown expect operator"), "{err}");
}

#[test]
fn condition_without_history_is_error() {
    let ctx = Context {
        requests: &[],
        responses: &[],
    };
    assert!(check_condition("status == 200", &ctx).is_err());
}

#[test]
fn condition_empty_is_error() {
    let ctx = Context {
        requests: &[],
        responses: &[],
    };
    assert!(check_condition("  ", &ctx).is_err());
}

// --- template functions ---

fn empty_ctx() -> Context<'static> {
    Context {
        requests: &[],
        responses: &[],
    }
}

#[test]
fn uuid_function_generates_v4() {
    let out = interpolate("${{ uuid() }}", &empty_ctx()).unwrap();
    assert_eq!(out.len(), 36);
    assert_eq!(out.as_bytes()[14], b'4');
    for i in [8, 13, 18, 23] {
        assert_eq!(out.as_bytes()[i], b'-', "{out}");
    }
    // Each placeholder is evaluated independently.
    let two = interpolate("${{ uuid() }} ${{ uuid() }}", &empty_ctx()).unwrap();
    let (a, b) = two.split_once(' ').unwrap();
    assert_ne!(a, b);
}

#[test]
fn uuid_function_rejects_arguments() {
    assert!(interpolate("${{ uuid(7) }}", &empty_ctx()).is_err());
}

#[test]
fn now_function_returns_unix_seconds() {
    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let out: i64 = interpolate("${{ now() }}", &empty_ctx())
        .unwrap()
        .parse()
        .unwrap();
    assert!((out - before).abs() <= 2, "now()={out} vs {before}");
}

#[test]
fn now_function_applies_offset() {
    let base: i64 = interpolate("${{ now() }}", &empty_ctx())
        .unwrap()
        .parse()
        .unwrap();
    let plus: i64 = interpolate("${{ now(+3600) }}", &empty_ctx())
        .unwrap()
        .parse()
        .unwrap();
    let minus: i64 = interpolate("${{ now(-60) }}", &empty_ctx())
        .unwrap()
        .parse()
        .unwrap();
    assert!((plus - base - 3600).abs() <= 2);
    assert!((minus - base + 60).abs() <= 2);
}

#[test]
fn now_function_invalid_offset_is_error() {
    assert!(interpolate("${{ now(soon) }}", &empty_ctx()).is_err());
}

#[test]
fn base64_function_encodes_literal() {
    assert_eq!(
        interpolate("${{ base64('hello') }}", &empty_ctx()).unwrap(),
        "aGVsbG8="
    );
    assert_eq!(
        interpolate("${{ base64('user:pass') }}", &empty_ctx()).unwrap(),
        "dXNlcjpwYXNz"
    );
    // Padding edge cases.
    assert_eq!(
        interpolate("${{ base64('a') }}", &empty_ctx()).unwrap(),
        "YQ=="
    );
    assert_eq!(
        interpolate("${{ base64('ab') }}", &empty_ctx()).unwrap(),
        "YWI="
    );
    assert_eq!(
        interpolate("${{ base64('abc') }}", &empty_ctx()).unwrap(),
        "YWJj"
    );
}

#[test]
fn base64_function_evaluates_nested_expression() {
    let responses = [record(200, r#"{"token":"abc"}"#, &[])];
    let ctx = Context {
        requests: &[],
        responses: &responses,
    };
    assert_eq!(
        interpolate("${{ base64(body.token) }}", &ctx).unwrap(),
        "YWJj"
    );
}

#[test]
fn base64_function_without_argument_is_error() {
    assert!(interpolate("${{ base64() }}", &empty_ctx()).is_err());
}

#[test]
fn unknown_function_is_error() {
    let err = interpolate("${{ rot13('x') }}", &empty_ctx())
        .unwrap_err()
        .to_string();
    assert!(err.contains("unknown template function"), "{err}");
}

// --- default values ---

#[test]
fn default_used_when_env_var_missing() {
    assert_eq!(
        interpolate(
            "${{ env.REEL_TEST_SURELY_UNSET | default: localhost:8080 }}",
            &empty_ctx()
        )
        .unwrap(),
        "localhost:8080"
    );
}

#[test]
fn default_ignored_when_expression_resolves() {
    // Safe: test-only variable, no other test reads it.
    unsafe { std::env::set_var("REEL_TEST_DEFAULT_SET", "real") };
    assert_eq!(
        interpolate(
            "${{ env.REEL_TEST_DEFAULT_SET | default: fallback }}",
            &empty_ctx()
        )
        .unwrap(),
        "real"
    );
}

#[test]
fn default_used_when_history_missing() {
    assert_eq!(
        interpolate("${{ body.token | default: anonymous }}", &empty_ctx()).unwrap(),
        "anonymous"
    );
}

#[test]
fn default_value_may_contain_spaces_and_be_empty() {
    assert_eq!(
        interpolate(
            "${{ env.REEL_TEST_SURELY_UNSET | default: a b c }}",
            &empty_ctx()
        )
        .unwrap(),
        "a b c"
    );
    assert_eq!(
        interpolate("${{ env.REEL_TEST_SURELY_UNSET | default: }}", &empty_ctx()).unwrap(),
        ""
    );
}

#[test]
fn default_does_not_mask_unknown_function() {
    // A misspelled function is a structural error, not missing data — it must
    // propagate even with a default present.
    let err = interpolate("${{ uuidd() | default: x }}", &empty_ctx())
        .unwrap_err()
        .to_string();
    assert!(err.contains("unknown template function"), "{err}");
}

#[test]
fn default_does_not_mask_unknown_expression() {
    let responses = [record(200, "{}", &[])];
    let ctx = Context {
        requests: &[],
        responses: &responses,
    };
    let err = interpolate("${{ statas | default: x }}", &ctx)
        .unwrap_err()
        .to_string();
    assert!(err.contains("unknown template expression"), "{err}");

    let rq = request("GET", "https://x", None, &[]);
    let err = interp_full("${{ request.methud | default: x }}", &[rq], &[])
        .unwrap_err()
        .to_string();
    assert!(err.contains("unknown request expression"), "{err}");
}

#[test]
fn default_applies_to_unresolvable_inside_function() {
    // The Unresolvable marker survives the nested evaluation in base64(...).
    assert_eq!(
        interpolate(
            "${{ base64(env.REEL_TEST_SURELY_UNSET) | default: none }}",
            &empty_ctx()
        )
        .unwrap(),
        "none"
    );
}

#[test]
fn pipe_without_default_keyword_is_error() {
    let err = interpolate("${{ status | fallback: x }}", &empty_ctx())
        .unwrap_err()
        .to_string();
    assert!(err.contains("default:"), "{err}");
}

#[test]
fn pipe_inside_quoted_literal_is_not_a_default() {
    assert_eq!(
        interpolate("${{ base64('a|b') }}", &empty_ctx()).unwrap(),
        "YXxi"
    );
}

// --- elapsed ---

#[test]
fn elapsed_expression_resolves() {
    let mut r = record(200, "{}", &[]);
    r.elapsed_ms = 142;
    let responses = [record(200, "{}", &[]), r];
    let ctx = Context {
        requests: &[],
        responses: &responses,
    };
    assert_eq!(interpolate("${{ elapsed }}", &ctx).unwrap(), "142");
    assert_eq!(
        interpolate("${{ response[1].elapsed }}", &ctx).unwrap(),
        "0"
    );
    assert!(check_condition("elapsed == 142", &ctx).is_ok());
}
