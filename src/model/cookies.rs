use serde::{Deserialize, Serialize};

// A cookie captured from a Set-Cookie response header, scoped by domain and
// path (simplified RFC 6265: Expires/Max-Age lifetimes are not tracked — a
// cookie lives as long as the reel session — but a Max-Age <= 0 deletion is
// honoured, and HttpOnly/SameSite are irrelevant for a CLI client).
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct Cookie {
    pub name: String,
    pub value: String,
    // Lowercase. The request host, unless a Domain attribute widened it.
    pub domain: String,
    pub path: String,
    #[serde(default)]
    pub secure: bool,
    // True when the Set-Cookie had no Domain attribute: exact-host match only.
    #[serde(default)]
    pub host_only: bool,
}

// Outcome of parsing one Set-Cookie header: a cookie to store, or an
// instruction to delete an existing one (Max-Age <= 0).
pub enum SetCookie {
    Set(Cookie),
    Delete {
        name: String,
        domain: String,
        path: String,
    },
}

// RFC 6265 default-path: the request path up to (excluding) its last '/'.
fn default_path(request_path: &str) -> String {
    if !request_path.starts_with('/') {
        return "/".to_string();
    }
    match request_path.rfind('/') {
        Some(0) | None => "/".to_string(),
        Some(idx) => request_path[..idx].to_string(),
    }
}

// Parse a raw Set-Cookie header value in the context of the request that
// received it. Returns None for malformed values and for Domain attributes
// that are not a suffix of the request host (a site cannot set cookies for
// an unrelated domain). `request_host` must be lowercase.
pub fn parse_set_cookie(raw: &str, request_host: &str, request_path: &str) -> Option<SetCookie> {
    let mut segments = raw.split(';');
    let (name, value) = segments.next()?.split_once('=')?;
    let name = name.trim();
    if name.is_empty() {
        return None;
    }

    let mut cookie = Cookie {
        name: name.to_string(),
        value: value.trim().to_string(),
        domain: request_host.to_string(),
        path: default_path(request_path),
        secure: false,
        host_only: true,
    };
    let mut expired = false;

    for segment in segments {
        let (attr, attr_value) = match segment.split_once('=') {
            Some((a, v)) => (a.trim(), v.trim()),
            None => (segment.trim(), ""),
        };
        if attr.eq_ignore_ascii_case("domain") {
            let domain = attr_value.trim_start_matches('.').to_lowercase();
            if domain.is_empty() {
                continue;
            }
            let is_suffix = request_host == domain
                || request_host
                    .strip_suffix(&domain)
                    .is_some_and(|prefix| prefix.ends_with('.'));
            if !is_suffix {
                return None;
            }
            cookie.domain = domain;
            cookie.host_only = false;
        } else if attr.eq_ignore_ascii_case("path") {
            if attr_value.starts_with('/') {
                cookie.path = attr_value.to_string();
            }
        } else if attr.eq_ignore_ascii_case("max-age") {
            if let Ok(age) = attr_value.parse::<i64>() {
                expired = age <= 0;
            }
        } else if attr.eq_ignore_ascii_case("secure") {
            cookie.secure = true;
        }
    }

    Some(if expired {
        SetCookie::Delete {
            name: cookie.name,
            domain: cookie.domain,
            path: cookie.path,
        }
    } else {
        SetCookie::Set(cookie)
    })
}

// RFC 6265 path-match.
fn path_matches(cookie_path: &str, request_path: &str) -> bool {
    request_path == cookie_path
        || (request_path.starts_with(cookie_path)
            && (cookie_path.ends_with('/')
                || request_path.as_bytes().get(cookie_path.len()) == Some(&b'/')))
}

impl Cookie {
    fn domain_matches(&self, host: &str) -> bool {
        if self.host_only {
            host == self.domain
        } else {
            host == self.domain
                || host
                    .strip_suffix(&self.domain)
                    .is_some_and(|prefix| prefix.ends_with('.'))
        }
    }

    pub fn matches(&self, host: &str, path: &str, https: bool) -> bool {
        self.domain_matches(host) && path_matches(&self.path, path) && (https || !self.secure)
    }
}

// Apply one parsed Set-Cookie to the jar. A Set replaces any cookie with the
// same (name, domain, path); a Delete removes it.
pub fn update_jar(jar: &mut Vec<Cookie>, update: SetCookie) {
    let (name, domain, path) = match &update {
        SetCookie::Set(c) => (&c.name, &c.domain, &c.path),
        SetCookie::Delete { name, domain, path } => (name, domain, path),
    };
    jar.retain(|c| !(c.name == *name && c.domain == *domain && c.path == *path));
    if let SetCookie::Set(cookie) = update {
        jar.push(cookie);
    }
}

// Build the Cookie header value for a request to `host`/`path`, or None when
// no stored cookie matches. Longer paths come first (RFC 6265 §5.4), name as
// tiebreaker for deterministic output.
pub fn cookie_header(jar: &[Cookie], host: &str, path: &str, https: bool) -> Option<String> {
    let mut matching: Vec<&Cookie> = jar
        .iter()
        .filter(|c| c.matches(host, path, https))
        .collect();
    if matching.is_empty() {
        return None;
    }
    matching.sort_by(|a, b| {
        b.path
            .len()
            .cmp(&a.path.len())
            .then_with(|| a.name.cmp(&b.name))
    });
    Some(
        matching
            .iter()
            .map(|c| format!("{}={}", c.name, c.value))
            .collect::<Vec<_>>()
            .join("; "),
    )
}
