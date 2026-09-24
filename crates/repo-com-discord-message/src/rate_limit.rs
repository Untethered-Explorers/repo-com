use std::time::Duration;

use reqwest::{
    Response,
    header::{HeaderMap, RETRY_AFTER},
};

/// The maximum Discord-directed wait exposed for one transport attempt.
pub const MAX_DISCORD_DIRECTED_WAIT: Duration = Duration::from_secs(30);

/// Whether dynamic metadata describes a route bucket or the global limit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RateLimitClass {
    /// A route bucket, including user- or shared-scoped 429 responses.
    Route,
    /// Discord's global limit.
    Global,
    /// No recognized rate-limit metadata was present.
    Unknown,
}

/// The scope Discord reported for a 429 response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RateLimitScope {
    /// A per-route or global limit identified by the response headers.
    Route,
    /// Discord's global limit.
    Global,
    /// A limit scoped to the authenticated bot or user.
    User,
    /// A limit shared by multiple resource scopes.
    Shared,
    /// Discord omitted or used an unrecognized scope.
    Unknown,
}

/// Dynamic rate-limit metadata parsed without retaining a response body.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateLimitInfo {
    /// Whether this is route or global limit metadata.
    pub class: RateLimitClass,
    /// Scope reported by `X-RateLimit-Scope`, when known.
    pub scope: RateLimitScope,
    /// Stable bucket identifier reported by Discord.
    pub bucket: Option<String>,
    /// Limit reported by `X-RateLimit-Limit`.
    pub limit: Option<u64>,
    /// Remaining requests reported by `X-RateLimit-Remaining`.
    pub remaining: Option<u64>,
    /// Absolute reset value reported by `X-RateLimit-Reset`.
    pub reset_at: Option<String>,
    /// Dynamic reset delay from `X-RateLimit-Reset-After`, capped at 30 seconds.
    pub reset_after: Option<Duration>,
    /// Dynamic 429 delay from `Retry-After` or `retry_after`, capped at 30 seconds.
    pub retry_after: Option<Duration>,
}

impl RateLimitInfo {
    pub(crate) fn from_success(response: &Response) -> Self {
        Self::from_headers(response.headers(), false)
    }

    pub(crate) fn from_rate_limited(response: &Response) -> Self {
        Self::from_headers(response.headers(), true)
    }

    pub(crate) fn include_body_retry_after(&mut self, seconds: Option<f64>) {
        let Some(candidate) = seconds.and_then(bounded_duration) else {
            return;
        };
        self.retry_after = Some(match self.retry_after {
            Some(current) => current.max(candidate),
            None => candidate,
        });
    }

    pub(crate) fn include_body_global(&mut self, global: Option<bool>) {
        if global == Some(true) {
            self.class = RateLimitClass::Global;
            self.scope = RateLimitScope::Global;
        }
    }

    fn from_headers(headers: &HeaderMap, limited: bool) -> Self {
        let scope = scope(headers);
        let global = bool_header(headers, "x-ratelimit-global").unwrap_or(false)
            || scope == RateLimitScope::Global;
        let has_metadata = headers.contains_key("x-ratelimit-limit")
            || headers.contains_key("x-ratelimit-remaining")
            || headers.contains_key("x-ratelimit-reset")
            || headers.contains_key("x-ratelimit-reset-after")
            || headers.contains_key("x-ratelimit-bucket");
        let class = if global {
            RateLimitClass::Global
        } else if has_metadata || limited {
            RateLimitClass::Route
        } else {
            RateLimitClass::Unknown
        };
        let scope = if !limited && scope == RateLimitScope::Unknown {
            RateLimitScope::Route
        } else {
            scope
        };

        Self {
            class,
            scope,
            bucket: bounded_header(headers, "x-ratelimit-bucket", 128),
            limit: unsigned_header(headers, "x-ratelimit-limit"),
            remaining: unsigned_header(headers, "x-ratelimit-remaining"),
            reset_at: bounded_header(headers, "x-ratelimit-reset", 64),
            reset_after: duration_header(headers, "x-ratelimit-reset-after"),
            retry_after: duration_header(headers, RETRY_AFTER.as_str()),
        }
    }
}

fn scope(headers: &HeaderMap) -> RateLimitScope {
    let Some(value) = headers
        .get("x-ratelimit-scope")
        .and_then(|value| value.to_str().ok())
    else {
        return RateLimitScope::Unknown;
    };
    if value.eq_ignore_ascii_case("global") {
        RateLimitScope::Global
    } else if value.eq_ignore_ascii_case("user") {
        RateLimitScope::User
    } else if value.eq_ignore_ascii_case("shared") {
        RateLimitScope::Shared
    } else if value.eq_ignore_ascii_case("route") {
        RateLimitScope::Route
    } else {
        RateLimitScope::Unknown
    }
}

fn bool_header(headers: &HeaderMap, name: &str) -> Option<bool> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| match value.to_ascii_lowercase().as_str() {
            "true" | "1" => Some(true),
            "false" | "0" => Some(false),
            _ => None,
        })
}

fn unsigned_header(headers: &HeaderMap, name: &str) -> Option<u64> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
}

fn bounded_header(headers: &HeaderMap, name: &str, maximum: usize) -> Option<String> {
    let value = headers.get(name)?.to_str().ok()?;
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return None;
    }
    Some(value.to_owned())
}

fn duration_header(headers: &HeaderMap, name: &str) -> Option<Duration> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<f64>().ok())
        .and_then(bounded_duration)
}

fn bounded_duration(seconds: f64) -> Option<Duration> {
    if !seconds.is_finite() || seconds < 0.0 {
        return None;
    }
    Some(Duration::from_secs_f64(
        seconds.min(MAX_DISCORD_DIRECTED_WAIT.as_secs_f64()),
    ))
}
