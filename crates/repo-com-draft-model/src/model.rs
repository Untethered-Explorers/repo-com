use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use repo_com_config::{MentionTarget, ResolvedConfig, ResolvedDestination};
use serde::{Deserialize, Serialize};

pub const DEFAULT_EXPIRY_SECONDS: u64 = 24 * 60 * 60;
pub const MAX_EXPIRY_SECONDS: u64 = 7 * 24 * 60 * 60;
pub const MAX_DRAFT_BODY_BYTES: usize = 64 * 1024;
pub const MAX_METADATA_VALUE_BYTES: usize = 256;
pub const MAX_EVENT_TYPE_BYTES: usize = 128;
pub const MAX_SEVERITY_BYTES: usize = 64;
pub const MAX_DRAFT_ID_BYTES: usize = 128;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DraftError {
    InvalidDraftId,
    InvalidEventType,
    InvalidSeverity,
    InvalidDestinationAlias,
    InvalidText,
    InvalidMetadataValue {
        field: &'static str,
    },
    InvalidReplyReference {
        field: &'static str,
    },
    InvalidResolvedDestination {
        field: &'static str,
    },
    UnknownDestinationAlias,
    ReplyRepositoryMismatch,
    ReplyWorkspaceMismatch,
    ReplyChannelMismatch,
    ExpiryMustBePositive,
    ExpiryTooLong {
        requested_seconds: u64,
        maximum_seconds: u64,
    },
    TimestampOverflow,
    DraftIdMismatch,
    RevisionNotFound {
        draft_id: String,
        revision: u64,
    },
    RevisionExpired {
        draft_id: String,
        revision: u64,
    },
}

impl fmt::Display for DraftError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidDraftId => formatter.write_str("draft ID is not a valid identifier"),
            Self::InvalidEventType => {
                formatter.write_str("event type must be one exact non-empty value")
            }
            Self::InvalidSeverity => {
                formatter.write_str("severity must be one exact non-empty value")
            }
            Self::InvalidDestinationAlias => {
                formatter.write_str("destination must be one named repository alias")
            }
            Self::InvalidText => formatter.write_str("draft text must be non-empty"),
            Self::InvalidMetadataValue { field } => {
                write!(formatter, "{field} is not valid bounded metadata")
            }
            Self::InvalidReplyReference { field } => {
                write!(formatter, "validated reply reference has invalid {field}")
            }
            Self::InvalidResolvedDestination { field } => {
                write!(formatter, "resolved destination has invalid {field}")
            }
            Self::UnknownDestinationAlias => {
                formatter.write_str("destination alias is not configured")
            }
            Self::ReplyRepositoryMismatch => {
                formatter.write_str("reply reference repository does not match the draft")
            }
            Self::ReplyWorkspaceMismatch => {
                formatter.write_str("reply reference workspace does not match the destination")
            }
            Self::ReplyChannelMismatch => {
                formatter.write_str("reply reference channel does not match the destination")
            }
            Self::ExpiryMustBePositive => formatter.write_str("draft expiry must be positive"),
            Self::ExpiryTooLong {
                requested_seconds,
                maximum_seconds,
            } => write!(
                formatter,
                "draft expiry {requested_seconds} seconds exceeds the {maximum_seconds}-second maximum"
            ),
            Self::TimestampOverflow => formatter.write_str("draft expiry timestamp overflowed"),
            Self::DraftIdMismatch => formatter.write_str("a revision cannot change its draft ID"),
            Self::RevisionNotFound { draft_id, revision } => {
                write!(formatter, "draft {draft_id} has no revision {revision}")
            }
            Self::RevisionExpired { draft_id, revision } => {
                write!(formatter, "draft {draft_id} revision {revision} is expired")
            }
        }
    }
}

impl Error for DraftError {}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String")]
pub struct DraftId(String);

impl DraftId {
    pub fn new(value: impl Into<String>) -> Result<Self, DraftError> {
        let value = value.into();
        if is_valid_local_id(&value, MAX_DRAFT_ID_BYTES) {
            Ok(Self(value))
        } else {
            Err(DraftError::InvalidDraftId)
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for DraftId {
    type Error = DraftError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String")]
pub struct EventType(String);

impl EventType {
    pub fn new(value: impl Into<String>) -> Result<Self, DraftError> {
        let value = value.into();
        if is_exact_component(&value, MAX_EVENT_TYPE_BYTES) {
            Ok(Self(value))
        } else {
            Err(DraftError::InvalidEventType)
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for EventType {
    type Error = DraftError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String")]
pub struct Severity(String);

impl Severity {
    pub fn new(value: impl Into<String>) -> Result<Self, DraftError> {
        let value = value.into();
        if is_exact_component(&value, MAX_SEVERITY_BYTES) {
            Ok(Self(value))
        } else {
            Err(DraftError::InvalidSeverity)
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for Severity {
    type Error = DraftError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(try_from = "String")]
pub struct DestinationAlias(String);

impl DestinationAlias {
    pub fn new(value: impl Into<String>) -> Result<Self, DraftError> {
        let value = value.into();
        let mut characters = value.chars();
        let valid = value.len() <= 64
            && characters
                .next()
                .is_some_and(|character| character.is_ascii_alphabetic())
            && characters.all(|character| {
                character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
            });
        if valid {
            Ok(Self(value))
        } else {
            Err(DraftError::InvalidDestinationAlias)
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for DestinationAlias {
    type Error = DraftError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "String")]
pub struct DraftBody(String);

impl DraftBody {
    pub fn new(value: impl Into<String>) -> Result<Self, DraftError> {
        let value = value.into();
        if value.len() <= MAX_DRAFT_BODY_BYTES && !value.trim().is_empty() {
            Ok(Self(value))
        } else {
            Err(DraftError::InvalidText)
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl TryFrom<String> for DraftBody {
    type Error = DraftError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "String")]
struct MetadataValue(String);

impl MetadataValue {
    fn new(field: &'static str, value: impl Into<String>) -> Result<Self, DraftError> {
        let value = value.into();
        if value.len() <= MAX_METADATA_VALUE_BYTES
            && !value.trim().is_empty()
            && !value.chars().any(char::is_control)
        {
            Ok(Self(value))
        } else {
            Err(DraftError::InvalidMetadataValue { field })
        }
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for MetadataValue {
    type Error = DraftError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new("metadata", value)
    }
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct DraftMetadata {
    repository_label: Option<MetadataValue>,
    branch: Option<MetadataValue>,
    commit: Option<MetadataValue>,
}

impl DraftMetadata {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_repository_label(mut self, value: impl Into<String>) -> Result<Self, DraftError> {
        self.repository_label = Some(MetadataValue::new("repository_label", value)?);
        Ok(self)
    }

    pub fn with_branch(mut self, value: impl Into<String>) -> Result<Self, DraftError> {
        self.branch = Some(MetadataValue::new("branch", value)?);
        Ok(self)
    }

    pub fn with_commit(mut self, value: impl Into<String>) -> Result<Self, DraftError> {
        self.commit = Some(MetadataValue::new("commit", value)?);
        Ok(self)
    }

    #[must_use]
    pub fn repository_label(&self) -> Option<&str> {
        self.repository_label.as_ref().map(MetadataValue::as_str)
    }

    #[must_use]
    pub fn branch(&self) -> Option<&str> {
        self.branch.as_ref().map(MetadataValue::as_str)
    }

    #[must_use]
    pub fn commit(&self) -> Option<&str> {
        self.commit.as_ref().map(MetadataValue::as_str)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(try_from = "ReplyReferenceWire")]
pub struct AuthorizedReplyReference {
    repository_id: String,
    workspace_id: String,
    channel_id: String,
    inbound_item_id: String,
    message_id: String,
    authorization_reference: String,
    validated_snapshot_hash: String,
}

impl AuthorizedReplyReference {
    #[allow(clippy::too_many_arguments)]
    pub fn from_validated_target(
        repository_id: impl Into<String>,
        workspace_id: impl Into<String>,
        channel_id: impl Into<String>,
        inbound_item_id: impl Into<String>,
        message_id: impl Into<String>,
        authorization_reference: impl Into<String>,
        validated_snapshot_hash: impl Into<String>,
    ) -> Result<Self, DraftError> {
        let reference = Self {
            repository_id: repository_id.into(),
            workspace_id: workspace_id.into(),
            channel_id: channel_id.into(),
            inbound_item_id: inbound_item_id.into(),
            message_id: message_id.into(),
            authorization_reference: authorization_reference.into(),
            validated_snapshot_hash: validated_snapshot_hash.into(),
        };
        reference.validate()?;
        Ok(reference)
    }

    fn validate(&self) -> Result<(), DraftError> {
        if !is_valid_repository_id(&self.repository_id) {
            return Err(DraftError::InvalidReplyReference {
                field: "repository_id",
            });
        }
        if !is_valid_discord_id(&self.workspace_id) {
            return Err(DraftError::InvalidReplyReference {
                field: "workspace_id",
            });
        }
        if !is_valid_discord_id(&self.channel_id) {
            return Err(DraftError::InvalidReplyReference {
                field: "channel_id",
            });
        }
        if !is_valid_local_id(&self.inbound_item_id, 128) {
            return Err(DraftError::InvalidReplyReference {
                field: "inbound_item_id",
            });
        }
        if !is_valid_discord_id(&self.message_id) {
            return Err(DraftError::InvalidReplyReference {
                field: "message_id",
            });
        }
        if !is_valid_local_id(&self.authorization_reference, 128) {
            return Err(DraftError::InvalidReplyReference {
                field: "authorization_reference",
            });
        }
        if !is_lowercase_sha256_hex(&self.validated_snapshot_hash) {
            return Err(DraftError::InvalidReplyReference {
                field: "validated_snapshot_hash",
            });
        }
        Ok(())
    }

    #[must_use]
    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }

    #[must_use]
    pub fn workspace_id(&self) -> &str {
        &self.workspace_id
    }

    #[must_use]
    pub fn channel_id(&self) -> &str {
        &self.channel_id
    }

    #[must_use]
    pub fn inbound_item_id(&self) -> &str {
        &self.inbound_item_id
    }

    #[must_use]
    pub fn message_id(&self) -> &str {
        &self.message_id
    }

    #[must_use]
    pub fn authorization_reference(&self) -> &str {
        &self.authorization_reference
    }

    #[must_use]
    pub fn validated_snapshot_hash(&self) -> &str {
        &self.validated_snapshot_hash
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct ReplyReferenceWire {
    repository_id: String,
    workspace_id: String,
    channel_id: String,
    inbound_item_id: String,
    message_id: String,
    authorization_reference: String,
    validated_snapshot_hash: String,
}

impl TryFrom<ReplyReferenceWire> for AuthorizedReplyReference {
    type Error = DraftError;

    fn try_from(value: ReplyReferenceWire) -> Result<Self, Self::Error> {
        Self::from_validated_target(
            value.repository_id,
            value.workspace_id,
            value.channel_id,
            value.inbound_item_id,
            value.message_id,
            value.authorization_reference,
            value.validated_snapshot_hash,
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DraftRequest {
    draft_id: DraftId,
    destination_alias: DestinationAlias,
    text: DraftBody,
    event_type: EventType,
    severity: Severity,
    #[serde(default)]
    metadata: DraftMetadata,
    #[serde(default)]
    expires_in_seconds: Option<u64>,
    #[serde(default)]
    reply_reference: Option<AuthorizedReplyReference>,
}

impl DraftRequest {
    pub fn new(
        draft_id: impl Into<String>,
        destination_alias: impl Into<String>,
        text: impl Into<String>,
        event_type: impl Into<String>,
        severity: impl Into<String>,
    ) -> Result<Self, DraftError> {
        Ok(Self {
            draft_id: DraftId::new(draft_id)?,
            destination_alias: DestinationAlias::new(destination_alias)?,
            text: DraftBody::new(text)?,
            event_type: EventType::new(event_type)?,
            severity: Severity::new(severity)?,
            metadata: DraftMetadata::default(),
            expires_in_seconds: None,
            reply_reference: None,
        })
    }

    #[must_use]
    pub fn with_metadata(mut self, metadata: DraftMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    #[must_use]
    pub fn with_expiry_seconds(mut self, seconds: u64) -> Self {
        self.expires_in_seconds = Some(seconds);
        self
    }

    #[must_use]
    pub fn with_reply_reference(mut self, reference: AuthorizedReplyReference) -> Self {
        self.reply_reference = Some(reference);
        self
    }

    #[must_use]
    pub fn draft_id(&self) -> &DraftId {
        &self.draft_id
    }

    #[must_use]
    pub fn destination_alias(&self) -> &DestinationAlias {
        &self.destination_alias
    }

    #[must_use]
    pub fn text(&self) -> &DraftBody {
        &self.text
    }

    #[must_use]
    pub fn event_type(&self) -> &EventType {
        &self.event_type
    }

    #[must_use]
    pub fn severity(&self) -> &Severity {
        &self.severity
    }

    #[must_use]
    pub fn metadata(&self) -> &DraftMetadata {
        &self.metadata
    }

    #[must_use]
    pub fn expires_in_seconds(&self) -> Option<u64> {
        self.expires_in_seconds
    }

    #[must_use]
    pub fn reply_reference(&self) -> Option<&AuthorizedReplyReference> {
        self.reply_reference.as_ref()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct DraftExpiry {
    created_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
}

impl DraftExpiry {
    pub fn new(created_at_unix_seconds: u64, lifetime_seconds: u64) -> Result<Self, DraftError> {
        if lifetime_seconds == 0 {
            return Err(DraftError::ExpiryMustBePositive);
        }
        if lifetime_seconds > MAX_EXPIRY_SECONDS {
            return Err(DraftError::ExpiryTooLong {
                requested_seconds: lifetime_seconds,
                maximum_seconds: MAX_EXPIRY_SECONDS,
            });
        }
        let expires_at_unix_seconds = created_at_unix_seconds
            .checked_add(lifetime_seconds)
            .ok_or(DraftError::TimestampOverflow)?;
        Ok(Self {
            created_at_unix_seconds,
            expires_at_unix_seconds,
        })
    }

    #[must_use]
    pub fn created_at_unix_seconds(&self) -> u64 {
        self.created_at_unix_seconds
    }

    #[must_use]
    pub fn expires_at_unix_seconds(&self) -> u64 {
        self.expires_at_unix_seconds
    }

    #[must_use]
    pub fn is_expired(&self, at_unix_seconds: u64) -> bool {
        at_unix_seconds >= self.expires_at_unix_seconds
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DraftRevision {
    pub(crate) revision: u64,
    pub(crate) repository_id: String,
    pub(crate) text: DraftBody,
    pub(crate) metadata: DraftMetadata,
    pub(crate) event_type: EventType,
    pub(crate) severity: Severity,
    pub(crate) destination_alias: DestinationAlias,
    pub(crate) resolved_destination: ResolvedDestination,
    pub(crate) reply_reference: Option<AuthorizedReplyReference>,
    pub(crate) expiry: DraftExpiry,
    pub(crate) content_hash: String,
}

impl DraftRevision {
    #[must_use]
    pub fn number(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }

    #[must_use]
    pub fn text(&self) -> &DraftBody {
        &self.text
    }

    #[must_use]
    pub fn exact_text(&self) -> &str {
        self.text.as_str()
    }

    #[must_use]
    pub fn metadata(&self) -> &DraftMetadata {
        &self.metadata
    }

    #[must_use]
    pub fn event_type(&self) -> &EventType {
        &self.event_type
    }

    #[must_use]
    pub fn severity(&self) -> &Severity {
        &self.severity
    }

    #[must_use]
    pub fn destination_alias(&self) -> &DestinationAlias {
        &self.destination_alias
    }

    #[must_use]
    pub fn resolved_destination(&self) -> &ResolvedDestination {
        &self.resolved_destination
    }

    #[must_use]
    pub fn reply_reference(&self) -> Option<&AuthorizedReplyReference> {
        self.reply_reference.as_ref()
    }

    #[must_use]
    pub fn expiry(&self) -> DraftExpiry {
        self.expiry
    }

    #[must_use]
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    #[must_use]
    pub fn is_expired(&self, at_unix_seconds: u64) -> bool {
        self.expiry.is_expired(at_unix_seconds)
    }

    pub fn ensure_unexpired(
        &self,
        draft_id: &DraftId,
        at_unix_seconds: u64,
    ) -> Result<(), DraftError> {
        if self.is_expired(at_unix_seconds) {
            Err(DraftError::RevisionExpired {
                draft_id: draft_id.as_str().to_owned(),
                revision: self.revision,
            })
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DraftModel {
    draft_id: DraftId,
    revisions: Vec<DraftRevision>,
}

impl DraftModel {
    pub fn create(
        request: DraftRequest,
        config: &ResolvedConfig,
        created_at_unix_seconds: u64,
    ) -> Result<Self, DraftError> {
        let revision = build_revision(&request, config, 1, created_at_unix_seconds)?;
        Ok(Self {
            draft_id: request.draft_id,
            revisions: vec![revision],
        })
    }

    pub fn revise(
        &mut self,
        request: DraftRequest,
        config: &ResolvedConfig,
        created_at_unix_seconds: u64,
    ) -> Result<&DraftRevision, DraftError> {
        if request.draft_id != self.draft_id {
            return Err(DraftError::DraftIdMismatch);
        }
        let next = u64::try_from(self.revisions.len())
            .ok()
            .and_then(|length| length.checked_add(1))
            .ok_or(DraftError::TimestampOverflow)?;
        let revision = build_revision(&request, config, next, created_at_unix_seconds)?;
        self.revisions.push(revision);
        Ok(self.revisions.last().expect("a pushed revision is present"))
    }

    #[must_use]
    pub fn draft_id(&self) -> &DraftId {
        &self.draft_id
    }

    #[must_use]
    pub fn revisions(&self) -> &[DraftRevision] {
        &self.revisions
    }

    #[must_use]
    pub fn revision(&self, revision: u64) -> Option<&DraftRevision> {
        self.revisions
            .iter()
            .find(|candidate| candidate.revision == revision)
    }

    #[must_use]
    pub fn current_revision(&self) -> &DraftRevision {
        self.revisions
            .last()
            .expect("a created draft always has one immutable revision")
    }

    pub fn ensure_current_unexpired(&self, at_unix_seconds: u64) -> Result<(), DraftError> {
        self.current_revision()
            .ensure_unexpired(&self.draft_id, at_unix_seconds)
    }
}

pub type Draft = DraftModel;

fn build_revision(
    request: &DraftRequest,
    config: &ResolvedConfig,
    revision: u64,
    created_at_unix_seconds: u64,
) -> Result<DraftRevision, DraftError> {
    let repository_id = config.config.repository_id.clone();
    if !is_valid_repository_id(&repository_id) {
        return Err(DraftError::InvalidResolvedDestination {
            field: "repository_id",
        });
    }

    let alias = request.destination_alias.as_str();
    let resolved_destination = config
        .destination(alias)
        .ok_or(DraftError::UnknownDestinationAlias)?
        .clone();
    let resolved_destination = validate_and_normalize_destination(alias, resolved_destination)?;

    if let Some(reference) = &request.reply_reference {
        if reference.repository_id() != repository_id {
            return Err(DraftError::ReplyRepositoryMismatch);
        }
        if reference.workspace_id() != resolved_destination.workspace_id {
            return Err(DraftError::ReplyWorkspaceMismatch);
        }
        if reference.channel_id() != resolved_destination.channel_id {
            return Err(DraftError::ReplyChannelMismatch);
        }
    }

    let lifetime_seconds = request.expires_in_seconds.unwrap_or(DEFAULT_EXPIRY_SECONDS);
    let expiry = DraftExpiry::new(created_at_unix_seconds, lifetime_seconds)?;

    let mut revision = DraftRevision {
        revision,
        repository_id,
        text: request.text.clone(),
        metadata: request.metadata.clone(),
        event_type: request.event_type.clone(),
        severity: request.severity.clone(),
        destination_alias: request.destination_alias.clone(),
        resolved_destination,
        reply_reference: request.reply_reference.clone(),
        expiry,
        content_hash: String::new(),
    };
    revision.content_hash = revision.canonical_hash();
    Ok(revision)
}

fn validate_and_normalize_destination(
    requested_alias: &str,
    mut destination: ResolvedDestination,
) -> Result<ResolvedDestination, DraftError> {
    if destination.alias != requested_alias {
        return Err(DraftError::InvalidResolvedDestination { field: "alias" });
    }
    if !is_valid_discord_id(&destination.workspace_id) {
        return Err(DraftError::InvalidResolvedDestination {
            field: "workspace_id",
        });
    }
    if !is_valid_discord_id(&destination.channel_id) {
        return Err(DraftError::InvalidResolvedDestination {
            field: "channel_id",
        });
    }

    let mut aliases = BTreeSet::new();
    for mention in &destination.allowed_mentions {
        if !is_valid_alias_name(&mention.alias) || !aliases.insert(mention.alias.clone()) {
            return Err(DraftError::InvalidResolvedDestination {
                field: "allowed_mentions",
            });
        }
        if MentionTarget::parse(&mention.target.as_prefixed()).as_ref() != Some(&mention.target) {
            return Err(DraftError::InvalidResolvedDestination {
                field: "allowed_mentions",
            });
        }
    }
    destination
        .allowed_mentions
        .sort_by(|left, right| left.alias.cmp(&right.alias));
    Ok(destination)
}

fn is_exact_component(value: &str, maximum_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum_bytes
        && value.chars().all(|character| {
            !character.is_control()
                && !character.is_whitespace()
                && !matches!(character, '*' | '?' | '[' | ']' | '{' | '}')
        })
}

fn is_valid_local_id(value: &str, maximum_bytes: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum_bytes
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
}

fn is_valid_alias_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic())
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
}

fn is_valid_repository_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "/-_.".contains(character))
}

fn is_valid_discord_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 32 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_lowercase_sha256_hex(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
