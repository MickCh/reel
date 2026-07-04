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

// HTTP headers with case-insensitive name semantics. Names keep their original
// casing (round-tripped through session/preset files as-is), but lookup,
// insertion dedup, and removal all compare names case-insensitively.
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
#[serde(transparent)]
pub struct Headers(HashMap<String, String>);

impl Headers {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    // Replaces any existing entry whose name matches case-insensitively;
    // the new name's casing is kept as given.
    pub fn insert(&mut self, name: String, value: String) {
        self.0.retain(|k, _| !k.eq_ignore_ascii_case(&name));
        self.0.insert(name, value);
    }

    // Returns true if an entry was removed.
    pub fn remove(&mut self, name: &str) -> bool {
        let before = self.0.len();
        self.0.retain(|k, _| !k.eq_ignore_ascii_case(name));
        self.0.len() != before
    }

    pub fn clear(&mut self) {
        self.0.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &String)> {
        self.0.iter()
    }

    pub fn values_mut(&mut self) -> impl Iterator<Item = &mut String> {
        self.0.values_mut()
    }

    // Entries sorted by name — for deterministic display output.
    pub fn sorted(&self) -> Vec<(&String, &String)> {
        let mut entries: Vec<_> = self.0.iter().collect();
        entries.sort_by_key(|(k, _)| k.as_str());
        entries
    }
}

impl From<HashMap<String, String>> for Headers {
    fn from(map: HashMap<String, String>) -> Self {
        Self(map)
    }
}

impl FromIterator<(String, String)> for Headers {
    fn from_iter<I: IntoIterator<Item = (String, String)>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

// An HTTP request: the fields the user builds up before sending. The same type
// is used for history records — each send/then stores a snapshot of the request
// as actually sent (after template interpolation), so `request.*` templates
// reflect the real values.
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct Request {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Headers::is_empty")]
    pub headers: Headers,
    #[serde(skip_serializing_if = "Option::is_none", with = "body_serde", default)]
    pub body: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct ResponseRecord {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub status: u16,
    pub headers: Headers,
    pub body: String,
}

// A session: the request currently being built plus the request/response
// history accumulated by send/then. The current request is flattened during
// (de)serialization, so session and preset files keep their flat JSON shape.
#[derive(Debug, Serialize, Deserialize, Default, Clone)]
pub struct State {
    #[serde(flatten)]
    pub request: Request,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub requests: Vec<Request>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub responses: Vec<ResponseRecord>,
}

#[cfg(test)]
mod tests;
