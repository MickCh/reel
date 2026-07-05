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

// Canonical reason phrase for an HTTP status code (e.g. 200 → "OK").
// IANA-registered codes (RFC 9110 and friends); a plain lookup table so the
// data layer stays free of HTTP-client dependencies.
pub fn status_reason(code: u16) -> Option<&'static str> {
    Some(match code {
        100 => "Continue",
        101 => "Switching Protocols",
        102 => "Processing",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        203 => "Non-Authoritative Information",
        204 => "No Content",
        205 => "Reset Content",
        206 => "Partial Content",
        207 => "Multi-Status",
        208 => "Already Reported",
        226 => "IM Used",
        300 => "Multiple Choices",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        305 => "Use Proxy",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        402 => "Payment Required",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        407 => "Proxy Authentication Required",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        412 => "Precondition Failed",
        413 => "Payload Too Large",
        414 => "URI Too Long",
        415 => "Unsupported Media Type",
        416 => "Range Not Satisfiable",
        417 => "Expectation Failed",
        418 => "I'm a teapot",
        421 => "Misdirected Request",
        422 => "Unprocessable Entity",
        423 => "Locked",
        424 => "Failed Dependency",
        426 => "Upgrade Required",
        428 => "Precondition Required",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        451 => "Unavailable For Legal Reasons",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        505 => "HTTP Version Not Supported",
        506 => "Variant Also Negotiates",
        507 => "Insufficient Storage",
        508 => "Loop Detected",
        510 => "Not Extended",
        511 => "Network Authentication Required",
        _ => return None,
    })
}

#[cfg(test)]
mod tests;
