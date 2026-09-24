use std::collections::{BTreeMap, BTreeSet};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use toml::Value;

use crate::model::{MentionTarget, RepositoryConfig, SCHEMA_VERSION};

/// The filesystem operation that failed while loading configuration.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum IoOperation {
    /// Reading the configuration bytes.
    Read,
    /// Normalizing or canonicalizing a path.
    Normalize,
    /// Inspecting a path.
    Metadata,
    /// Determining the current working directory.
    CurrentDirectory,
    /// Finding the repository root.
    RepositoryRoot,
}

/// Broad TOML syntax failure classes. The original parser text is deliberately
/// not retained in this public error type.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TomlErrorKind {
    /// The document is not valid TOML.
    Syntax,
    /// TOML rejected a repeated key or table.
    DuplicateKey,
}

/// A safe, typed configuration error.
///
/// Every variant contains only paths, field paths, counts, or fixed classes.
/// No TOML source text, parsed scalar value, or remote response is retained.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "code")]
pub enum ConfigError {
    /// A filesystem operation failed.
    #[serde(rename = "io")]
    Io {
        /// Path involved in the failed operation.
        path: PathBuf,
        /// Safe operation class.
        operation: IoOperation,
    },
    /// No `.repo-com.toml` was found in the bounded search range.
    #[serde(rename = "config-not-found")]
    NoCandidates {
        /// Directory where the search started.
        start: PathBuf,
        /// Inclusive lower boundary of the search.
        repository_root: PathBuf,
    },
    /// More than one `.repo-com.toml` was found in the bounded search range.
    #[serde(rename = "multiple-config-candidates")]
    MultipleCandidates {
        /// Directory where the search started.
        start: PathBuf,
        /// Inclusive lower boundary of the search.
        repository_root: PathBuf,
        /// All candidates, in deterministic path order.
        candidates: Vec<PathBuf>,
    },
    /// No repository root marker was found.
    #[serde(rename = "repository-root-not-found")]
    RepositoryRootNotFound {
        /// Starting path supplied to root detection.
        start: PathBuf,
    },
    /// An explicit path or search start escaped the supplied repository root.
    #[serde(rename = "path-outside-repository")]
    PathOutsideRepository {
        /// Path that was outside the root.
        path: PathBuf,
        /// Expected repository root.
        repository_root: PathBuf,
    },
    /// TOML syntax was invalid.
    #[serde(rename = "invalid-toml")]
    InvalidToml {
        /// File being parsed.
        path: PathBuf,
        /// One-based line when available.
        line: Option<usize>,
        /// One-based column when available.
        column: Option<usize>,
        /// Sanitized syntax class.
        kind: TomlErrorKind,
    },
    /// A required schema field was absent.
    #[serde(rename = "missing-field")]
    MissingField {
        /// File being parsed.
        path: PathBuf,
        /// Safe field path.
        field: String,
    },
    /// A field had the wrong TOML type.
    #[serde(rename = "invalid-field-type")]
    InvalidFieldType {
        /// File being parsed.
        path: PathBuf,
        /// Safe field path.
        field: String,
    },
    /// The schema version is not the supported version 1.
    #[serde(rename = "unsupported-schema-version")]
    UnsupportedSchemaVersion {
        /// File being parsed.
        path: PathBuf,
        /// Numeric version read from the document.
        version: i64,
    },
    /// A field is not part of the strict schema.
    #[serde(rename = "unknown-field")]
    UnknownField {
        /// File being parsed.
        path: PathBuf,
        /// Safe field path.
        field: String,
    },
    /// A secret-like key was found. Its key and value are not echoed.
    #[serde(rename = "secret-field")]
    SecretField {
        /// File being parsed.
        path: PathBuf,
        /// Redacted field class/path.
        field: String,
    },
    /// A raw destination field was found where an alias is required.
    #[serde(rename = "raw-destination-field")]
    RawDestinationField {
        /// File being parsed.
        path: PathBuf,
        /// Safe field path.
        field: String,
    },
    /// Repository identity is empty or malformed.
    #[serde(rename = "invalid-repository-id")]
    InvalidRepositoryId {
        /// File being validated.
        path: PathBuf,
        /// Safe field path.
        field: String,
    },
    /// Workspace identity is empty or malformed.
    #[serde(rename = "invalid-workspace-id")]
    InvalidWorkspaceId {
        /// File being validated.
        path: PathBuf,
        /// Safe field path.
        field: String,
    },
    /// A channel identifier is empty or malformed.
    #[serde(rename = "invalid-channel-id")]
    InvalidChannelId {
        /// File being validated.
        path: PathBuf,
        /// Safe field path.
        field: String,
    },
    /// An alias name is empty or malformed.
    #[serde(rename = "invalid-alias")]
    InvalidAlias {
        /// File being validated.
        path: PathBuf,
        /// Safe field path.
        field: String,
    },
    /// An alias or allowlist entry occurs more than once.
    #[serde(rename = "duplicate-alias")]
    DuplicateAlias {
        /// File being validated.
        path: PathBuf,
        /// Safe field path.
        field: String,
    },
    /// A mention definition omitted its target.
    #[serde(rename = "missing-mention-target")]
    MissingMentionTarget {
        /// File being validated.
        path: PathBuf,
        /// Safe field path.
        field: String,
    },
    /// A mention target has an unsupported prefix or malformed identifier.
    #[serde(rename = "invalid-mention-target")]
    InvalidMentionTarget {
        /// File being validated.
        path: PathBuf,
        /// Safe field path.
        field: String,
    },
    /// A destination allowlist names a missing mention definition.
    #[serde(rename = "missing-mention-alias")]
    MissingMentionAlias {
        /// File being validated.
        path: PathBuf,
        /// Safe field path.
        field: String,
    },
    /// An exact auto-send tuple is repeated.
    #[serde(rename = "duplicate-auto-send")]
    DuplicateAutoSend {
        /// File being validated.
        path: PathBuf,
        /// Safe field path.
        field: String,
    },
    /// An auto-send entry names a destination alias that is not configured.
    #[serde(rename = "unknown-destination-alias")]
    UnknownDestinationAlias {
        /// File being validated.
        path: PathBuf,
        /// Safe field path.
        field: String,
    },
    /// An inbound alias does not name a configured destination channel.
    #[serde(rename = "unconfigured-inbound-alias")]
    UnconfiguredInboundAlias {
        /// File being validated.
        path: PathBuf,
        /// Safe field path.
        field: String,
    },
    /// An auto-send entry is empty or otherwise structurally invalid.
    #[serde(rename = "invalid-auto-send")]
    InvalidAutoSend {
        /// File being validated.
        path: PathBuf,
        /// Safe field path.
        field: String,
    },
    /// A retention value is outside the accepted positive range.
    #[serde(rename = "invalid-retention")]
    InvalidRetention {
        /// File being validated.
        path: PathBuf,
        /// Safe field path.
        field: String,
    },
    /// A configured reference points at a different workspace.
    #[serde(rename = "cross-workspace-reference")]
    CrossWorkspaceReference {
        /// File or in-memory document being validated.
        path: PathBuf,
        /// Safe field path.
        field: String,
    },
}

/// Compatibility name for callers that use the longer error type name.
pub type ConfigurationError = ConfigError;

impl ConfigError {
    /// Returns the stable machine-readable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Io { .. } => "io",
            Self::NoCandidates { .. } => "config-not-found",
            Self::MultipleCandidates { .. } => "multiple-config-candidates",
            Self::RepositoryRootNotFound { .. } => "repository-root-not-found",
            Self::PathOutsideRepository { .. } => "path-outside-repository",
            Self::InvalidToml { .. } => "invalid-toml",
            Self::MissingField { .. } => "missing-field",
            Self::InvalidFieldType { .. } => "invalid-field-type",
            Self::UnsupportedSchemaVersion { .. } => "unsupported-schema-version",
            Self::UnknownField { .. } => "unknown-field",
            Self::SecretField { .. } => "secret-field",
            Self::RawDestinationField { .. } => "raw-destination-field",
            Self::InvalidRepositoryId { .. } => "invalid-repository-id",
            Self::InvalidWorkspaceId { .. } => "invalid-workspace-id",
            Self::InvalidChannelId { .. } => "invalid-channel-id",
            Self::InvalidAlias { .. } => "invalid-alias",
            Self::DuplicateAlias { .. } => "duplicate-alias",
            Self::MissingMentionTarget { .. } => "missing-mention-target",
            Self::InvalidMentionTarget { .. } => "invalid-mention-target",
            Self::MissingMentionAlias { .. } => "missing-mention-alias",
            Self::DuplicateAutoSend { .. } => "duplicate-auto-send",
            Self::UnknownDestinationAlias { .. } => "unknown-destination-alias",
            Self::UnconfiguredInboundAlias { .. } => "unconfigured-inbound-alias",
            Self::InvalidAutoSend { .. } => "invalid-auto-send",
            Self::InvalidRetention { .. } => "invalid-retention",
            Self::CrossWorkspaceReference { .. } => "cross-workspace-reference",
        }
    }

    /// Returns the safe human-readable explanation without interpolated source
    /// values.
    #[must_use]
    pub fn message(&self) -> &'static str {
        match self {
            Self::Io { .. } => "configuration filesystem operation failed",
            Self::NoCandidates { .. } => "no repository configuration was found",
            Self::MultipleCandidates { .. } => "multiple repository configurations were found",
            Self::RepositoryRootNotFound { .. } => "repository root could not be detected",
            Self::PathOutsideRepository { .. } => "configuration path is outside the repository",
            Self::InvalidToml { .. } => "configuration is not valid TOML",
            Self::MissingField { .. } => "required configuration field is missing",
            Self::InvalidFieldType { .. } => "configuration field has an invalid type",
            Self::UnsupportedSchemaVersion { .. } => "configuration schema version is unsupported",
            Self::UnknownField { .. } => "configuration field is not allowed",
            Self::SecretField { .. } => "secret-like configuration fields are not allowed",
            Self::RawDestinationField { .. } => "raw destination fields are not allowed",
            Self::InvalidRepositoryId { .. } => "repository identity is invalid",
            Self::InvalidWorkspaceId { .. } => "workspace identity is invalid",
            Self::InvalidChannelId { .. } => "channel identity is invalid",
            Self::InvalidAlias { .. } => "configuration alias is invalid",
            Self::DuplicateAlias { .. } => "configuration alias is duplicated",
            Self::MissingMentionTarget { .. } => "mention target is missing",
            Self::InvalidMentionTarget { .. } => "mention target is invalid",
            Self::MissingMentionAlias { .. } => "destination references a missing mention alias",
            Self::DuplicateAutoSend { .. } => "exact auto-send tuple is duplicated",
            Self::UnknownDestinationAlias { .. } => {
                "auto-send references an unknown destination alias"
            }
            Self::UnconfiguredInboundAlias { .. } => {
                "inbound alias does not name a configured channel"
            }
            Self::InvalidAutoSend { .. } => "auto-send entry is invalid",
            Self::InvalidRetention { .. } => "retention value is invalid",
            Self::CrossWorkspaceReference { .. } => "configuration references another workspace",
        }
    }

    /// Returns the file path associated with the error, when one exists.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Io { path, .. }
            | Self::NoCandidates { start: path, .. }
            | Self::MultipleCandidates { start: path, .. }
            | Self::PathOutsideRepository { path, .. }
            | Self::InvalidToml { path, .. }
            | Self::MissingField { path, .. }
            | Self::InvalidFieldType { path, .. }
            | Self::UnsupportedSchemaVersion { path, .. }
            | Self::UnknownField { path, .. }
            | Self::SecretField { path, .. }
            | Self::RawDestinationField { path, .. }
            | Self::InvalidRepositoryId { path, .. }
            | Self::InvalidWorkspaceId { path, .. }
            | Self::InvalidChannelId { path, .. }
            | Self::InvalidAlias { path, .. }
            | Self::DuplicateAlias { path, .. }
            | Self::MissingMentionTarget { path, .. }
            | Self::InvalidMentionTarget { path, .. }
            | Self::MissingMentionAlias { path, .. }
            | Self::DuplicateAutoSend { path, .. }
            | Self::UnknownDestinationAlias { path, .. }
            | Self::UnconfiguredInboundAlias { path, .. }
            | Self::InvalidAutoSend { path, .. }
            | Self::InvalidRetention { path, .. }
            | Self::CrossWorkspaceReference { path, .. } => Some(path),
            Self::RepositoryRootNotFound { start, .. } => Some(start),
        }
    }

    /// Returns the safe field path associated with the error, when one exists.
    #[must_use]
    pub fn field_path(&self) -> Option<&str> {
        match self {
            Self::MissingField { field, .. }
            | Self::InvalidFieldType { field, .. }
            | Self::UnknownField { field, .. }
            | Self::SecretField { field, .. }
            | Self::RawDestinationField { field, .. }
            | Self::InvalidRepositoryId { field, .. }
            | Self::InvalidWorkspaceId { field, .. }
            | Self::InvalidChannelId { field, .. }
            | Self::InvalidAlias { field, .. }
            | Self::DuplicateAlias { field, .. }
            | Self::MissingMentionTarget { field, .. }
            | Self::InvalidMentionTarget { field, .. }
            | Self::MissingMentionAlias { field, .. }
            | Self::DuplicateAutoSend { field, .. }
            | Self::UnknownDestinationAlias { field, .. }
            | Self::UnconfiguredInboundAlias { field, .. }
            | Self::InvalidAutoSend { field, .. }
            | Self::InvalidRetention { field, .. }
            | Self::CrossWorkspaceReference { field, .. } => Some(field),
            _ => None,
        }
    }

    /// Serializes this error as a safe protocol JSON value with a stable code,
    /// message, and optional path metadata.
    pub fn to_protocol_value(&self) -> Result<serde_json::Value, serde_json::Error> {
        let mut object = serde_json::Map::new();
        object.insert(
            "code".to_owned(),
            serde_json::Value::String(self.code().to_owned()),
        );
        object.insert(
            "message".to_owned(),
            serde_json::Value::String(self.message().to_owned()),
        );
        if let Some(path) = self.path() {
            object.insert(
                "path".to_owned(),
                serde_json::Value::String(path.to_string_lossy().into_owned()),
            );
        }
        match self {
            Self::NoCandidates {
                repository_root, ..
            }
            | Self::MultipleCandidates {
                repository_root, ..
            }
            | Self::PathOutsideRepository {
                repository_root, ..
            } => {
                object.insert(
                    "repository_root".to_owned(),
                    serde_json::Value::String(repository_root.to_string_lossy().into_owned()),
                );
            }
            _ => {}
        }
        if let Some(field) = self.field_path() {
            object.insert(
                "field".to_owned(),
                serde_json::Value::String(field.to_owned()),
            );
        }
        if let Self::InvalidToml { line, column, .. } = self {
            if let Some(line) = line {
                object.insert("line".to_owned(), serde_json::Value::from(*line));
            }
            if let Some(column) = column {
                object.insert("column".to_owned(), serde_json::Value::from(*column));
            }
        }
        if let Self::MultipleCandidates { candidates, .. } = self {
            object.insert(
                "candidates".to_owned(),
                serde_json::Value::Array(
                    candidates
                        .iter()
                        .map(|path| serde_json::Value::String(path.to_string_lossy().into_owned()))
                        .collect(),
                ),
            );
        }
        Ok(serde_json::Value::Object(object))
    }

    /// Serializes this error as compact protocol JSON.
    pub fn to_protocol_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.to_protocol_value()?)
    }

    /// Compatibility alias for [`ConfigError::code`].
    #[must_use]
    pub fn error_code(&self) -> &'static str {
        self.code()
    }

    /// Compatibility alias for [`ConfigError::code`].
    #[must_use]
    pub fn stable_code(&self) -> &'static str {
        self.code()
    }

    /// Compatibility alias for [`ConfigError::to_protocol_json`].
    pub fn protocol_json(&self) -> Result<String, serde_json::Error> {
        self.to_protocol_json()
    }

    /// Maps the safe configuration error onto the foundation's stable
    /// usage/schema category for command layers.
    #[must_use]
    pub fn to_repo_com_error(&self) -> repo_com_foundation::RepoComError {
        let location = self
            .path()
            .map(|path| format!(" at {}", path.display()))
            .unwrap_or_default();
        repo_com_foundation::RepoComError::usage(format!(
            "configuration {}: {}{}",
            self.code(),
            self.message(),
            location
        ))
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code(), self.message())?;
        if let Some(path) = self.path() {
            write!(formatter, " ({})", path.display())?;
        }
        if let Some(field) = self.field_path() {
            write!(formatter, " at {field}")?;
        }
        Ok(())
    }
}

impl Error for ConfigError {}

/// Optional local index used to verify that configured Discord references are
/// in the same workspace. The configuration crate never calls Discord; callers
/// may populate this index from a separately validated setup result.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WorkspaceReferenceIndex {
    channels: BTreeMap<String, String>,
    mentions: BTreeMap<String, String>,
}

/// Compatibility name for a local workspace reference catalog.
pub type WorkspaceCatalog = WorkspaceReferenceIndex;

impl WorkspaceReferenceIndex {
    /// Creates an empty reference index.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a channel-to-workspace relation and returns the updated index.
    #[must_use]
    pub fn with_channel(
        mut self,
        channel_id: impl Into<String>,
        workspace_id: impl Into<String>,
    ) -> Self {
        self.channels.insert(channel_id.into(), workspace_id.into());
        self
    }

    /// Adds or replaces a channel-to-workspace relation in an existing index.
    pub fn insert_channel(
        &mut self,
        channel_id: impl Into<String>,
        workspace_id: impl Into<String>,
    ) -> &mut Self {
        self.channels.insert(channel_id.into(), workspace_id.into());
        self
    }

    /// Adds a mention-to-workspace relation and returns the updated index.
    #[must_use]
    pub fn with_mention(
        mut self,
        target: impl Into<String>,
        workspace_id: impl Into<String>,
    ) -> Self {
        self.mentions.insert(target.into(), workspace_id.into());
        self
    }

    /// Adds or replaces a mention-to-workspace relation in an existing index.
    pub fn insert_mention(
        &mut self,
        target: impl Into<String>,
        workspace_id: impl Into<String>,
    ) -> &mut Self {
        self.mentions.insert(target.into(), workspace_id.into());
        self
    }

    /// Returns a known channel workspace, if one was indexed.
    #[must_use]
    pub fn channel_workspace(&self, channel_id: &str) -> Option<&str> {
        self.channels.get(channel_id).map(String::as_str)
    }

    /// Returns a known mention workspace, if one was indexed.
    #[must_use]
    pub fn mention_workspace(&self, target: &MentionTarget) -> Option<&str> {
        self.mentions
            .get(&target.as_prefixed())
            .or_else(|| self.mentions.get(target.id()))
            .map(String::as_str)
    }
}

/// Parses and fully validates one TOML document without performing any I/O.
pub fn parse_config(path: &Path, source: &str) -> Result<RepositoryConfig, ConfigError> {
    let value =
        toml::from_str::<Value>(source).map_err(|error| toml_error(path, source, &error))?;
    validate_document_shape(&value, path)?;
    let config =
        value
            .try_into::<RepositoryConfig>()
            .map_err(|_error| ConfigError::InvalidFieldType {
                path: path.to_path_buf(),
                field: "<document>".to_owned(),
            })?;
    validate_config(&config, path)?;
    Ok(config)
}

/// Compatibility alias for [`parse_config`].
pub fn parse_config_str(path: &Path, source: &str) -> Result<RepositoryConfig, ConfigError> {
    parse_config(path, source)
}

/// Validates a model that was constructed by a caller or another decoder.
pub fn validate_config(config: &RepositoryConfig, path: &Path) -> Result<(), ConfigError> {
    if config.schema_version != SCHEMA_VERSION {
        return Err(ConfigError::UnsupportedSchemaVersion {
            path: path.to_path_buf(),
            version: i64::from(config.schema_version),
        });
    }
    if !is_valid_repository_id(&config.repository_id) {
        return Err(ConfigError::InvalidRepositoryId {
            path: path.to_path_buf(),
            field: "repository_id".to_owned(),
        });
    }
    if !is_valid_identifier(&config.discord.workspace_id) {
        return Err(ConfigError::InvalidWorkspaceId {
            path: path.to_path_buf(),
            field: "discord.workspace_id".to_owned(),
        });
    }

    for (alias, destination) in &config.destinations {
        let alias_field = format!("destinations.{alias}");
        validate_alias(path, &alias_field, alias)?;
        if !is_valid_identifier(&destination.channel_id) {
            return Err(ConfigError::InvalidChannelId {
                path: path.to_path_buf(),
                field: format!("{alias_field}.channel_id"),
            });
        }
        let mut seen = BTreeSet::new();
        for (index, mention_alias) in destination.allowed_mentions.iter().enumerate() {
            let field = format!("{alias_field}.allowed_mentions[{index}]");
            if !seen.insert(mention_alias) {
                return Err(ConfigError::DuplicateAlias {
                    path: path.to_path_buf(),
                    field,
                });
            }
            if !config.mentions.contains_key(mention_alias) {
                return Err(ConfigError::MissingMentionAlias {
                    path: path.to_path_buf(),
                    field,
                });
            }
        }
    }

    for (alias, mention) in &config.mentions {
        let alias_field = format!("mentions.{alias}");
        validate_alias(path, &alias_field, alias)?;
        if mention.target.is_empty() {
            return Err(ConfigError::MissingMentionTarget {
                path: path.to_path_buf(),
                field: format!("{alias_field}.target"),
            });
        }
        if MentionTarget::parse(&mention.target).is_none() {
            return Err(ConfigError::InvalidMentionTarget {
                path: path.to_path_buf(),
                field: format!("{alias_field}.target"),
            });
        }
    }

    for (alias, inbound) in &config.inbound {
        let alias_field = format!("inbound.{alias}");
        validate_alias(path, &alias_field, alias)?;
        if !config.destinations.contains_key(alias) {
            return Err(ConfigError::UnconfiguredInboundAlias {
                path: path.to_path_buf(),
                field: alias_field,
            });
        }
        if !inbound.enabled {
            // Disabled inbound aliases are still required to name a configured
            // channel; disabling retrieval must not turn into a raw channel.
            continue;
        }
    }

    validate_retention(&config.retention, path)?;

    let mut auto_send = BTreeSet::new();
    for (index, entry) in config.auto_send.iter().enumerate() {
        let field = format!("auto_send[{index}]");
        if !is_exact_policy_value(&entry.event_type)
            || !is_exact_policy_value(&entry.destination)
            || !is_exact_policy_value(&entry.severity)
        {
            return Err(ConfigError::InvalidAutoSend {
                path: path.to_path_buf(),
                field,
            });
        }
        if !config.destinations.contains_key(&entry.destination) {
            return Err(ConfigError::UnknownDestinationAlias {
                path: path.to_path_buf(),
                field: format!("{field}.destination"),
            });
        }
        let tuple = (&entry.event_type, &entry.destination, &entry.severity);
        if !auto_send.insert(tuple) {
            return Err(ConfigError::DuplicateAutoSend {
                path: path.to_path_buf(),
                field,
            });
        }
    }

    Ok(())
}

/// Verifies any locally known Discord references against the configured
/// workspace. Unknown references remain unresolved for the Discord adapter;
/// known references in another workspace fail closed here.
pub fn validate_workspace_references(
    config: &RepositoryConfig,
    path: &Path,
    index: &WorkspaceReferenceIndex,
) -> Result<(), ConfigError> {
    for (alias, destination) in &config.destinations {
        if let Some(workspace_id) = index.channel_workspace(&destination.channel_id)
            && workspace_id != config.discord.workspace_id
        {
            return Err(ConfigError::CrossWorkspaceReference {
                path: path.to_path_buf(),
                field: format!("destinations.{alias}.channel_id"),
            });
        }
    }
    for (alias, mention) in &config.mentions {
        let Some(target) = MentionTarget::parse(&mention.target) else {
            return Err(ConfigError::InvalidMentionTarget {
                path: path.to_path_buf(),
                field: format!("mentions.{alias}.target"),
            });
        };
        if let Some(workspace_id) = index.mention_workspace(&target)
            && workspace_id != config.discord.workspace_id
        {
            return Err(ConfigError::CrossWorkspaceReference {
                path: path.to_path_buf(),
                field: format!("mentions.{alias}.target"),
            });
        }
    }
    Ok(())
}

/// Returns whether a key is secret-like and therefore forbidden in committed
/// configuration. The check is key-only; values are never inspected.
#[must_use]
pub fn is_secret_like_field(key: &str) -> bool {
    let compact = compact_key(key);
    let auth_like = compact == "auth"
        || compact.contains("authorization")
        || compact.contains("authheader")
        || compact.contains("authtoken");
    [
        "token",
        "password",
        "passwd",
        "secret",
        "privatekey",
        "cookie",
        "credential",
        "apikey",
        "accesskey",
        "clientsecret",
        "webhook",
        "bearer",
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
    ]
    .iter()
    .any(|term| compact.contains(term))
        || auth_like
}

fn compact_key(key: &str) -> String {
    key.chars()
        .filter(|character| !matches!(character, '_' | '-' | '.' | ' '))
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_raw_destination_field(key: &str) -> bool {
    matches!(
        compact_key(key).as_str(),
        "channelid"
            | "channel"
            | "destinationid"
            | "destination"
            | "rawdestination"
            | "rawchannelid"
            | "rawtarget"
            | "recipient"
            | "recipientid"
            | "guildid"
            | "guild"
            | "serverid"
            | "server"
            | "workspaceid"
            | "workspace"
            | "to"
    )
}

fn is_exact_policy_value(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            !character.is_control()
                && !character.is_whitespace()
                && !matches!(character, '*' | '?' | '[' | ']' | '{' | '}')
        })
}

fn is_valid_repository_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '/' | '-' | '_' | '.')
        })
}

fn is_valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
}

fn validate_alias(path: &Path, field: &str, alias: &str) -> Result<(), ConfigError> {
    if is_secret_like_field(alias) {
        return Err(ConfigError::SecretField {
            path: path.to_path_buf(),
            field: "<secret-like-field>".to_owned(),
        });
    }
    if is_valid_alias_name(alias) {
        Ok(())
    } else {
        Err(ConfigError::InvalidAlias {
            path: path.to_path_buf(),
            field: field.to_owned(),
        })
    }
}

fn validate_retention(
    retention: &crate::model::RetentionConfig,
    path: &Path,
) -> Result<(), ConfigError> {
    if retention.content_days == 0 {
        return Err(ConfigError::InvalidRetention {
            path: path.to_path_buf(),
            field: "retention.content_days".to_owned(),
        });
    }
    if retention.metadata_days == 0 {
        return Err(ConfigError::InvalidRetention {
            path: path.to_path_buf(),
            field: "retention.metadata_days".to_owned(),
        });
    }
    Ok(())
}

fn toml_error(path: &Path, source: &str, error: &toml::de::Error) -> ConfigError {
    let (line, column) = error
        .span()
        .map(|span| line_column(source, span.start))
        .unzip();
    let kind = if error.message().to_ascii_lowercase().contains("duplicate") {
        TomlErrorKind::DuplicateKey
    } else {
        TomlErrorKind::Syntax
    };
    ConfigError::InvalidToml {
        path: path.to_path_buf(),
        line,
        column,
        kind,
    }
}

fn line_column(source: &str, offset: usize) -> (usize, usize) {
    let mut byte_offset = offset.min(source.len());
    while byte_offset > 0 && !source.is_char_boundary(byte_offset) {
        byte_offset -= 1;
    }
    let prefix = &source[..byte_offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, tail)| tail)
        .chars()
        .count()
        + 1;
    (line, column)
}

#[derive(Clone, Copy)]
enum ShapeContext {
    Root,
    Discord,
    Destination,
    Mention,
    Inbound,
    Retention,
    AutoSend,
}

impl ShapeContext {
    fn is_raw_destination_context(self, key: &str) -> bool {
        is_raw_destination_field(key)
            || matches!(
                (self, compact_key(key).as_str()),
                (
                    Self::Root
                        | Self::Discord
                        | Self::Destination
                        | Self::Mention
                        | Self::Inbound
                        | Self::Retention
                        | Self::AutoSend,
                    "target" | "destination"
                )
            )
    }
}

fn validate_document_shape(value: &Value, path: &Path) -> Result<(), ConfigError> {
    let mut segments = Vec::new();
    if let Some(error) = scan_secret_keys(value, path, &mut segments) {
        return Err(error);
    }

    let root = value
        .as_table()
        .ok_or_else(|| ConfigError::InvalidFieldType {
            path: path.to_path_buf(),
            field: "<document>".to_owned(),
        })?;
    validate_keys(
        root,
        &[
            "schema_version",
            "repository_id",
            "discord",
            "destinations",
            "mentions",
            "inbound",
            "retention",
            "auto_send",
        ],
        ShapeContext::Root,
        path,
        &[],
    )?;

    let schema = required(root, "schema_version", path)?;
    let version = schema
        .as_integer()
        .ok_or_else(|| ConfigError::InvalidFieldType {
            path: path.to_path_buf(),
            field: "schema_version".to_owned(),
        })?;
    if version != i64::from(SCHEMA_VERSION) {
        return Err(ConfigError::UnsupportedSchemaVersion {
            path: path.to_path_buf(),
            version,
        });
    }
    expect_string(
        required(root, "repository_id", path)?,
        "repository_id",
        path,
    )?;

    let discord = expect_table(required(root, "discord", path)?, "discord", path)?;
    validate_keys(
        discord,
        &["workspace_id"],
        ShapeContext::Discord,
        path,
        &["discord".to_owned()],
    )?;
    expect_string(
        required_path(discord, "workspace_id", "discord.workspace_id", path)?,
        "discord.workspace_id",
        path,
    )?;

    let destinations_value = required(root, "destinations", path)?;
    let destinations = expect_table(destinations_value, "destinations", path)?;
    validate_alias_table(
        destinations,
        "destinations",
        path,
        ShapeContext::Destination,
        &["channel_id", "allowed_mentions"],
    )?;

    let mentions_value = required(root, "mentions", path)?;
    let mentions = expect_table(mentions_value, "mentions", path)?;
    validate_alias_table(
        mentions,
        "mentions",
        path,
        ShapeContext::Mention,
        &["target"],
    )?;

    let inbound_value = required(root, "inbound", path)?;
    let inbound = expect_table(inbound_value, "inbound", path)?;
    validate_alias_table(
        inbound,
        "inbound",
        path,
        ShapeContext::Inbound,
        &["enabled"],
    )?;

    let retention_value = required(root, "retention", path)?;
    let retention = expect_table(retention_value, "retention", path)?;
    validate_keys(
        retention,
        &["content_days", "metadata_days"],
        ShapeContext::Retention,
        path,
        &["retention".to_owned()],
    )?;
    expect_positive_integer(
        required_path(retention, "content_days", "retention.content_days", path)?,
        "retention.content_days",
        path,
    )?;
    expect_positive_integer(
        required_path(retention, "metadata_days", "retention.metadata_days", path)?,
        "retention.metadata_days",
        path,
    )?;

    let auto_send_value = required(root, "auto_send", path)?;
    let auto_send = auto_send_value
        .as_array()
        .ok_or_else(|| ConfigError::InvalidFieldType {
            path: path.to_path_buf(),
            field: "auto_send".to_owned(),
        })?;
    for (index, entry) in auto_send.iter().enumerate() {
        let entry_path = format!("auto_send[{index}]");
        let entry_table = entry
            .as_table()
            .ok_or_else(|| ConfigError::InvalidFieldType {
                path: path.to_path_buf(),
                field: entry_path.clone(),
            })?;
        validate_keys(
            entry_table,
            &["event_type", "destination", "severity"],
            ShapeContext::AutoSend,
            path,
            std::slice::from_ref(&entry_path),
        )?;
        expect_string(
            required_path(
                entry_table,
                "event_type",
                &format!("{entry_path}.event_type"),
                path,
            )?,
            &format!("{entry_path}.event_type"),
            path,
        )?;
        expect_string(
            required_path(
                entry_table,
                "destination",
                &format!("{entry_path}.destination"),
                path,
            )?,
            &format!("{entry_path}.destination"),
            path,
        )?;
        expect_string(
            required_path(
                entry_table,
                "severity",
                &format!("{entry_path}.severity"),
                path,
            )?,
            &format!("{entry_path}.severity"),
            path,
        )?;
    }

    Ok(())
}

fn validate_alias_table(
    table: &toml::Table,
    map_name: &str,
    path: &Path,
    entry_context: ShapeContext,
    allowed_fields: &[&str],
) -> Result<(), ConfigError> {
    for (alias, value) in table {
        if is_secret_like_field(alias) {
            return Err(ConfigError::SecretField {
                path: path.to_path_buf(),
                field: format!("{map_name}.<secret-like-field>"),
            });
        }
        let alias_field = format!("{map_name}.{alias}");
        if !is_valid_alias_name(alias) {
            return Err(ConfigError::InvalidAlias {
                path: path.to_path_buf(),
                field: alias_field,
            });
        }
        let entry = value
            .as_table()
            .ok_or_else(|| ConfigError::InvalidFieldType {
                path: path.to_path_buf(),
                field: alias_field.clone(),
            })?;
        validate_keys(
            entry,
            allowed_fields,
            entry_context,
            path,
            &[map_name.to_owned(), alias.clone()],
        )?;
        match entry_context {
            ShapeContext::Destination => {
                expect_string(
                    required_path(
                        entry,
                        "channel_id",
                        &format!("{alias_field}.channel_id"),
                        path,
                    )?,
                    &format!("{alias_field}.channel_id"),
                    path,
                )?;
                let mentions_value = required_path(
                    entry,
                    "allowed_mentions",
                    &format!("{alias_field}.allowed_mentions"),
                    path,
                )?;
                let mentions =
                    mentions_value
                        .as_array()
                        .ok_or_else(|| ConfigError::InvalidFieldType {
                            path: path.to_path_buf(),
                            field: format!("{alias_field}.allowed_mentions"),
                        })?;
                for (index, mention) in mentions.iter().enumerate() {
                    if mention.as_str().is_none() {
                        return Err(ConfigError::InvalidFieldType {
                            path: path.to_path_buf(),
                            field: format!("{alias_field}.allowed_mentions[{index}]"),
                        });
                    }
                }
            }
            ShapeContext::Mention => {
                let target =
                    entry
                        .get("target")
                        .ok_or_else(|| ConfigError::MissingMentionTarget {
                            path: path.to_path_buf(),
                            field: format!("{alias_field}.target"),
                        })?;
                expect_string(target, &format!("{alias_field}.target"), path)?;
            }
            ShapeContext::Inbound => {
                match required_path(entry, "enabled", &format!("{alias_field}.enabled"), path)?
                    .as_bool()
                {
                    Some(_) => {}
                    None => {
                        return Err(ConfigError::InvalidFieldType {
                            path: path.to_path_buf(),
                            field: format!("{alias_field}.enabled"),
                        });
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn scan_secret_keys(value: &Value, path: &Path, segments: &mut Vec<String>) -> Option<ConfigError> {
    match value {
        Value::Table(table) => {
            for (key, child) in table {
                segments.push(key.clone());
                if is_secret_like_field(key) {
                    return Some(ConfigError::SecretField {
                        path: path.to_path_buf(),
                        field: redacted_field_path(segments),
                    });
                }
                if let Some(error) = scan_secret_keys(child, path, segments) {
                    return Some(error);
                }
                segments.pop();
            }
        }
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                segments.push(format!("[{index}]"));
                if let Some(error) = scan_secret_keys(child, path, segments) {
                    return Some(error);
                }
                segments.pop();
            }
        }
        _ => {}
    }
    None
}

fn validate_keys(
    table: &toml::Table,
    allowed: &[&str],
    context: ShapeContext,
    path: &Path,
    parent_segments: &[String],
) -> Result<(), ConfigError> {
    for key in table.keys() {
        let mut segments = parent_segments.to_vec();
        segments.push(key.clone());
        if is_secret_like_field(key) {
            return Err(ConfigError::SecretField {
                path: path.to_path_buf(),
                field: redacted_field_path(&segments),
            });
        }
        if !allowed.contains(&key.as_str()) {
            if context.is_raw_destination_context(key) {
                return Err(ConfigError::RawDestinationField {
                    path: path.to_path_buf(),
                    field: field_path(&segments),
                });
            }
            return Err(ConfigError::UnknownField {
                path: path.to_path_buf(),
                field: field_path(&segments),
            });
        }
    }
    Ok(())
}

fn required<'a>(table: &'a toml::Table, key: &str, path: &Path) -> Result<&'a Value, ConfigError> {
    table.get(key).ok_or_else(|| ConfigError::MissingField {
        path: path.to_path_buf(),
        field: key.to_owned(),
    })
}

fn required_path<'a>(
    table: &'a toml::Table,
    key: &str,
    field: &str,
    path: &Path,
) -> Result<&'a Value, ConfigError> {
    table.get(key).ok_or_else(|| ConfigError::MissingField {
        path: path.to_path_buf(),
        field: field.to_owned(),
    })
}

fn expect_table<'a>(
    value: &'a Value,
    field: &str,
    path: &Path,
) -> Result<&'a toml::Table, ConfigError> {
    value
        .as_table()
        .ok_or_else(|| ConfigError::InvalidFieldType {
            path: path.to_path_buf(),
            field: field.to_owned(),
        })
}

fn expect_string(value: &Value, field: &str, path: &Path) -> Result<(), ConfigError> {
    if value.as_str().is_some() {
        Ok(())
    } else {
        Err(ConfigError::InvalidFieldType {
            path: path.to_path_buf(),
            field: field.to_owned(),
        })
    }
}

fn expect_positive_integer(value: &Value, field: &str, path: &Path) -> Result<(), ConfigError> {
    match value.as_integer() {
        Some(value) if value > 0 && value <= i64::from(u32::MAX) => Ok(()),
        Some(_) => Err(ConfigError::InvalidRetention {
            path: path.to_path_buf(),
            field: field.to_owned(),
        }),
        None => Err(ConfigError::InvalidFieldType {
            path: path.to_path_buf(),
            field: field.to_owned(),
        }),
    }
}

fn is_valid_alias_name(alias: &str) -> bool {
    !alias.is_empty()
        && alias.len() <= 64
        && alias
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && alias
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
}

fn field_path(segments: &[String]) -> String {
    let mut output = String::new();
    for segment in segments {
        if segment.starts_with('[') {
            output.push_str(segment);
        } else {
            if !output.is_empty() && !output.ends_with('[') {
                output.push('.');
            }
            output.push_str(segment);
        }
    }
    if output.is_empty() {
        "<root>".to_owned()
    } else {
        output
    }
}

fn redacted_field_path(segments: &[String]) -> String {
    let redacted: Vec<String> = segments
        .iter()
        .map(|segment| {
            if is_secret_like_field(segment) {
                "<secret-like-field>".to_owned()
            } else {
                segment.clone()
            }
        })
        .collect();
    field_path(&redacted)
}

#[cfg(test)]
mod tests {
    use super::{is_secret_like_field, parse_config};
    use std::path::Path;

    #[test]
    fn secret_key_classification_is_key_only() {
        assert!(is_secret_like_field("api_key"));
        assert!(is_secret_like_field("Authorization"));
        assert!(!is_secret_like_field("workspace_id"));
        assert!(!is_secret_like_field("allowed_mentions"));
        assert!(!is_secret_like_field("author"));
    }

    #[test]
    fn parse_error_does_not_retain_source_text() {
        let source = "schema_version = 1\nrepository_id = \"DO_NOT_ECHO_SENTINEL\"\n";
        let error = parse_config(Path::new("config.toml"), source).expect_err("missing sections");
        assert!(!error.to_string().contains("DO_NOT_ECHO_SENTINEL"));
    }
}
