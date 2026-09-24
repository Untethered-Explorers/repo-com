use std::fmt;

use serde::{Deserialize, Serialize};

/// Stable reason codes emitted by the pre-send safety scanner.
///
/// The serialized and human-readable representation is intentionally limited
/// to these fixed values. A finding never contains the text that caused it.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SecretReasonCode {
    /// A value has the high-confidence shape of a Discord bot token.
    DiscordBotToken,
    /// An authorization header or equivalent contains a credential value.
    AuthorizationValue,
    /// A PEM/OpenSSH/PGP private-key marker is present.
    PrivateKeyMarker,
    /// A URL contains user-info or a sensitive query credential.
    CredentialUrl,
    /// A conventional credential assignment contains a value.
    SecretAssignment,
}

impl SecretReasonCode {
    /// Every reason code in stable scanner order.
    pub const ALL: [Self; 5] = [
        Self::DiscordBotToken,
        Self::AuthorizationValue,
        Self::PrivateKeyMarker,
        Self::CredentialUrl,
        Self::SecretAssignment,
    ];

    /// Returns the stable machine-readable reason code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::DiscordBotToken => "discord-bot-token",
            Self::AuthorizationValue => "authorization-value",
            Self::PrivateKeyMarker => "private-key-marker",
            Self::CredentialUrl => "credential-url",
            Self::SecretAssignment => "secret-assignment",
        }
    }

    /// Compatibility alias for [`Self::code`].
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        self.code()
    }

    /// Compatibility alias for [`Self::code`].
    #[must_use]
    pub const fn stable_code(self) -> &'static str {
        self.code()
    }

    /// Parses a stable machine-readable reason code.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reason| reason.code() == code)
    }

    /// Named constants make call sites readable while preserving one canonical
    /// serialized code for each reason.
    pub const DISCORD_BOT_TOKEN: Self = Self::DiscordBotToken;
    /// See [`SecretReasonCode::DiscordBotToken`].
    pub const DISCORD_TOKEN: Self = Self::DiscordBotToken;
    /// See [`SecretReasonCode::AuthorizationValue`].
    pub const AUTHORIZATION_VALUE: Self = Self::AuthorizationValue;
    /// See [`SecretReasonCode::AuthorizationValue`].
    pub const AUTHORIZATION_HEADER: Self = Self::AuthorizationValue;
    /// See [`SecretReasonCode::PrivateKeyMarker`].
    pub const PRIVATE_KEY_MARKER: Self = Self::PrivateKeyMarker;
    /// See [`SecretReasonCode::CredentialUrl`].
    pub const CREDENTIAL_URL: Self = Self::CredentialUrl;
    /// See [`SecretReasonCode::SecretAssignment`].
    pub const SECRET_ASSIGNMENT: Self = Self::SecretAssignment;
}

impl fmt::Display for SecretReasonCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

/// The safe source label for a finding location.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MatchSource {
    /// The exact final rendered Discord text.
    RenderedText,
    /// One bounded value from draft metadata.
    Metadata,
}

impl MatchSource {
    /// Returns the stable machine-readable source label.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::RenderedText => "rendered-text",
            Self::Metadata => "metadata",
        }
    }
}

impl fmt::Display for MatchSource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

/// The bounded metadata field containing a finding.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MetadataField {
    /// The optional repository label.
    RepositoryLabel,
    /// The optional branch name.
    Branch,
    /// The optional commit identifier.
    Commit,
}

impl MetadataField {
    /// Returns the stable machine-readable field label.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::RepositoryLabel => "repository-label",
            Self::Branch => "branch",
            Self::Commit => "commit",
        }
    }
}

impl fmt::Display for MetadataField {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

/// A redacted location for one finding.
///
/// Offsets are byte offsets into the relevant source and the end is exclusive.
/// No source excerpt, matched value, or diagnostic text is retained.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct SecretLocation {
    source: MatchSource,
    field: Option<MetadataField>,
    start: usize,
    end: usize,
}

impl SecretLocation {
    #[must_use]
    const fn new(
        source: MatchSource,
        field: Option<MetadataField>,
        start: usize,
        end: usize,
    ) -> Self {
        Self {
            source,
            field,
            start,
            end,
        }
    }

    /// Creates a location in the final rendered text.
    #[must_use]
    pub const fn rendered_text(start: usize, end: usize) -> Self {
        Self::new(MatchSource::RenderedText, None, start, end)
    }

    /// Creates a location in one metadata field value.
    #[must_use]
    pub const fn metadata(field: MetadataField, start: usize, end: usize) -> Self {
        Self::new(MatchSource::Metadata, Some(field), start, end)
    }

    /// Returns the safe source label.
    #[must_use]
    pub const fn source(&self) -> MatchSource {
        self.source
    }

    /// Returns the metadata field, when this is a metadata location.
    #[must_use]
    pub const fn field(&self) -> Option<MetadataField> {
        self.field
    }

    /// Compatibility alias for [`Self::field`].
    #[must_use]
    pub const fn metadata_field(&self) -> Option<MetadataField> {
        self.field()
    }

    /// Returns the inclusive start byte offset.
    #[must_use]
    pub const fn start(&self) -> usize {
        self.start
    }

    /// Returns the exclusive end byte offset.
    #[must_use]
    pub const fn end(&self) -> usize {
        self.end
    }

    /// Returns the half-open byte range.
    #[must_use]
    pub const fn byte_range(&self) -> std::ops::Range<usize> {
        self.start..self.end
    }

    /// Returns whether this location points at final rendered text.
    #[must_use]
    pub const fn is_rendered_text(&self) -> bool {
        matches!(self.source, MatchSource::RenderedText)
    }

    /// Returns whether this location points at metadata.
    #[must_use]
    pub const fn is_metadata(&self) -> bool {
        matches!(self.source, MatchSource::Metadata)
    }
}

impl fmt::Display for SecretLocation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.field {
            Some(field) => write!(formatter, "metadata.{field}:{}..{}", self.start, self.end),
            None => write!(formatter, "{}:{}..{}", self.source, self.start, self.end),
        }
    }
}

/// One safe, redacted secret finding.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct SecretFinding {
    reason_code: SecretReasonCode,
    location: SecretLocation,
}

impl SecretFinding {
    #[must_use]
    pub(crate) const fn new(reason_code: SecretReasonCode, location: SecretLocation) -> Self {
        Self {
            reason_code,
            location,
        }
    }

    /// Returns the stable reason code.
    #[must_use]
    pub const fn reason_code(&self) -> SecretReasonCode {
        self.reason_code
    }

    /// Compatibility alias for [`Self::reason_code`].
    #[must_use]
    pub const fn reason(&self) -> SecretReasonCode {
        self.reason_code()
    }

    /// Returns the stable string form of the reason code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        self.reason_code.code()
    }

    /// Compatibility alias for [`Self::code`].
    #[must_use]
    pub const fn stable_code(&self) -> &'static str {
        self.code()
    }

    /// Returns the redacted location.
    #[must_use]
    pub const fn location(&self) -> &SecretLocation {
        &self.location
    }
}

impl fmt::Display for SecretFinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} at {}", self.reason_code, self.location)
    }
}

/// Compatibility name for callers that use the shorter finding type name.
pub type Finding = SecretFinding;
/// Compatibility name for callers that use the shorter location type name.
pub type FindingLocation = SecretLocation;
/// Compatibility name for a stable scanner reason code.
pub type ReasonCode = SecretReasonCode;
