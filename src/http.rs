use std::collections::HashMap;
use std::time::Duration;

use crate::model::{ResponseRecord, State};
use crate::session::save_state;

pub fn build_client(insecure: bool) -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(30))
        .danger_accept_invalid_certs(insecure)
        .build()
        .expect("failed to build HTTP client")
}

// Execute the current state as an HTTP request.
// Appends the response to state.responses and saves the session before writing to stdout
// so a broken pipe (e.g. `reel send | head -5`) never prevents persistence.
// Returns Ok(status_code) on success, Err(()) on network/connection failure.
pub fn execute(
    client: &reqwest::blocking::Client,
    state: &mut State,
    source: Option<&str>,
) -> Result<u16, ()> {
    let url = match &state.url {
        Some(u) => u.clone(),
        None => {
            eprintln!("error: URL not set (use: reel url <URL>)");
            return Err(());
        }
    };

    let method = state.method.as_deref().unwrap_or("GET").to_uppercase();

    let req_builder = match method.as_str() {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "PATCH" => client.patch(&url),
        "DELETE" => client.delete(&url),
        "HEAD" => client.head(&url),
        other => match reqwest::Method::from_bytes(other.as_bytes()) {
            Ok(m) => client.request(m, &url),
            Err(_) => {
                eprintln!("error: invalid HTTP method '{}'", other);
                return Err(());
            }
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

    match req_builder.send() {
        Ok(resp) => {
            let status = resp.status();
            eprintln!("{} {}", status.as_u16(), status.canonical_reason().unwrap_or(""));

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

            match resp.text() {
                Ok(body) => {
                    state.responses.push(ResponseRecord {
                        source: source.map(|s| s.to_string()),
                        status: status.as_u16(),
                        headers: resp_headers,
                        body: body.clone(),
                    });
                    save_state(state);
                    print!("{}", body);
                    Ok(status.as_u16())
                }
                Err(e) => {
                    eprintln!("error reading response: {}", e);
                    Err(())
                }
            }
        }
        Err(e) => {
            eprintln!("error: {}", e);
            Err(())
        }
    }
}
