use std::collections::HashMap;

use crate::model::{LastResponse, State};
use crate::session::save_state;

// Execute the current state as an HTTP request.
// Stores the response in state.last and saves the session before writing to stdout
// so a broken pipe (e.g. `reel send | head -5`) never prevents persistence.
// Returns false on failure so the caller can abort a chain.
pub fn execute(state: &mut State) -> bool {
    let url = match &state.url {
        Some(u) => u.clone(),
        None => {
            eprintln!("error: URL not set (use: reel url <URL>)");
            return false;
        }
    };

    let method = state.method.as_deref().unwrap_or("GET").to_uppercase();

    let client = reqwest::blocking::Client::new();

    let req_builder = match method.as_str() {
        "GET" => client.get(&url),
        "POST" => client.post(&url),
        "PUT" => client.put(&url),
        "PATCH" => client.patch(&url),
        "DELETE" => client.delete(&url),
        "HEAD" => client.head(&url),
        other => {
            eprintln!("error: unknown method '{}'", other);
            return false;
        }
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

            let resp_headers: HashMap<String, String> = resp
                .headers()
                .iter()
                .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_string())))
                .collect();

            match resp.text() {
                Ok(body) => {
                    state.last = Some(LastResponse {
                        status: status.as_u16(),
                        headers: resp_headers,
                        body: body.clone(),
                    });
                    save_state(state);
                    print!("{}", body);
                    true
                }
                Err(e) => {
                    eprintln!("error reading response: {}", e);
                    false
                }
            }
        }
        Err(e) => {
            eprintln!("error: {}", e);
            false
        }
    }
}
