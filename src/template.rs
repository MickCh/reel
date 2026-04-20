use crate::model::{ResponseRecord, State};

// Navigate a dot-separated path through a JSON value.
// Array indices are supported as numeric path segments (e.g. "items.0.id").
fn traverse_json(value: &serde_json::Value, path: &str) -> Option<String> {
    let mut current = value;
    for key in path.split('.') {
        current = match current {
            serde_json::Value::Object(map) => map.get(key)?,
            serde_json::Value::Array(arr) => {
                let idx: usize = key.parse().ok()?;
                arr.get(idx)?
            }
            _ => return None,
        };
    }
    Some(match current {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

// Evaluate a single ${{ expr }} against a response record.
//
// Supported forms:
//   status              → HTTP status code as a string
//   body                → raw response body
//   body.<path>         → dot-path into the JSON body (e.g. body.access.token)
//   headers.<name>      → response header value (e.g. headers.content-type)
fn eval_expr(expr: &str, record: &ResponseRecord) -> Result<String, String> {
    if expr == "status" {
        return Ok(record.status.to_string());
    }
    if expr == "body" {
        return Ok(record.body.clone());
    }
    if let Some(path) = expr.strip_prefix("headers.") {
        return record
            .headers
            .get(path)
            .cloned()
            .ok_or_else(|| format!("header '{}' not found in last response", path));
    }
    if let Some(path) = expr.strip_prefix("body.") {
        let json: serde_json::Value = serde_json::from_str(&record.body)
            .map_err(|e| format!("response body is not valid JSON: {}", e))?;
        return traverse_json(&json, path)
            .ok_or_else(|| format!("path '{}' not found in response body", path));
    }
    Err(format!("unknown template expression '${{{{ {} }}}}'", expr))
}

// Replace all ${{ expr }} placeholders in `text` using values from `record`.
pub fn interpolate(text: &str, record: &ResponseRecord) -> Result<String, String> {
    let mut result = String::new();
    let mut remaining = text;
    while let Some(start) = remaining.find("${{") {
        result.push_str(&remaining[..start]);
        remaining = &remaining[start + 3..];
        let end = remaining
            .find("}}")
            .ok_or_else(|| "unclosed '${{' in template".to_string())?;
        let expr = remaining[..end].trim();
        remaining = &remaining[end + 2..];
        result.push_str(&eval_expr(expr, record)?);
    }
    result.push_str(remaining);
    Ok(result)
}

// Apply interpolation to all string fields of state (url, body, header values).
pub fn apply_interpolation(state: &mut State, record: &ResponseRecord) -> Result<(), String> {
    if let Some(url) = state.url.take() {
        state.url = Some(interpolate(&url, record)?);
    }
    if let Some(body) = state.body.take() {
        state.body = Some(interpolate(&body, record)?);
    }
    let keys: Vec<String> = state.headers.keys().cloned().collect();
    for key in keys {
        let val = state.headers[&key].clone();
        state.headers.insert(key, interpolate(&val, record)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn record(status: u16, body: &str, headers: &[(&str, &str)]) -> ResponseRecord {
        ResponseRecord {
            source: None,
            status,
            body: body.to_string(),
            headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    #[test]
    fn no_placeholders() {
        let r = record(200, "hello", &[]);
        assert_eq!(interpolate("plain text", &r).unwrap(), "plain text");
    }

    #[test]
    fn status_placeholder() {
        let r = record(404, "", &[]);
        assert_eq!(interpolate("code=${{ status }}", &r).unwrap(), "code=404");
    }

    #[test]
    fn body_placeholder() {
        let r = record(200, "raw body", &[]);
        assert_eq!(interpolate("x=${{ body }}", &r).unwrap(), "x=raw body");
    }

    #[test]
    fn header_placeholder() {
        let r = record(200, "", &[("content-type", "application/json")]);
        assert_eq!(
            interpolate("ct=${{ headers.content-type }}", &r).unwrap(),
            "ct=application/json"
        );
    }

    #[test]
    fn header_missing_is_error() {
        let r = record(200, "", &[]);
        assert!(interpolate("${{ headers.x-missing }}", &r).is_err());
    }

    #[test]
    fn body_dot_path() {
        let r = record(200, r#"{"token":"abc123"}"#, &[]);
        assert_eq!(interpolate("${{ body.token }}", &r).unwrap(), "abc123");
    }

    #[test]
    fn body_nested_path() {
        let r = record(200, r#"{"access":{"token":"t42"}}"#, &[]);
        assert_eq!(interpolate("${{ body.access.token }}", &r).unwrap(), "t42");
    }

    #[test]
    fn body_array_index() {
        let r = record(200, r#"{"items":[{"id":"first"},{"id":"second"}]}"#, &[]);
        assert_eq!(interpolate("${{ body.items.1.id }}", &r).unwrap(), "second");
    }

    #[test]
    fn body_numeric_value_stringified() {
        let r = record(200, r#"{"count":42}"#, &[]);
        assert_eq!(interpolate("${{ body.count }}", &r).unwrap(), "42");
    }

    #[test]
    fn body_path_missing_is_error() {
        let r = record(200, r#"{"a":1}"#, &[]);
        assert!(interpolate("${{ body.b }}", &r).is_err());
    }

    #[test]
    fn body_not_json_with_path_is_error() {
        let r = record(200, "not json", &[]);
        assert!(interpolate("${{ body.field }}", &r).is_err());
    }

    #[test]
    fn unclosed_placeholder_is_error() {
        let r = record(200, "", &[]);
        assert!(interpolate("${{ status", &r).is_err());
    }

    #[test]
    fn unknown_expression_is_error() {
        let r = record(200, "", &[]);
        assert!(interpolate("${{ unknown }}", &r).is_err());
    }

    #[test]
    fn multiple_placeholders() {
        let r = record(201, r#"{"id":"99"}"#, &[]);
        assert_eq!(
            interpolate("status=${{ status }} id=${{ body.id }}", &r).unwrap(),
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
        apply_interpolation(&mut state, &r).unwrap();
        assert_eq!(state.url.unwrap(), "https://example.com/xyz");
        assert_eq!(state.headers["X-Token"], "xyz");
    }
}
