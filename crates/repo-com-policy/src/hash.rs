use std::fmt;

use repo_com_config::{AutoSendEntry, RepositoryConfig};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The stable hash algorithm used for configuration and policy bindings.
pub const HASH_ALGORITHM: &str = "sha256";
/// The number of hexadecimal characters in a SHA-256 digest.
pub const HASH_HEX_LENGTH: usize = 64;

/// The exact event, destination alias, and severity tuple used by policy.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct PolicyTuple {
    /// Exact event type.
    pub event_type: String,
    /// Exact repository-local destination alias.
    pub destination_alias: String,
    /// Exact severity value.
    pub severity: String,
}

impl PolicyTuple {
    /// Creates an exact tuple without normalizing or broadening any component.
    #[must_use]
    pub fn new(
        event_type: impl Into<String>,
        destination_alias: impl Into<String>,
        severity: impl Into<String>,
    ) -> Self {
        Self {
            event_type: event_type.into(),
            destination_alias: destination_alias.into(),
            severity: severity.into(),
        }
    }

    /// Creates a tuple from a configuration entry.
    #[must_use]
    pub fn from_entry(entry: &AutoSendEntry) -> Self {
        Self::new(
            entry.event_type.clone(),
            entry.destination.clone(),
            entry.severity.clone(),
        )
    }

    /// Returns the exact destination alias.
    #[must_use]
    pub fn destination(&self) -> &str {
        &self.destination_alias
    }

    /// Returns the exact destination alias using the state-layer spelling.
    #[must_use]
    pub fn destination_alias(&self) -> &str {
        &self.destination_alias
    }

    /// Returns the exact event type.
    #[must_use]
    pub fn event(&self) -> &str {
        &self.event_type
    }

    /// Returns the exact severity.
    #[must_use]
    pub fn severity(&self) -> &str {
        &self.severity
    }

    /// Returns whether this tuple is a valid exact policy value.
    ///
    /// Wildcard characters are rejected rather than interpreted. Prefixes and
    /// severity labels are not expanded: callers still use ordinary equality.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        is_exact_component(&self.event_type)
            && is_exact_component(&self.destination_alias)
            && is_exact_component(&self.severity)
    }

    /// Validates that no component contains a wildcard or ambiguous syntax.
    pub fn validate(&self) -> Result<(), PolicyTupleError> {
        for (field, value) in [
            ("event_type", &self.event_type),
            ("destination_alias", &self.destination_alias),
            ("severity", &self.severity),
        ] {
            if value.is_empty() {
                return Err(PolicyTupleError::EmptyField { field });
            }
            if value.chars().any(is_wildcard_character) {
                return Err(PolicyTupleError::WildcardField { field });
            }
            if value
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
            {
                return Err(PolicyTupleError::AmbiguousCharacter { field });
            }
        }
        Ok(())
    }

    /// Returns the fixed-order canonical JSON representation of this tuple.
    #[must_use]
    pub fn canonical_json(&self) -> String {
        serde_json::to_string(self).expect("PolicyTuple contains serializable strings")
    }

    /// Returns the fixed-order canonical bytes used for the tuple hash.
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("PolicyTuple contains serializable strings")
    }

    /// Returns the deterministic SHA-256 hash of this exact tuple.
    #[must_use]
    pub fn canonical_hash(&self) -> String {
        sha256_hex(&self.canonical_bytes())
    }

    /// Returns the raw SHA-256 digest of this exact tuple.
    #[must_use]
    pub fn canonical_hash_bytes(&self) -> [u8; 32] {
        Sha256::digest(self.canonical_bytes()).into()
    }

    /// Returns whether two tuples are exactly equal.
    #[must_use]
    pub fn matches_exact(&self, other: &Self) -> bool {
        self == other
    }
}

impl From<&AutoSendEntry> for PolicyTuple {
    fn from(entry: &AutoSendEntry) -> Self {
        Self::from_entry(entry)
    }
}

impl From<AutoSendEntry> for PolicyTuple {
    fn from(entry: AutoSendEntry) -> Self {
        Self::new(entry.event_type, entry.destination, entry.severity)
    }
}

impl fmt::Display for PolicyTuple {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}/{}/{}",
            self.event_type, self.destination_alias, self.severity
        )
    }
}

/// A safe validation error for an exact policy tuple.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyTupleError {
    /// One required component was empty.
    EmptyField {
        /// Safe component name.
        field: &'static str,
    },
    /// One component contained a wildcard or pattern character.
    WildcardField {
        /// Safe component name.
        field: &'static str,
    },
    /// One component contained whitespace, control characters, or pattern
    /// delimiters that could make a tuple ambiguous.
    AmbiguousCharacter {
        /// Safe component name.
        field: &'static str,
    },
}

impl fmt::Display for PolicyTupleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyField { field } => write!(formatter, "policy tuple field {field} is empty"),
            Self::WildcardField { field } => {
                write!(formatter, "policy tuple field {field} contains a wildcard")
            }
            Self::AmbiguousCharacter { field } => {
                write!(formatter, "policy tuple field {field} is ambiguous")
            }
        }
    }
}

impl std::error::Error for PolicyTupleError {}

/// The two hashes that bind an activation to current state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PolicyHashes {
    /// Hash of the complete normalized repository configuration.
    pub config_hash: String,
    /// Hash of the exact event/destination/severity tuple.
    pub tuple_hash: String,
}

impl PolicyHashes {
    /// Computes both hashes for a configuration and tuple.
    #[must_use]
    pub fn for_config_tuple(config: &RepositoryConfig, tuple: &PolicyTuple) -> Self {
        Self {
            config_hash: canonical_config_hash(config),
            tuple_hash: canonical_tuple_hash(tuple),
        }
    }

    /// Returns whether both hashes have the expected SHA-256 representation.
    #[must_use]
    pub fn are_well_formed(&self) -> bool {
        is_sha256_hex(&self.config_hash) && is_sha256_hex(&self.tuple_hash)
    }
}

/// Returns the normalized canonical JSON bytes for the complete configuration.
///
/// The configuration crate owns schema normalization. This wrapper deliberately
/// hashes that complete normalized document, including destinations, mentions,
/// inbound settings, retention, and all auto-send entries; it never hashes a
/// source path or raw TOML formatting.
#[must_use]
pub fn canonical_config_bytes(config: &RepositoryConfig) -> Vec<u8> {
    config
        .normalized()
        .canonical_bytes()
        .expect("RepositoryConfig contains serializable primitive values")
}

/// Returns the deterministic SHA-256 hash of the complete normalized config.
#[must_use]
pub fn canonical_config_hash(config: &RepositoryConfig) -> String {
    sha256_hex(&canonical_config_bytes(config))
}

/// Returns the raw SHA-256 digest of the complete normalized config.
#[must_use]
pub fn canonical_config_hash_bytes(config: &RepositoryConfig) -> [u8; 32] {
    Sha256::digest(canonical_config_bytes(config)).into()
}

/// Returns the canonical hash of one exact policy tuple.
#[must_use]
pub fn canonical_tuple_hash(tuple: &PolicyTuple) -> String {
    tuple.canonical_hash()
}

/// Returns whether a value is a lowercase hexadecimal SHA-256 digest.
#[must_use]
pub fn is_sha256_hex(value: &str) -> bool {
    value.len() == HASH_HEX_LENGTH
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(HASH_HEX_LENGTH);
    for byte in digest {
        use std::fmt::Write;
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

fn is_exact_component(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            !character.is_control()
                && !character.is_whitespace()
                && !is_wildcard_character(character)
        })
}

fn is_wildcard_character(character: char) -> bool {
    matches!(character, '*' | '?' | '[' | ']' | '{' | '}')
}
