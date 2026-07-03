use std::collections::HashMap;

use serde::{Deserialize, Serialize};

mod body_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use serde_json::Value;

    pub fn serialize<S>(value: &Option<String>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            None => serializer.serialize_none(),
            Some(s) => match serde_json::from_str::<Value>(s) {
                Ok(v) => v.serialize(serializer),
                Err(_) => serializer.serialize_str(s),
            },
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = Option::<Value>::deserialize(deserializer)?;
        Ok(v.map(|v| match v {
            Value::String(s) => s,
            other => other.to_string(),
        }))
    }
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct ResponseRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub status: u16,
    pub headers: HashMap<String, String>,
    pub body: String,
}

// A snapshot of a request as it was actually sent (after template interpolation).
// Stored alongside each response so templates can reference `request.*`.
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct RequestRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none", with = "body_serde", default)]
    pub body: Option<String>,
}

impl RequestRecord {
    // Capture the request fields of `state` as they stand right before sending.
    pub fn from_state(state: &State) -> Self {
        Self {
            method: state.method.clone(),
            url: state.url.clone(),
            headers: state.headers.clone(),
            body: state.body.clone(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct State {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none", with = "body_serde", default)]
    pub body: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub requests: Vec<RequestRecord>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub responses: Vec<ResponseRecord>,
}

#[cfg(test)]
mod tests;
