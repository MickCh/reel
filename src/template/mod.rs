use anyhow::{Result, bail};

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

// Evaluate a field expression against a specific response record.
//
// Supported forms:
//   status              → HTTP status code as a string
//   body                → raw response body
//   body.<path>         → dot-path into the JSON body (e.g. body.access.token)
//   headers.<name>      → response header value (e.g. headers.content-type)
fn eval_record_expr(expr: &str, record: &ResponseRecord) -> Result<String> {
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
            .ok_or_else(|| anyhow::anyhow!("header '{}' not found in response", path));
    }
    if let Some(path) = expr.strip_prefix("body.") {
        let json: serde_json::Value = serde_json::from_str(&record.body)
            .map_err(|e| anyhow::anyhow!("response body is not valid JSON: {}", e))?;
        return traverse_json(&json, path)
            .ok_or_else(|| anyhow::anyhow!("path '{}' not found in response body", path));
    }
    bail!("unknown template expression '${{{{ {} }}}}'", expr)
}

// Evaluate a single ${{ expr }} against the response history.
//
// Supported forms:
//   status                     → status of the last response
//   body                       → raw body of the last response
//   body.<path>                → dot-path into the last response body
//   headers.<name>             → header from the last response
//   response[N].status         → status of the Nth response (1-based)
//   response[N].body           → raw body of the Nth response
//   response[N].body.<path>    → dot-path into the Nth response body
//   response[N].headers.<name> → header from the Nth response
fn eval_expr(expr: &str, responses: &[ResponseRecord]) -> Result<String> {
    if let Some(rest) = expr.strip_prefix("response[")
        && let Some(bracket_end) = rest.find(']')
    {
        let idx_str = &rest[..bracket_end];
        let idx: usize = idx_str
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid response index 'response[{}]'", idx_str))?;
        if idx == 0 {
            bail!("response indices start at 1");
        }
        let record = responses.get(idx - 1).ok_or_else(|| {
            anyhow::anyhow!(
                "response[{}]: only {} response(s) available",
                idx,
                responses.len()
            )
        })?;
        let after_bracket = &rest[bracket_end + 1..];
        let field = after_bracket.strip_prefix('.').ok_or_else(|| {
            anyhow::anyhow!(
                "expected 'response[{}].<field>' (e.g. response[{}].body.token)",
                idx,
                idx
            )
        })?;
        return eval_record_expr(field, record);
    }

    let record = responses
        .last()
        .ok_or_else(|| anyhow::anyhow!("no previous response"))?;
    eval_record_expr(expr, record)
}

// Replace all ${{ expr }} placeholders in `text` using values from `responses`.
pub fn interpolate(text: &str, responses: &[ResponseRecord]) -> Result<String> {
    let mut result = String::new();
    let mut remaining = text;
    while let Some(start) = remaining.find("${{") {
        result.push_str(&remaining[..start]);
        remaining = &remaining[start + 3..];
        let end = remaining
            .find("}}")
            .ok_or_else(|| anyhow::anyhow!("unclosed '${{' in template"))?;
        let expr = remaining[..end].trim();
        remaining = &remaining[end + 2..];
        result.push_str(&eval_expr(expr, responses)?);
    }
    result.push_str(remaining);
    Ok(result)
}

// Apply interpolation to all string fields of state (url, body, header values).
pub fn apply_interpolation(state: &mut State, responses: &[ResponseRecord]) -> Result<()> {
    if let Some(url) = state.url.take() {
        state.url = Some(interpolate(&url, responses)?);
    }
    if let Some(body) = state.body.take() {
        state.body = Some(interpolate(&body, responses)?);
    }
    let keys: Vec<String> = state.headers.keys().cloned().collect();
    for key in keys {
        let val = state.headers[&key].clone();
        state.headers.insert(key, interpolate(&val, responses)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
