use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;

use crate::model::{ResponseRecord, State};

pub trait HttpClient {
    fn execute(&self, state: &State, source: Option<&str>) -> Result<ResponseRecord>;
}

pub struct ReqwestClient {
    client: reqwest::blocking::Client,
}

impl ReqwestClient {
    pub fn new(insecure: bool) -> Self {
        Self {
            client: reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(30))
                .danger_accept_invalid_certs(insecure)
                .build()
                .expect("failed to build HTTP client"),
        }
    }
}

impl HttpClient for ReqwestClient {
    fn execute(&self, state: &State, source: Option<&str>) -> Result<ResponseRecord> {
        let url = match &state.url {
            Some(u) => u.clone(),
            None => anyhow::bail!("error: URL not set (use: reel url <URL>)"),
        };

        let method = state.method.as_deref().unwrap_or("GET").to_uppercase();

        let req_builder = match method.as_str() {
            "GET" => self.client.get(&url),
            "POST" => self.client.post(&url),
            "PUT" => self.client.put(&url),
            "PATCH" => self.client.patch(&url),
            "DELETE" => self.client.delete(&url),
            "HEAD" => self.client.head(&url),
            "OPTIONS" => self.client.request(reqwest::Method::OPTIONS, &url),
            other => match reqwest::Method::from_bytes(other.as_bytes()) {
                Ok(m) => self.client.request(m, &url),
                Err(_) => anyhow::bail!("error: invalid HTTP method '{}'", other),
            },
        };

        let req_builder = state
            .headers
            .iter()
            .fold(req_builder, |b, (k, v)| b.header(k, v));

        let req_builder = match &state.body {
            Some(b) => req_builder.body(b.clone()),
            None => req_builder,
        };

        if state.body.is_some()
            && !state.headers.keys().any(|k| k.eq_ignore_ascii_case("content-type"))
        {
            eprintln!("warning: body is set but Content-Type header is missing");
        }

        let resp = req_builder.send().map_err(|e| {
            if e.is_builder() && !url.starts_with("http://") && !url.starts_with("https://") {
                anyhow::anyhow!("error: invalid URL '{}' — did you forget https://?", url)
            } else {
                anyhow::anyhow!("error: {}", e)
            }
        })?;
        let status = resp.status();

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

        let body = resp.text().map_err(|e| anyhow::anyhow!("error reading response: {}", e))?;

        Ok(ResponseRecord {
            source: source.map(|s| s.to_string()),
            status: status.as_u16(),
            headers: resp_headers,
            body,
        })
    }
}
