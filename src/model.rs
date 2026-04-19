use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct LastResponse {
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct State {
    pub method: Option<String>,
    pub url: Option<String>,
    pub headers: HashMap<String, String>,
    pub body: Option<String>,
    pub last: Option<LastResponse>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_default_is_empty() {
        let s = State::default();
        assert!(s.method.is_none());
        assert!(s.url.is_none());
        assert!(s.headers.is_empty());
        assert!(s.body.is_none());
        assert!(s.last.is_none());
    }

    #[test]
    fn state_round_trips_through_json() {
        let mut s = State::default();
        s.method = Some("POST".to_string());
        s.url = Some("https://example.com".to_string());
        s.headers.insert("X-Foo".to_string(), "bar".to_string());
        s.body = Some(r#"{"key":"val"}"#.to_string());

        let json = serde_json::to_string(&s).unwrap();
        let restored: State = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.method, s.method);
        assert_eq!(restored.url, s.url);
        assert_eq!(restored.headers["X-Foo"], "bar");
        assert_eq!(restored.body, s.body);
        assert!(restored.last.is_none());
    }

    #[test]
    fn state_omitted_last_deserializes_as_none() {
        let json = r#"{"method":"GET","url":"https://x.com","headers":{}}"#;
        let s: State = serde_json::from_str(json).unwrap();
        assert!(s.last.is_none());
    }

    #[test]
    fn last_response_round_trips() {
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());
        let lr = LastResponse { status: 200, headers, body: "ok".to_string() };
        let json = serde_json::to_string(&lr).unwrap();
        let restored: LastResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.status, 200);
        assert_eq!(restored.body, "ok");
        assert_eq!(restored.headers["content-type"], "application/json");
    }
}
