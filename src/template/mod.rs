use std::collections::HashMap;

use anyhow::{Result, bail};

use crate::model::{Request, ResponseRecord};

// The data a template can interpolate against: the request/response history of
// the current session. Bare `body`/`status`/`headers.*` refer to the last
// response; `request.*`/`request[N].*` reach the captured requests; `env.*`
// reads process environment variables and `var.*` the session variables set
// with `reel var` (both independent of history).
pub struct Context<'a> {
    pub requests: &'a [Request],
    pub responses: &'a [ResponseRecord],
    pub vars: &'a HashMap<String, String>,
}

// Marker for "the expression is well-formed but the data it references is
// absent": an unset env var, missing history entry, absent JSON key or
// header, a non-JSON body. Only these errors are absorbed by a `| default:`
// fallback; structural errors (unknown function or expression, bad index
// syntax) always propagate, so a typo cannot silently become the default.
#[derive(Debug)]
struct Unresolvable(String);

impl std::fmt::Display for Unresolvable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unresolvable {}

fn unresolvable(msg: String) -> anyhow::Error {
    anyhow::Error::new(Unresolvable(msg))
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
//   elapsed             → request duration in milliseconds
//   body                → raw response body
//   body.<path>         → dot-path into the JSON body (e.g. body.access.token)
//   headers.<name>      → response header value (e.g. headers.content-type)
fn eval_record_expr(expr: &str, record: &ResponseRecord) -> Result<String> {
    if expr == "status" {
        return Ok(record.status.to_string());
    }
    if expr == "elapsed" {
        return Ok(record.elapsed_ms.to_string());
    }
    if expr == "body" {
        return Ok(record.body.clone());
    }
    if let Some(name) = expr.strip_prefix("headers.") {
        return record
            .headers
            .get(name)
            .map(str::to_string)
            .ok_or_else(|| unresolvable(format!("header '{}' not found in response", name)));
    }
    if let Some(path) = expr.strip_prefix("body.") {
        let json: serde_json::Value = serde_json::from_str(&record.body)
            .map_err(|e| unresolvable(format!("response body is not valid JSON: {}", e)))?;
        return traverse_json(&json, path)
            .ok_or_else(|| unresolvable(format!("path '{}' not found in response body", path)));
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
fn eval_request_expr(expr: &str, record: &Request) -> Result<String> {
    match expr {
        "method" => record
            .method
            .clone()
            .ok_or_else(|| unresolvable("request method not set".to_string())),
        "url" => record
            .url
            .clone()
            .ok_or_else(|| unresolvable("request url not set".to_string())),
        "body" => record
            .body
            .clone()
            .ok_or_else(|| unresolvable("request body not set".to_string())),
        _ => {
            if let Some(name) = expr.strip_prefix("headers.") {
                return record.headers.get(name).map(str::to_string).ok_or_else(|| {
                    unresolvable(format!("header '{}' not found in request", name))
                });
            }
            if let Some(path) = expr.strip_prefix("body.") {
                let body = record
                    .body
                    .as_deref()
                    .ok_or_else(|| unresolvable("request body not set".to_string()))?;
                let json: serde_json::Value = serde_json::from_str(body)
                    .map_err(|e| unresolvable(format!("request body is not valid JSON: {}", e)))?;
                return traverse_json(&json, path).ok_or_else(|| {
                    unresolvable(format!("path '{}' not found in request body", path))
                });
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
            .ok_or_else(|| unresolvable(format!("no previous {}", kind))),
        Some(n) => records.get(n - 1).ok_or_else(|| {
            unresolvable(format!(
                "{}[{}]: only {} {}(s) available",
                kind,
                n,
                records.len(),
                kind
            ))
        }),
    }
}

fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = u32::from_be_bytes([
            0,
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ]);
        out.push(ALPHABET[(n >> 18) as usize & 63] as char);
        out.push(ALPHABET[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

// Evaluate a `name(args)` template function.
//
// Supported functions:
//   uuid()              → random v4 UUID (e.g. idempotency keys)
//   now()               → current Unix timestamp in seconds
//   now(±N)             → Unix timestamp shifted by N seconds (e.g. now(+3600))
//   base64(<arg>)       → base64 of the argument: either a single-quoted
//                         literal ('user:pass') or a nested expression
//                         (env.CREDS, body.token, ...), evaluated recursively
fn eval_function(name: &str, args: &str, ctx: &Context) -> Result<String> {
    match name {
        "uuid" => {
            if !args.is_empty() {
                bail!("uuid() takes no arguments");
            }
            Ok(uuid::Uuid::new_v4().to_string())
        }
        "now" => {
            let offset: i64 = if args.is_empty() {
                0
            } else {
                args.strip_prefix('+')
                    .unwrap_or(args)
                    .parse()
                    .map_err(|_| anyhow::anyhow!("invalid now() offset '{}'", args))?
            };
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("system clock before Unix epoch")
                .as_secs() as i64;
            Ok((now + offset).to_string())
        }
        "base64" => {
            if args.is_empty() {
                bail!("base64() requires an argument (a 'literal' or an expression)");
            }
            let value = match args.strip_prefix('\'').and_then(|a| a.strip_suffix('\'')) {
                Some(literal) => literal.to_string(),
                None => eval_expr(args, ctx)?,
            };
            Ok(base64_encode(value.as_bytes()))
        }
        other => bail!(
            "unknown template function '{}()' (supported: uuid, now, base64)",
            other
        ),
    }
}

// Evaluate a single ${{ expr }} against the session history and environment.
//
// Supported forms:
//   uuid() / now() / base64(<arg>) → template functions (see eval_function)
//   env.<NAME>                 → process environment variable
//   var.<name>                 → session variable set with `reel var`
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
    // Function call: name(args). Only identifier-shaped names count — a
    // parenthesis inside a field path (a JSON key like body.items(0)) must
    // stay a path segment, not become a function call.
    if let Some(open) = expr.find('(')
        && let Some(args) = expr[open + 1..].strip_suffix(')')
    {
        let name = expr[..open].trim_end();
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            return eval_function(name, args.trim(), ctx);
        }
    }

    // Environment variables — independent of request/response history.
    if let Some(name) = expr.strip_prefix("env.") {
        return std::env::var(name)
            .map_err(|_| unresolvable(format!("environment variable '{}' not set", name)));
    }

    // Session variables — like env.*, independent of history. Case-sensitive.
    if let Some(name) = expr.strip_prefix("var.") {
        return ctx.vars.get(name).cloned().ok_or_else(|| {
            unresolvable(format!(
                "variable '{}' not set (set it with: reel var {} <VALUE>)",
                name, name
            ))
        });
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
        .ok_or_else(|| unresolvable("no previous response".to_string()))?;
    eval_record_expr(expr, record)
}

// Check an `expect` condition against the session history. Supported forms:
//
//   <expr>                     → passes when the expression resolves at all
//   <expr> == <value>          → string equality (status codes compare as strings)
//   <expr> != <value>          → string inequality
//   <expr> contains <value>    → substring match
//
// `<expr>` is any template expression (no `${{ }}` wrapper); the first
// whitespace outside single quotes separates it from the operator — the
// expression itself may contain a quoted literal with spaces, e.g.
// `base64('user pass') == <v>`.
pub fn check_condition(condition: &str, ctx: &Context) -> Result<()> {
    let condition = condition.trim();
    if condition.is_empty() {
        bail!("empty expect condition");
    }

    let (expr, rest) = split_condition(condition);
    let actual = eval_expr(expr, ctx).map_err(|e| anyhow::anyhow!("expect '{}': {}", expr, e))?;

    if rest.is_empty() {
        return Ok(());
    }
    let (op, expected) = match rest.split_once(char::is_whitespace) {
        Some((op, value)) => (op, value.trim_start()),
        None => (rest, ""),
    };
    let passed = match op {
        "==" => actual == expected,
        "!=" => actual != expected,
        "contains" => actual.contains(expected),
        other => bail!(
            "unknown expect operator '{}' (supported: ==, !=, contains)",
            other
        ),
    };
    if !passed {
        bail!("expect failed: {} (actual: {})", condition, actual);
    }
    Ok(())
}

// Split an expect condition into expression and operator part at the first
// whitespace outside single quotes (quotes shield literals like
// base64('user pass')).
fn split_condition(condition: &str) -> (&str, &str) {
    let mut in_quotes = false;
    for (i, c) in condition.char_indices() {
        match c {
            '\'' => in_quotes = !in_quotes,
            c if c.is_whitespace() && !in_quotes => {
                return (&condition[..i], condition[i + c.len_utf8()..].trim_start());
            }
            _ => {}
        }
    }
    (condition, "")
}

// Split `expr | default: value` at the first '|' outside single quotes
// (quotes shield literals like base64('a|b')).
fn split_default(expr: &str) -> Option<(&str, &str)> {
    let mut in_quotes = false;
    for (i, c) in expr.char_indices() {
        match c {
            '\'' => in_quotes = !in_quotes,
            '|' if !in_quotes => return Some((&expr[..i], &expr[i + 1..])),
            _ => {}
        }
    }
    None
}

// Evaluate one placeholder, honouring an optional `| default: <value>`
// fallback: when the expression cannot be resolved (unset env var, missing
// history, absent JSON key, ...), the default text is used instead. Only
// `Unresolvable` errors are absorbed — a structural error such as a
// misspelled function name propagates even with a default present.
fn eval_placeholder(expr: &str, ctx: &Context) -> Result<String> {
    let Some((expr_part, rest)) = split_default(expr) else {
        return eval_expr(expr, ctx);
    };
    let default = rest.trim().strip_prefix("default:").ok_or_else(|| {
        anyhow::anyhow!(
            "expected '| default: <value>' after '|' in '${{{{ {} }}}}'",
            expr
        )
    })?;
    match eval_expr(expr_part.trim_end(), ctx) {
        Ok(value) => Ok(value),
        Err(e) if e.is::<Unresolvable>() => Ok(default.trim_start().to_string()),
        Err(e) => Err(e),
    }
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
        result.push_str(&eval_placeholder(expr, ctx)?);
    }
    result.push_str(remaining);
    Ok(result)
}

// Apply interpolation to all string fields of the request (url, body, header values).
pub fn apply_interpolation(request: &mut Request, ctx: &Context) -> Result<()> {
    if let Some(url) = request.url.take() {
        request.url = Some(interpolate(&url, ctx)?);
    }
    if let Some(body) = request.body.take() {
        request.body = Some(interpolate(&body, ctx)?);
    }
    for value in request.headers.values_mut() {
        *value = interpolate(value, ctx)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
