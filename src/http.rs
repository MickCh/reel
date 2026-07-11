use std::borrow::Cow;
use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;

use crate::model::{Request, ResponseRecord};

pub trait HttpClient {
    fn execute(&self, request: &Request, source: Option<&str>) -> Result<ResponseRecord>;
}

// Marker for errors no retry can fix — a malformed URL, method, or header.
// The retry loop in cli/runner.rs gives up immediately when it sees one
// instead of burning the attempt budget on a deterministic failure.
#[derive(Debug)]
pub struct PermanentError(pub String);

impl std::fmt::Display for PermanentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PermanentError {}

fn permanent(msg: String) -> anyhow::Error {
    anyhow::Error::new(PermanentError(msg))
}

pub struct ReqwestClient {
    client: reqwest::blocking::Client,
}

impl ReqwestClient {
    pub fn new(insecure: bool) -> Result<Self> {
        Ok(Self {
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(30))
                .danger_accept_invalid_certs(insecure)
                // curl parity: 3xx responses surface to the user instead of
                // being followed silently. This also keeps the cookie jar in
                // the loop — reqwest-internal redirects would swallow the
                // Set-Cookie headers of intermediate responses (login 302s)
                // and could carry Cookie/Authorization across an https→http
                // downgrade on the same host.
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| anyhow::anyhow!("error: cannot initialize HTTP client: {}", e))?,
        })
    }
}

// Decode the response body into the String the data model stores. A declared
// non-UTF-8 charset is transcoded by reqwest as before; otherwise the bytes
// are taken as-is, and invalid UTF-8 (binary data) is replaced with U+FFFD
// after a warning — piped output cannot be byte-exact for such bodies.
fn read_body(resp: reqwest::blocking::Response) -> Result<String> {
    let charset = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_ascii_lowercase)
        .and_then(|ct| {
            ct.split(';').find_map(|p| {
                p.trim()
                    .strip_prefix("charset=")
                    .map(|c| c.trim_matches('"').to_string())
            })
        });
    if matches!(charset.as_deref(), Some(c) if !matches!(c, "utf-8" | "utf8" | "us-ascii" | "ascii"))
    {
        return resp
            .text()
            .map_err(|e| anyhow::anyhow!("error reading response: {}", e));
    }
    let bytes = resp
        .bytes()
        .map_err(|e| anyhow::anyhow!("error reading response: {}", e))?;
    match String::from_utf8_lossy(&bytes) {
        Cow::Borrowed(s) => Ok(s.to_string()),
        Cow::Owned(s) => {
            eprintln!(
                "warning: response body is not valid UTF-8 (binary data?); invalid bytes were replaced — piped output is not byte-exact"
            );
            Ok(s)
        }
    }
}

impl HttpClient for ReqwestClient {
    fn execute(&self, request: &Request, source: Option<&str>) -> Result<ResponseRecord> {
        let url = match &request.url {
            Some(u) => u.clone(),
            None => anyhow::bail!("error: URL not set (use: reel url <URL>)"),
        };

        let method_name = request.method.as_deref().unwrap_or("GET").to_uppercase();
        let method = reqwest::Method::from_bytes(method_name.as_bytes())
            .map_err(|_| permanent(format!("error: invalid HTTP method '{}'", method_name)))?;

        let started = std::time::Instant::now();
        let mut builder = self.client.request(method, &url);
        for (k, v) in request.headers.iter() {
            builder = builder.header(k, v);
        }
        if let Some(body) = &request.body {
            builder = builder.body(body.clone());
        }

        // Builder errors (bad URL, bad header) are deterministic — mark them
        // permanent so the retry loop fails fast.
        let resp = builder.send().map_err(|e| {
            if e.is_builder() && !url.starts_with("http://") && !url.starts_with("https://") {
                permanent(format!(
                    "error: invalid URL '{}' — did you forget https://?",
                    url
                ))
            } else if e.is_builder() {
                permanent(format!("error: {}", e))
            } else {
                anyhow::anyhow!("error: {}", e)
            }
        })?;
        let status = resp.status();

        // Set-Cookie values are kept as a raw list — they cannot be joined
        // with ", " like other headers (Expires dates contain commas).
        let set_cookies: Vec<String> = resp
            .headers()
            .get_all(reqwest::header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok().map(str::to_string))
            .collect();

        // Duplicate header names are joined with ", " per RFC 7230.
        let mut resp_headers: HashMap<String, String> = HashMap::new();
        for (k, v) in resp.headers().iter() {
            if let Ok(v_str) = v.to_str() {
                let entry = resp_headers.entry(k.to_string()).or_default();
                if entry.is_empty() {
                    *entry = v_str.to_string();
                } else {
                    entry.push_str(", ");
                    entry.push_str(v_str);
                }
            }
        }

        let body = read_body(resp)?;

        Ok(ResponseRecord {
            source: source.map(|s| s.to_string()),
            status: status.as_u16(),
            headers: resp_headers.into(),
            body,
            set_cookies,
            elapsed_ms: started.elapsed().as_millis() as u64,
        })
    }
}
