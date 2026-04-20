use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct ResponseRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
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
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub responses: Vec<ResponseRecord>,
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
        assert!(s.responses.is_empty());
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
        assert!(restored.responses.is_empty());
    }

    #[test]
    fn state_omitted_responses_deserializes_as_empty() {
        let json = r#"{"method":"GET","url":"https://x.com","headers":{}}"#;
        let s: State = serde_json::from_str(json).unwrap();
        assert!(s.responses.is_empty());
    }

    #[test]
    fn state_old_last_field_is_ignored() {
        let json = r#"{"method":"GET","url":"https://x.com","headers":{},"last":{"status":200,"headers":{},"body":"ok"}}"#;
        let s: State = serde_json::from_str(json).unwrap();
        assert!(s.responses.is_empty());
    }

    #[test]
    fn response_record_round_trips() {
        let mut headers = HashMap::new();
        headers.insert("content-type".to_string(), "application/json".to_string());
        let r = ResponseRecord {
            source: Some("login.json".to_string()),
            status: 200,
            headers,
            body: "ok".to_string(),
        };
        let json = serde_json::to_string(&r).unwrap();
        let restored: ResponseRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.status, 200);
        assert_eq!(restored.body, "ok");
        assert_eq!(restored.source.as_deref(), Some("login.json"));
        assert_eq!(restored.headers["content-type"], "application/json");
    }

    #[test]
    fn response_record_without_source_omits_field() {
        let r = ResponseRecord {
            source: None,
            status: 404,
            headers: HashMap::new(),
            body: "not found".to_string(),
        };
        let json = serde_json::to_string(&r).unwrap();
        assert!(!json.contains("source"));
        let restored: ResponseRecord = serde_json::from_str(&json).unwrap();
        assert!(restored.source.is_none());
        assert_eq!(restored.status, 404);
    }
}
