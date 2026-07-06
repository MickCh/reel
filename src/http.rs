use std::collections::HashMap;
use std::time::Duration;

use anyhow::Result;

use crate::model::{Request, ResponseRecord};

pub trait HttpClient {
    fn execute(&self, request: &Request, source: Option<&str>) -> Result<ResponseRecord>;
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
    fn execute(&self, request: &Request, source: Option<&str>) -> Result<ResponseRecord> {
        let url = match &request.url {
            Some(u) => u.clone(),
            None => anyhow::bail!("error: URL not set (use: reel url <URL>)"),
        };

        let method_name = request.method.as_deref().unwrap_or("GET").to_uppercase();
        let method = reqwest::Method::from_bytes(method_name.as_bytes())
            .map_err(|_| anyhow::anyhow!("error: invalid HTTP method '{}'", method_name))?;

        let started = std::time::Instant::now();
        let mut builder = self.client.request(method, &url);
        for (k, v) in request.headers.iter() {
            builder = builder.header(k, v);
        }
        if let Some(body) = &request.body {
            builder = builder.body(body.clone());
        }

        let resp = builder.send().map_err(|e| {
            if e.is_builder() && !url.starts_with("http://") && !url.starts_with("https://") {
                anyhow::anyhow!("error: invalid URL '{}' — did you forget https://?", url)
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

        let body = resp
            .text()
            .map_err(|e| anyhow::anyhow!("error reading response: {}", e))?;

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
