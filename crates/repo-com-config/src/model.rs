use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The only repository configuration schema version supported by this crate.
pub const SCHEMA_VERSION: u32 = 1;

/// A validated, repository-local configuration document.
///
/// The maps are intentionally `BTreeMap`s. They make alias iteration and the
/// canonical representation deterministic without relying on insertion order in
/// a TOML document.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryConfig {
    /// The schema version. Only version 1 is accepted by the resolver.
    pub schema_version: u32,
    /// Stable repository identity used by later local-state components.
    pub repository_id: String,
    /// The single Discord workspace associated with this repository.
    pub discord: DiscordConfig,
    /// Named outbound destinations.
    pub destinations: BTreeMap<String, DestinationConfig>,
    /// Named role/user mention targets.
    pub mentions: BTreeMap<String, MentionConfig>,
    /// Named inbound channel aliases.
    pub inbound: BTreeMap<String, InboundConfig>,
    /// Local retention settings.
    pub retention: RetentionConfig,
    /// Exact, non-activated auto-send tuples.
    pub auto_send: Vec<AutoSendEntry>,
}

/// Compatibility name for callers that use the shorter configuration name.
pub type Config = RepositoryConfig;

/// Discord identity fields that are safe to commit in repository configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiscordConfig {
    /// The one configured Discord workspace (guild) identifier.
    pub workspace_id: String,
}

/// A named outbound destination. The name is the workflow-facing identifier;
/// callers resolve it to exactly one channel before sending.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DestinationConfig {
    /// The configured channel identifier.
    pub channel_id: String,
    /// Repository-local mention aliases allowed for this destination.
    pub allowed_mentions: Vec<String>,
}

/// A named mention target. The string is deliberately prefixed so a raw role
/// or user identifier cannot be used as a normal workflow value.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MentionConfig {
    /// A `role:` or `user:` target identifier.
    pub target: String,
}

/// An inbound alias. The alias names a channel through the matching outbound
/// destination; it never carries a raw channel identifier.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InboundConfig {
    /// Whether inbound retrieval is enabled for this alias.
    pub enabled: bool,
}

/// Local retention periods in days.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionConfig {
    /// Number of days to retain message content locally.
    pub content_days: u32,
    /// Number of days to retain non-content metadata locally.
    pub metadata_days: u32,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            content_days: 30,
            metadata_days: 365,
        }
    }
}

/// One exact auto-send policy tuple. This crate only parses and validates the
/// tuple; activation and send eligibility belong to other tasks.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AutoSendEntry {
    /// Exact event type matched by policy.
    pub event_type: String,
    /// Destination alias, never a raw channel identifier.
    pub destination: String,
    /// Exact severity matched by policy.
    pub severity: String,
}

/// The two supported mention target classes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MentionKind {
    /// A Discord role target.
    Role,
    /// A Discord user target.
    User,
}

impl MentionKind {
    /// Returns the wire prefix for this mention kind.
    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Role => "role",
            Self::User => "user",
        }
    }
}

/// A parsed mention target.
#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct MentionTarget {
    /// Whether the target is a role or user.
    pub kind: MentionKind,
    /// The identifier following the `role:` or `user:` prefix.
    pub id: String,
}

impl MentionTarget {
    /// Parses a target and rejects empty, whitespace-containing, or additional
    /// prefixes. Identifier syntax is intentionally limited to a portable,
    /// non-secret character set; the Discord adapter performs remote checks.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        let (prefix, id) = raw.split_once(':')?;
        if id.contains(':') || !is_portable_identifier(id) {
            return None;
        }
        let kind = match prefix {
            "role" => MentionKind::Role,
            "user" => MentionKind::User,
            _ => return None,
        };
        Some(Self {
            kind,
            id: id.to_owned(),
        })
    }

    /// Returns the canonical `role:<id>` or `user:<id>` representation.
    #[must_use]
    pub fn as_prefixed(&self) -> String {
        format!("{}:{}", self.kind.prefix(), self.id)
    }

    /// Returns the target kind.
    #[must_use]
    pub const fn kind(&self) -> MentionKind {
        self.kind
    }

    /// Returns the target identifier without its prefix.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }
}

impl std::fmt::Display for MentionTarget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.as_prefixed())
    }
}

impl RepositoryConfig {
    /// Returns a deterministic copy with mention lists and policy tuples
    /// sorted by their canonical fields.
    #[must_use]
    pub fn normalized(&self) -> Self {
        let mut normalized = self.clone();
        for destination in normalized.destinations.values_mut() {
            destination.allowed_mentions.sort();
        }
        normalized.auto_send.sort_by(|left, right| {
            (&left.event_type, &left.destination, &left.severity).cmp(&(
                &right.event_type,
                &right.destination,
                &right.severity,
            ))
        });
        normalized
    }

    /// Returns the compact canonical JSON representation used for hashing.
    pub fn canonical_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.normalized())
    }

    /// Returns canonical JSON bytes used for hashing and later policy work.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(&self.normalized())
    }

    /// Returns a deterministic SHA-256 hash of the normalized configuration.
    #[must_use]
    pub fn canonical_hash(&self) -> String {
        let bytes = self
            .canonical_bytes()
            .expect("RepositoryConfig contains only serializable primitive values");
        let digest = Sha256::digest(bytes);
        digest.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    /// Returns the raw SHA-256 digest of the normalized configuration.
    #[must_use]
    pub fn canonical_hash_bytes(&self) -> [u8; 32] {
        let bytes = self
            .canonical_bytes()
            .expect("RepositoryConfig contains only serializable primitive values");
        Sha256::digest(bytes).into()
    }

    /// Compatibility alias for [`RepositoryConfig::canonical_hash`].
    #[must_use]
    pub fn config_hash(&self) -> String {
        self.canonical_hash()
    }
}

/// Compatibility alias for [`DiscordConfig`].
pub type DiscordWorkspace = DiscordConfig;
/// Compatibility alias for [`DestinationConfig`].
pub type Destination = DestinationConfig;
/// Compatibility alias for [`MentionConfig`].
pub type Mention = MentionConfig;
/// Compatibility alias for [`InboundConfig`].
pub type Inbound = InboundConfig;
/// Compatibility alias for [`AutoSendEntry`].
pub type AutoSend = AutoSendEntry;
/// Compatibility alias for [`AutoSendEntry`].
pub type AutoSendConfig = AutoSendEntry;
/// Compatibility alias for [`RetentionConfig`].
pub type RetentionPolicy = RetentionConfig;
/// Compatibility alias for [`DiscordConfig`].
pub type WorkspaceConfig = DiscordConfig;

fn is_portable_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
}

/// Returns whether a string is a portable non-empty identifier used by the
/// configuration model. It does not inspect or classify secret values.
#[must_use]
pub fn is_valid_identifier(value: &str) -> bool {
    is_portable_identifier(value)
}

#[cfg(test)]
mod tests {
    use super::{MentionKind, MentionTarget};

    #[test]
    fn mention_target_requires_an_exact_supported_prefix() {
        assert_eq!(
            MentionTarget::parse("role:123"),
            Some(MentionTarget {
                kind: MentionKind::Role,
                id: "123".to_owned(),
            })
        );
        assert!(MentionTarget::parse("channel:123").is_none());
        assert!(MentionTarget::parse("role:").is_none());
        assert!(MentionTarget::parse("role:123:extra").is_none());
    }
}
