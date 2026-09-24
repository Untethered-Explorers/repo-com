use serde::Serialize;
use serde_json::{Map, Value};

use crate::{AuditError, AuditResult};

/// The only value used in place of content or credential material.
pub const REDACTED: &str = "[REDACTED]";

/// A safe replacement for an unexpectedly credential-shaped object key.
pub const REDACTED_KEY: &str = "[REDACTED_KEY]";

const SECRET_TERMS: &[&str] = &[
    "authorization",
    "authheader",
    "authtoken",
    "bearer",
    "token",
    "secret",
    "password",
    "passwd",
    "privatekey",
    "credential",
    "cookie",
    "apikey",
    "accesskey",
    "clientsecret",
    "webhook",
    "sessionid",
    "refreshtoken",
    "signingkey",
    "oauth",
    "passphrase",
    "certificate",
    "jwt",
    "session",
    "dsn",
    "connectionstring",
    "bot",
];

const SECRET_ASSIGNMENT_TERMS: &[&str] = &[
    "private_key",
    "private-key",
    "api_key",
    "api-key",
    "access_key",
    "access-key",
    "client_secret",
    "client-secret",
    "secret_key",
    "secret-key",
    "auth_token",
    "bot_token",
    "discord_token",
    "x-api-key",
];

const CONTENT_TERMS: &[&str] = &[
    "message",
    "content",
    "body",
    "text",
    "prompt",
    "description",
    "payload",
    "raw",
    "html",
    "markdown",
];

const SAFE_KEYS: &[&str] = &[
    "active",
    "actor",
    "actorkind",
    "alias",
    "attempt",
    "attemptid",
    "attemptnumber",
    "category",
    "channel",
    "channelid",
    "code",
    "config",
    "confighash",
    "configurationhash",
    "contenthash",
    "count",
    "deleted",
    "destination",
    "destinationalias",
    "disabled",
    "draftid",
    "durationms",
    "enabled",
    "eventid",
    "eventtype",
    "exists",
    "expired",
    "httpstatus",
    "inbounditemid",
    "inbound",
    "kind",
    "level",
    "localonly",
    "metadata",
    "noncehash",
    "objectid",
    "objecttype",
    "operation",
    "operationid",
    "outcome",
    "policyhash",
    "provider",
    "reason",
    "read only",
    "readonly",
    "remote",
    "remoteMessageId",
    "repositoryid",
    "result",
    "revision",
    "schema",
    "schemaversion",
    "source",
    "state",
    "status",
    "transition",
    "truncated",
    "type",
    "updatedat",
    "version",
    "workspaceid",
];

/// Returns whether a field name is secret-like and therefore unsafe to expose.
#[must_use]
pub fn is_secret_like_field(field: &str) -> bool {
    let normalized = normalize_key(field);
    SECRET_TERMS.iter().any(|term| normalized.contains(term))
}

/// Redacts a JSON value for local audit metadata or diagnostics.
///
/// Object keys are classified before their values are visited. Unknown
/// scalar strings are removed rather than copied, because an arbitrary string
/// can be message content. Known safe metadata fields are retained, while
/// every sensitive or content-bearing field is replaced by [`REDACTED`].
#[must_use]
pub fn redact_metadata(value: &Value) -> Value {
    match value {
        Value::Object(fields) => {
            let mut redacted = Map::new();
            for (key, child) in fields {
                let value = match classify_key(key) {
                    KeyClass::Secret | KeyClass::Content => Value::String(REDACTED.to_owned()),
                    KeyClass::Safe => redact_value(child, true),
                    KeyClass::Unknown => redact_value(child, false),
                };
                redacted.insert(safe_key(key), value);
            }
            Value::Object(redacted)
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| redact_value(value, false))
                .collect(),
        ),
        Value::String(_) => Value::String(REDACTED.to_owned()),
        Value::Number(_) | Value::Bool(_) | Value::Null => value.clone(),
    }
}

/// Compatibility alias for [`redact_metadata`].
#[must_use]
pub fn redact_json(value: &Value) -> Value {
    redact_metadata(value)
}

/// Redacts metadata and fails closed unless the result remains a JSON object.
pub fn redact_metadata_checked(value: &Value) -> AuditResult<Value> {
    let redacted = redact_metadata(value);
    if redacted.is_object() {
        Ok(redacted)
    } else {
        Err(AuditError::RedactionFailed)
    }
}

/// Serializes a caller value to JSON and then applies the same redaction
/// boundary. This is useful for diagnostics assembled from structured values.
pub fn redact_serializable<T: Serialize>(value: &T) -> AuditResult<Value> {
    Ok(redact_metadata(&serde_json::to_value(value)?))
}

/// Redacts a single string without ever returning a recognized credential or
/// authorization value. Ordinary safe text is retained after control characters
/// are neutralized for safe diagnostic output.
#[must_use]
pub fn redact_text(value: &str) -> String {
    if contains_sensitive_text(value) {
        return REDACTED.to_owned();
    }

    value
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

#[derive(Clone, Copy)]
enum KeyClass {
    Secret,
    Content,
    Safe,
    Unknown,
}

fn classify_key(key: &str) -> KeyClass {
    let normalized = normalize_key(key);

    if SECRET_TERMS.iter().any(|term| normalized.contains(term)) {
        return KeyClass::Secret;
    }

    if is_safe_identifier_key(&normalized) || SAFE_KEYS.iter().any(|safe| normalized == *safe) {
        return KeyClass::Safe;
    }

    if CONTENT_TERMS.iter().any(|term| normalized.contains(term)) {
        return KeyClass::Content;
    }

    KeyClass::Unknown
}

fn is_safe_identifier_key(normalized: &str) -> bool {
    (normalized.ends_with("id") || normalized.ends_with("ids") || normalized.ends_with("hash"))
        && !SECRET_TERMS.iter().any(|term| normalized.contains(term))
}

fn redact_value(value: &Value, allow_safe_strings: bool) -> Value {
    match value {
        Value::Object(fields) => {
            let mut redacted = Map::new();
            for (key, child) in fields {
                let value = match classify_key(key) {
                    KeyClass::Secret | KeyClass::Content => Value::String(REDACTED.to_owned()),
                    KeyClass::Safe => redact_value(child, true),
                    KeyClass::Unknown => redact_value(child, false),
                };
                redacted.insert(safe_key(key), value);
            }
            Value::Object(redacted)
        }
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|value| redact_value(value, allow_safe_strings))
                .collect(),
        ),
        Value::String(value) if allow_safe_strings => Value::String(redact_text(value)),
        Value::String(_) => Value::String(REDACTED.to_owned()),
        Value::Number(_) | Value::Bool(_) | Value::Null => value.clone(),
    }
}

fn safe_key(key: &str) -> String {
    if key.chars().any(char::is_control) || contains_sensitive_text(key) {
        REDACTED_KEY.to_owned()
    } else {
        key.to_owned()
    }
}

fn normalize_key(key: &str) -> String {
    key.chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

fn contains_sensitive_text(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();

    if lower.contains("-----begin") || lower.contains("private key-----") {
        return true;
    }
    if lower.contains("authorization:")
        || lower.contains("authorization=")
        || lower.contains("bearer ")
        || lower.contains("basic ")
    {
        return true;
    }

    for term in SECRET_TERMS.iter().chain(SECRET_ASSIGNMENT_TERMS) {
        if has_assignment_after(&lower, term) {
            return true;
        }
    }

    if lower.contains("ghp_")
        || lower.contains("gho_")
        || lower.contains("ghs_")
        || lower.contains("ghu_")
        || lower.contains("github_pat_")
        || lower.contains("xoxb-")
        || lower.contains("xoxp-")
        || lower.contains("akia")
    {
        return true;
    }

    if lower.contains("bot ") {
        return true;
    }
    if has_discord_token_shape(&lower) || has_jwt_shape(&lower) {
        return true;
    }

    false
}

fn has_assignment_after(value: &str, term: &str) -> bool {
    let mut start = 0;
    while let Some(found) = value[start..].find(term) {
        let position = start + found;
        let suffix = &value[position + term.len()..];
        let trimmed = suffix.trim_start_matches(['"', '\'']);
        if trimmed.starts_with([':', '=', ' ']) {
            let after_space = trimmed.trim_start_matches([':', '=', ' ', '\t']);
            if trimmed.starts_with(' ')
                || trimmed.starts_with('\t')
                || after_space.len() < trimmed.len()
            {
                return true;
            }
        }
        start = position + term.len();
    }
    false
}

fn has_discord_token_shape(value: &str) -> bool {
    let bytes = value.as_bytes();
    for start in 0..bytes.len() {
        if !value.is_char_boundary(start) {
            continue;
        }
        if start > 0 && is_token_byte(bytes[start - 1]) {
            continue;
        }
        let Some(first_end) = bytes[start..]
            .iter()
            .position(|byte| *byte == b'.')
            .map(|position| start + position)
        else {
            continue;
        };
        let Some(second_end) = bytes[first_end + 1..]
            .iter()
            .position(|byte| *byte == b'.')
            .map(|position| first_end + 1 + position)
        else {
            continue;
        };
        let first = &value[start..first_end];
        let second = &value[first_end + 1..second_end];
        let third_start = second_end + 1;
        let third_end = bytes[third_start..]
            .iter()
            .position(|byte| !is_token_byte(*byte))
            .map(|position| third_start + position)
            .unwrap_or(bytes.len());
        let third = &value[third_start..third_end];
        if first.len() >= 8
            && second.len() >= 3
            && third.len() >= 8
            && first.chars().all(is_token_character)
            && second.chars().all(is_token_character)
            && third.chars().all(is_token_character)
        {
            return true;
        }
    }
    false
}

fn has_jwt_shape(value: &str) -> bool {
    let parts = value.split('.').collect::<Vec<_>>();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| part.len() >= 8 && part.chars().all(is_token_character))
        && parts[0].starts_with("eyj")
}

fn is_token_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
}

fn is_token_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')
}
