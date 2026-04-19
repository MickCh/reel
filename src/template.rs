use crate::model::{LastResponse, State};

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

// Evaluate a single ${{ expr }} against the last response.
//
// Supported forms:
//   status              → HTTP status code as a string
//   body                → raw response body
//   body.<path>         → dot-path into the JSON body (e.g. body.access.token)
//   headers.<name>      → response header value (e.g. headers.content-type)
fn eval_expr(expr: &str, last: &LastResponse) -> Result<String, String> {
    if expr == "status" {
        return Ok(last.status.to_string());
    }
    if expr == "body" {
        return Ok(last.body.clone());
    }
    if let Some(path) = expr.strip_prefix("headers.") {
        return last
            .headers
            .get(path)
            .cloned()
            .ok_or_else(|| format!("header '{}' not found in last response", path));
    }
    if let Some(path) = expr.strip_prefix("body.") {
        let json: serde_json::Value = serde_json::from_str(&last.body)
            .map_err(|e| format!("response body is not valid JSON: {}", e))?;
        return traverse_json(&json, path)
            .ok_or_else(|| format!("path '{}' not found in response body", path));
    }
    Err(format!("unknown template expression '${{{{ {} }}}}'", expr))
}

// Replace all ${{ expr }} placeholders in `text` using values from `last`.
pub fn interpolate(text: &str, last: &LastResponse) -> Result<String, String> {
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
        result.push_str(&eval_expr(expr, last)?);
    }
    result.push_str(remaining);
    Ok(result)
}

// Apply interpolation to all string fields of state (url, body, header values).
pub fn apply_interpolation(state: &mut State, last: &LastResponse) -> Result<(), String> {
    if let Some(url) = &state.url.clone() {
        state.url = Some(interpolate(url, last)?);
    }
    if let Some(body) = &state.body.clone() {
        state.body = Some(interpolate(body, last)?);
    }
    let keys: Vec<String> = state.headers.keys().cloned().collect();
    for key in keys {
        let val = state.headers[&key].clone();
        state.headers.insert(key, interpolate(&val, last)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn last(status: u16, body: &str, headers: &[(&str, &str)]) -> LastResponse {
        LastResponse {
            status,
            body: body.to_string(),
            headers: headers.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
        }
    }

    #[test]
    fn no_placeholders() {
        let l = last(200, "hello", &[]);
        assert_eq!(interpolate("plain text", &l).unwrap(), "plain text");
    }

    #[test]
    fn status_placeholder() {
        let l = last(404, "", &[]);
        assert_eq!(interpolate("code=${{ status }}", &l).unwrap(), "code=404");
    }

    #[test]
    fn body_placeholder() {
        let l = last(200, "raw body", &[]);
        assert_eq!(interpolate("x=${{ body }}", &l).unwrap(), "x=raw body");
    }

    #[test]
    fn header_placeholder() {
        let l = last(200, "", &[("content-type", "application/json")]);
        assert_eq!(
            interpolate("ct=${{ headers.content-type }}", &l).unwrap(),
            "ct=application/json"
        );
    }

    #[test]
    fn header_missing_is_error() {
        let l = last(200, "", &[]);
        assert!(interpolate("${{ headers.x-missing }}", &l).is_err());
    }

    #[test]
    fn body_dot_path() {
        let l = last(200, r#"{"token":"abc123"}"#, &[]);
        assert_eq!(interpolate("${{ body.token }}", &l).unwrap(), "abc123");
    }

    #[test]
    fn body_nested_path() {
        let l = last(200, r#"{"access":{"token":"t42"}}"#, &[]);
        assert_eq!(interpolate("${{ body.access.token }}", &l).unwrap(), "t42");
    }

    #[test]
    fn body_array_index() {
        let l = last(200, r#"{"items":[{"id":"first"},{"id":"second"}]}"#, &[]);
        assert_eq!(interpolate("${{ body.items.1.id }}", &l).unwrap(), "second");
    }

    #[test]
    fn body_numeric_value_stringified() {
        let l = last(200, r#"{"count":42}"#, &[]);
        assert_eq!(interpolate("${{ body.count }}", &l).unwrap(), "42");
    }

    #[test]
    fn body_path_missing_is_error() {
        let l = last(200, r#"{"a":1}"#, &[]);
        assert!(interpolate("${{ body.b }}", &l).is_err());
    }

    #[test]
    fn body_not_json_with_path_is_error() {
        let l = last(200, "not json", &[]);
        assert!(interpolate("${{ body.field }}", &l).is_err());
    }

    #[test]
    fn unclosed_placeholder_is_error() {
        let l = last(200, "", &[]);
        assert!(interpolate("${{ status", &l).is_err());
    }

    #[test]
    fn unknown_expression_is_error() {
        let l = last(200, "", &[]);
        assert!(interpolate("${{ unknown }}", &l).is_err());
    }

    #[test]
    fn multiple_placeholders() {
        let l = last(201, r#"{"id":"99"}"#, &[]);
        assert_eq!(
            interpolate("status=${{ status }} id=${{ body.id }}", &l).unwrap(),
            "status=201 id=99"
        );
    }

    #[test]
    fn apply_interpolation_replaces_url_and_headers() {
        let l = last(200, r#"{"tok":"xyz"}"#, &[]);
        let mut state = State {
            url: Some("https://example.com/${{ body.tok }}".to_string()),
            headers: HashMap::from([("X-Token".to_string(), "${{ body.tok }}".to_string())]),
            ..Default::default()
        };
        apply_interpolation(&mut state, &l).unwrap();
        assert_eq!(state.url.unwrap(), "https://example.com/xyz");
        assert_eq!(state.headers["X-Token"], "xyz");
    }
}
