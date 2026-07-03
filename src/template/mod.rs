use anyhow::{Result, bail};

use crate::model::{RequestRecord, ResponseRecord, State};

// The data a template can interpolate against: the request/response history of
// the current session. Bare `body`/`status`/`headers.*` refer to the last
// response; `request.*`/`request[N].*` reach the captured requests; `env.*`
// reads process environment variables (independent of history).
pub struct Context<'a> {
    pub requests: &'a [RequestRecord],
    pub responses: &'a [ResponseRecord],
}

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

// Evaluate a field expression against a specific request record.
//
// Supported forms:
//   method              → HTTP method
//   url                 → request URL
//   body                → raw request body
//   body.<path>         → dot-path into the JSON request body
//   headers.<name>      → request header value (case-insensitive lookup)
fn eval_request_expr(expr: &str, record: &RequestRecord) -> Result<String> {
    match expr {
        "method" => record
            .method
            .clone()
            .ok_or_else(|| anyhow::anyhow!("request method not set")),
        "url" => record
            .url
            .clone()
            .ok_or_else(|| anyhow::anyhow!("request url not set")),
        "body" => record
            .body
            .clone()
            .ok_or_else(|| anyhow::anyhow!("request body not set")),
        _ => {
            if let Some(name) = expr.strip_prefix("headers.") {
                return record
                    .headers
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(name))
                    .map(|(_, v)| v.clone())
                    .ok_or_else(|| anyhow::anyhow!("header '{}' not found in request", name));
            }
            if let Some(path) = expr.strip_prefix("body.") {
                let body = record
                    .body
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("request body not set"))?;
                let json: serde_json::Value = serde_json::from_str(body)
                    .map_err(|e| anyhow::anyhow!("request body is not valid JSON: {}", e))?;
                return traverse_json(&json, path)
                    .ok_or_else(|| anyhow::anyhow!("path '{}' not found in request body", path));
            }
            bail!("unknown request expression 'request.{}'", expr)
        }
    }
}

// Split a `[N].field` or `.field` suffix (following a `request`/`response` prefix)
// into an optional 1-based index and the trailing field expression. A bare `.field`
// (no brackets) yields `None`, meaning "the last record".
fn split_index(rest: &str, kind: &str) -> Result<(Option<usize>, String)> {
    if let Some(after) = rest.strip_prefix('[') {
        let bracket_end = after
            .find(']')
            .ok_or_else(|| anyhow::anyhow!("unclosed '[' in '{}[...'", kind))?;
        let idx_str = &after[..bracket_end];
        let idx: usize = idx_str
            .parse()
            .map_err(|_| anyhow::anyhow!("invalid {} index '{}[{}]'", kind, kind, idx_str))?;
        if idx == 0 {
            bail!("{} indices start at 1", kind);
        }
        let field = after[bracket_end + 1..].strip_prefix('.').ok_or_else(|| {
            anyhow::anyhow!(
                "expected '{}[{}].<field>' (e.g. {}[{}].body.token)",
                kind,
                idx,
                kind,
                idx
            )
        })?;
        Ok((Some(idx), field.to_string()))
    } else if let Some(field) = rest.strip_prefix('.') {
        Ok((None, field.to_string()))
    } else {
        bail!("expected '{}.<field>' or '{}[N].<field>'", kind, kind)
    }
}

// Resolve an optional 1-based index into `records`; `None` selects the last one.
fn pick<'a, T>(records: &'a [T], idx: Option<usize>, kind: &str) -> Result<&'a T> {
    match idx {
        None => records
            .last()
            .ok_or_else(|| anyhow::anyhow!("no previous {}", kind)),
        Some(n) => records.get(n - 1).ok_or_else(|| {
            anyhow::anyhow!(
                "{}[{}]: only {} {}(s) available",
                kind,
                n,
                records.len(),
                kind
            )
        }),
    }
}

// Evaluate a single ${{ expr }} against the session history and environment.
//
// Supported forms:
//   env.<NAME>                 → process environment variable
//   status                     → status of the last response
//   body                       → raw body of the last response
//   body.<path>                → dot-path into the last response body
//   headers.<name>             → header from the last response
//   response[N].status         → status of the Nth response (1-based)
//   response[N].body[.path]    → body (or dot-path) of the Nth response
//   response[N].headers.<name> → header from the Nth response
//   request.method|url|body    → field of the last request
//   request.body.<path>        → dot-path into the last request body
//   request.headers.<name>     → header from the last request
//   request[N].<field>         → field of the Nth request (1-based)
fn eval_expr(expr: &str, ctx: &Context) -> Result<String> {
    // Environment variables — independent of request/response history.
    if let Some(name) = expr.strip_prefix("env.") {
        return std::env::var(name)
            .map_err(|_| anyhow::anyhow!("environment variable '{}' not set", name));
    }

    // Request history: request.<field> (last) or request[N].<field> (1-based).
    if let Some(rest) = expr.strip_prefix("request")
        && (rest.starts_with('.') || rest.starts_with('['))
    {
        let (idx, field) = split_index(rest, "request")?;
        let record = pick(ctx.requests, idx, "request")?;
        return eval_request_expr(&field, record);
    }

    // Response history: response[N].<field> (1-based).
    if let Some(rest) = expr.strip_prefix("response")
        && rest.starts_with('[')
    {
        let (idx, field) = split_index(rest, "response")?;
        let record = pick(ctx.responses, idx, "response")?;
        return eval_record_expr(&field, record);
    }

    // Bare forms refer to the last response.
    let record = ctx
        .responses
        .last()
        .ok_or_else(|| anyhow::anyhow!("no previous response"))?;
    eval_record_expr(expr, record)
}

// Replace all ${{ expr }} placeholders in `text` using values from `ctx`.
pub fn interpolate(text: &str, ctx: &Context) -> Result<String> {
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
        result.push_str(&eval_expr(expr, ctx)?);
    }
    result.push_str(remaining);
    Ok(result)
}

// Apply interpolation to all string fields of state (url, body, header values).
pub fn apply_interpolation(state: &mut State, ctx: &Context) -> Result<()> {
    if let Some(url) = state.url.take() {
        state.url = Some(interpolate(&url, ctx)?);
    }
    if let Some(body) = state.body.take() {
        state.body = Some(interpolate(&body, ctx)?);
    }
    let keys: Vec<String> = state.headers.keys().cloned().collect();
    for key in keys {
        let val = state.headers[&key].clone();
        state.headers.insert(key, interpolate(&val, ctx)?);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
