//! Strict protocol-version-1 input types for the messaging command layer.
//!
//! The command layer owns schema and identifier validation only.  Domain
//! crates remain responsible for draft, policy, delivery, inbound, and reply
//! safety decisions after a request is dispatched.

use repo_com_draft_model::{
    DestinationAlias, DraftBody, DraftId, DraftMetadata, EventType, Severity,
};
use repo_com_foundation::{ErrorCategory, PROTOCOL_VERSION, RepoComError};
use repo_com_inbox_fetch::{FetchBoundary, rfc3339_to_unix_millis};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

/// The command names understood by this crate.
///
/// The dotted spellings are the canonical protocol names.  The parser accepts
/// the equivalent kebab and snake spellings for callers that construct a
/// command name from a shell-style path, but it never supplies a default.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessagingCommand {
    /// Create one new draft.
    DraftCreate,
    /// Show one exact draft revision.
    DraftShow,
    /// Create a new immutable revision of an existing draft.
    DraftUpdate,
    /// Build a side-effect-free exact preview.
    DraftPreview,
    /// Record an exact human approval.
    DraftApprove,
    /// Record an exact redacted secret-finding override.
    DraftSecretOverride,
    /// Dispatch one exact send through the delivery owner.
    Send,
    /// Run the read-only Discord setup check.
    SetupCheck,
    /// Fetch one explicitly bounded inbound page.
    InboxFetch,
    /// Acknowledge one or more stored inbound items locally.
    InboxAcknowledge,
    /// Archive one or more stored inbound items locally.
    InboxArchive,
    /// Create a validated reply draft.
    ReplyDraftCreate,
}

impl MessagingCommand {
    /// Returns the canonical dotted protocol spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DraftCreate => "draft.create",
            Self::DraftShow => "draft.show",
            Self::DraftUpdate => "draft.update",
            Self::DraftPreview => "draft.preview",
            Self::DraftApprove => "draft.approve",
            Self::DraftSecretOverride => "draft.secret-override",
            Self::Send => "send",
            Self::SetupCheck => "setup-check",
            Self::InboxFetch => "inbox.fetch",
            Self::InboxAcknowledge => "inbox.acknowledge",
            Self::InboxArchive => "inbox.archive",
            Self::ReplyDraftCreate => "reply.draft-create",
        }
    }

    /// Parses a canonical or shell-style command spelling.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "draft.create" | "draft-create" | "draft_create" => Some(Self::DraftCreate),
            "draft.show" | "draft-show" | "draft_show" => Some(Self::DraftShow),
            "draft.update" | "draft-update" | "draft_update" => Some(Self::DraftUpdate),
            "draft.preview" | "draft-preview" | "draft_preview" => Some(Self::DraftPreview),
            "draft.approve" | "draft-approve" | "draft_approve" => Some(Self::DraftApprove),
            "draft.secret-override" | "draft-secret-override" | "draft_secret_override" => {
                Some(Self::DraftSecretOverride)
            }
            "send" => Some(Self::Send),
            "setup-check" | "setup.check" | "setup_check" => Some(Self::SetupCheck),
            "inbox.fetch" | "inbox-fetch" | "inbox_fetch" => Some(Self::InboxFetch),
            "inbox.acknowledge" | "inbox-acknowledge" | "inbox_acknowledge" => {
                Some(Self::InboxAcknowledge)
            }
            "inbox.archive" | "inbox-archive" | "inbox_archive" => Some(Self::InboxArchive),
            "reply.draft-create" | "reply-draft-create" | "reply_draft_create" => {
                Some(Self::ReplyDraftCreate)
            }
            _ => None,
        }
    }
}

impl std::fmt::Display for MessagingCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The strict outer protocol envelope.
///
/// Unknown envelope fields are rejected before command dispatch.  The input
/// value is decoded into one of the strict command-specific types below.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MessagingEnvelope {
    /// The only accepted protocol version.
    pub protocol_version: u8,
    /// A command name, never a default.
    pub command: String,
    /// A JSON object containing the command-specific fields.
    pub input: Value,
}

/// Strict metadata fields accepted by draft and reply creation/update.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MetadataInput {
    /// Optional repository label.
    pub repository_label: Option<String>,
    /// Optional branch name.
    pub branch: Option<String>,
    /// Optional commit identifier.
    pub commit: Option<String>,
}

impl MetadataInput {
    /// Converts the wire metadata into the domain model after bounded
    /// metadata validation.
    pub fn to_domain(&self) -> Result<DraftMetadata, RepoComError> {
        let mut metadata = DraftMetadata::new();
        if let Some(value) = &self.repository_label {
            metadata = metadata
                .with_repository_label(value.clone())
                .map_err(|_| usage("repository_label is not valid bounded metadata"))?;
        }
        if let Some(value) = &self.branch {
            metadata = metadata
                .with_branch(value.clone())
                .map_err(|_| usage("branch is not valid bounded metadata"))?;
        }
        if let Some(value) = &self.commit {
            metadata = metadata
                .with_commit(value.clone())
                .map_err(|_| usage("commit is not valid bounded metadata"))?;
        }
        Ok(metadata)
    }
}

/// Input for `draft.create`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DraftCreateInput {
    /// Exact repository scope.
    pub repository_id: String,
    /// Exact new draft identifier.
    pub draft_id: String,
    /// Named configured destination; a raw Discord channel is not accepted.
    pub destination_alias: String,
    /// Draft body.
    pub text: String,
    /// Exact event type.
    pub event_type: String,
    /// Exact severity.
    pub severity: String,
    /// Bounded optional metadata.
    #[serde(default)]
    pub metadata: MetadataInput,
    /// Canonical UTC creation timestamp supplied by the caller.
    pub created_at: String,
    /// Matching Unix creation time supplied by the caller.
    pub created_at_unix_seconds: u64,
    /// Optional expiry lifetime in seconds.
    #[serde(default)]
    pub expires_in_seconds: Option<u64>,
}

/// Input for `draft.show` and `draft.preview`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DraftIdentityInput {
    /// Exact repository scope.
    pub repository_id: String,
    /// Exact draft identifier.
    pub draft_id: String,
    /// Exact immutable revision number.
    pub revision: u64,
}

/// Input for `draft.update`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DraftUpdateInput {
    /// Exact repository scope.
    pub repository_id: String,
    /// Exact draft identifier.
    pub draft_id: String,
    /// Exact current revision being replaced.
    pub revision: u64,
    /// Named configured destination for the new immutable revision.
    pub destination_alias: String,
    /// New draft body.
    pub text: String,
    /// Exact event type.
    pub event_type: String,
    /// Exact severity.
    pub severity: String,
    /// Bounded optional metadata.
    #[serde(default)]
    pub metadata: MetadataInput,
    /// Canonical UTC creation timestamp for the new revision.
    pub created_at: String,
    /// Matching Unix creation time for the new revision.
    pub created_at_unix_seconds: u64,
    /// Optional expiry lifetime in seconds.
    #[serde(default)]
    pub expires_in_seconds: Option<u64>,
}

/// Input for `draft.approve` and `draft.secret-override`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DraftApprovalInput {
    /// Exact repository scope.
    pub repository_id: String,
    /// Exact draft identifier.
    pub draft_id: String,
    /// Exact immutable revision number.
    pub revision: u64,
}

/// Input for `send`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SendInput {
    /// Exact repository scope.
    pub repository_id: String,
    /// Exact draft identifier.
    pub draft_id: String,
    /// Exact immutable revision number.
    pub revision: u64,
}

/// Input for `setup-check`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetupCheckInput {
    /// Exact repository scope.
    pub repository_id: String,
}

/// Input for `inbox.fetch`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InboxFetchInput {
    /// Exact repository scope.
    pub repository_id: String,
    /// Named enabled inbound alias, never a raw channel identifier.
    pub alias: String,
    /// Optional last-event cursor.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Optional RFC 3339 lower-bound time.
    #[serde(default)]
    pub time: Option<String>,
    /// Dedicated bot user ID used only for untrusted mention evidence.
    pub bot_user_id: String,
    /// Optional deterministic observation timestamp.
    #[serde(default)]
    pub retrieved_at: Option<String>,
}

/// Input for `inbox.acknowledge` and `inbox.archive`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InboxItemActionInput {
    /// Exact repository scope.
    pub repository_id: String,
    /// One or more exact stored inbound item identifiers.
    pub item_ids: Vec<String>,
    /// Canonical UTC timestamp supplied by the caller.
    pub at: String,
}

/// Input for `reply.draft-create`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyDraftCreateInput {
    /// Exact repository scope.
    pub repository_id: String,
    /// Exact retained inbound item identifier.
    pub inbound_item_id: String,
    /// Exact new reply draft identifier.
    pub draft_id: String,
    /// Reply body; the target is selected by the domain service.
    pub text: String,
    /// Exact event type.
    pub event_type: String,
    /// Exact severity.
    pub severity: String,
    /// Bounded optional metadata.
    #[serde(default)]
    pub metadata: MetadataInput,
    /// Canonical UTC creation timestamp.
    pub created_at: String,
    /// Matching Unix creation time.
    pub created_at_unix_seconds: u64,
    /// Optional expiry lifetime in seconds.
    #[serde(default)]
    pub expires_in_seconds: Option<u64>,
}

/// A parsed and semantically validated messaging command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MessagingInput {
    /// Draft creation input.
    DraftCreate(DraftCreateInput),
    /// Exact draft lookup input.
    DraftShow(DraftIdentityInput),
    /// Immutable draft revision update input.
    DraftUpdate(DraftUpdateInput),
    /// Exact draft preview input.
    DraftPreview(DraftIdentityInput),
    /// Exact approval input.
    DraftApprove(DraftApprovalInput),
    /// Exact secret override input.
    DraftSecretOverride(DraftApprovalInput),
    /// Exact send input.
    Send(SendInput),
    /// Setup-check input.
    SetupCheck(SetupCheckInput),
    /// Bounded fetch input.
    InboxFetch(InboxFetchInput),
    /// Local acknowledgement input.
    InboxAcknowledge(InboxItemActionInput),
    /// Local archive input.
    InboxArchive(InboxItemActionInput),
    /// Reply-draft creation input.
    ReplyDraftCreate(ReplyDraftCreateInput),
}

impl MessagingInput {
    /// Returns the canonical command name.
    #[must_use]
    pub const fn command(&self) -> MessagingCommand {
        match self {
            Self::DraftCreate(_) => MessagingCommand::DraftCreate,
            Self::DraftShow(_) => MessagingCommand::DraftShow,
            Self::DraftUpdate(_) => MessagingCommand::DraftUpdate,
            Self::DraftPreview(_) => MessagingCommand::DraftPreview,
            Self::DraftApprove(_) => MessagingCommand::DraftApprove,
            Self::DraftSecretOverride(_) => MessagingCommand::DraftSecretOverride,
            Self::Send(_) => MessagingCommand::Send,
            Self::SetupCheck(_) => MessagingCommand::SetupCheck,
            Self::InboxFetch(_) => MessagingCommand::InboxFetch,
            Self::InboxAcknowledge(_) => MessagingCommand::InboxAcknowledge,
            Self::InboxArchive(_) => MessagingCommand::InboxArchive,
            Self::ReplyDraftCreate(_) => MessagingCommand::ReplyDraftCreate,
        }
    }
}

/// Parses a complete protocol-version-1 command from bytes.
pub fn parse(bytes: &[u8]) -> Result<MessagingInput, RepoComError> {
    let envelope: MessagingEnvelope =
        serde_json::from_slice(bytes).map_err(|_| protocol_shape_error())?;
    if envelope.protocol_version != PROTOCOL_VERSION {
        return Err(usage("unsupported messaging protocol version"));
    }
    let Some(command) = MessagingCommand::parse(&envelope.command) else {
        return Err(usage("unknown messaging command"));
    };
    let input = match command {
        MessagingCommand::DraftCreate => MessagingInput::DraftCreate(decode(envelope.input)?),
        MessagingCommand::DraftShow => MessagingInput::DraftShow(decode(envelope.input)?),
        MessagingCommand::DraftUpdate => MessagingInput::DraftUpdate(decode(envelope.input)?),
        MessagingCommand::DraftPreview => MessagingInput::DraftPreview(decode(envelope.input)?),
        MessagingCommand::DraftApprove => MessagingInput::DraftApprove(decode(envelope.input)?),
        MessagingCommand::DraftSecretOverride => {
            MessagingInput::DraftSecretOverride(decode(envelope.input)?)
        }
        MessagingCommand::Send => MessagingInput::Send(decode(envelope.input)?),
        MessagingCommand::SetupCheck => MessagingInput::SetupCheck(decode(envelope.input)?),
        MessagingCommand::InboxFetch => MessagingInput::InboxFetch(decode(envelope.input)?),
        MessagingCommand::InboxAcknowledge => {
            MessagingInput::InboxAcknowledge(decode(envelope.input)?)
        }
        MessagingCommand::InboxArchive => MessagingInput::InboxArchive(decode(envelope.input)?),
        MessagingCommand::ReplyDraftCreate => {
            MessagingInput::ReplyDraftCreate(decode(envelope.input)?)
        }
    };
    validate_input(&input)?;
    Ok(input)
}

/// Parses a complete command from UTF-8 text.
pub fn parse_str(value: &str) -> Result<MessagingInput, RepoComError> {
    parse(value.as_bytes())
}

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, RepoComError> {
    serde_json::from_value(value).map_err(|_| protocol_shape_error())
}

fn validate_input(input: &MessagingInput) -> Result<(), RepoComError> {
    match input {
        MessagingInput::DraftCreate(value) => {
            validate_scope(&value.repository_id)?;
            validate_draft_id(&value.draft_id)?;
            validate_destination_alias(&value.destination_alias)?;
            validate_event_and_severity(&value.event_type, &value.severity, &value.text)?;
            value.metadata.to_domain()?;
            validate_timestamp_seconds(&value.created_at, value.created_at_unix_seconds)?;
        }
        MessagingInput::DraftShow(value) | MessagingInput::DraftPreview(value) => {
            validate_identity(value)?;
        }
        MessagingInput::DraftUpdate(value) => {
            validate_scope(&value.repository_id)?;
            validate_draft_id(&value.draft_id)?;
            validate_revision(value.revision)?;
            validate_destination_alias(&value.destination_alias)?;
            validate_event_and_severity(&value.event_type, &value.severity, &value.text)?;
            value.metadata.to_domain()?;
            validate_timestamp_seconds(&value.created_at, value.created_at_unix_seconds)?;
        }
        MessagingInput::DraftApprove(value) | MessagingInput::DraftSecretOverride(value) => {
            validate_approval_identity(value)?;
        }
        MessagingInput::Send(value) => {
            validate_scope(&value.repository_id)?;
            validate_draft_id(&value.draft_id)?;
            validate_revision(value.revision)?;
        }
        MessagingInput::SetupCheck(value) => validate_scope(&value.repository_id)?,
        MessagingInput::InboxFetch(value) => {
            validate_scope(&value.repository_id)?;
            validate_alias_name(&value.alias, "inbound alias")?;
            validate_discord_id(&value.bot_user_id, "bot user id")?;
            FetchBoundary::from_parts(value.cursor.as_deref(), value.time.as_deref())
                .map_err(|_| usage("exactly one valid cursor or time boundary is required"))?;
            if let Some(retrieved_at) = &value.retrieved_at {
                validate_timestamp(retrieved_at)?;
            }
        }
        MessagingInput::InboxAcknowledge(value) | MessagingInput::InboxArchive(value) => {
            validate_scope(&value.repository_id)?;
            if value.item_ids.is_empty() {
                return Err(usage("at least one inbound item identifier is required"));
            }
            for item_id in &value.item_ids {
                validate_discord_id(item_id, "inbound item id")?;
            }
            validate_timestamp(&value.at)?;
        }
        MessagingInput::ReplyDraftCreate(value) => {
            validate_scope(&value.repository_id)?;
            validate_discord_id(&value.inbound_item_id, "inbound item id")?;
            validate_draft_id(&value.draft_id)?;
            validate_event_and_severity(&value.event_type, &value.severity, &value.text)?;
            value.metadata.to_domain()?;
            validate_timestamp_seconds(&value.created_at, value.created_at_unix_seconds)?;
        }
    }
    Ok(())
}

fn validate_identity(value: &DraftIdentityInput) -> Result<(), RepoComError> {
    validate_scope(&value.repository_id)?;
    validate_draft_id(&value.draft_id)?;
    validate_revision(value.revision)
}

fn validate_approval_identity(value: &DraftApprovalInput) -> Result<(), RepoComError> {
    validate_scope(&value.repository_id)?;
    validate_draft_id(&value.draft_id)?;
    validate_revision(value.revision)
}

fn validate_scope(value: &str) -> Result<(), RepoComError> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "/-_.".contains(character))
    {
        return Err(usage("repository_id is not a valid explicit identifier"));
    }
    Ok(())
}

fn validate_draft_id(value: &str) -> Result<(), RepoComError> {
    DraftId::new(value).map_err(|_| usage("draft_id is not a valid explicit identifier"))?;
    Ok(())
}

fn validate_revision(value: u64) -> Result<(), RepoComError> {
    if value == 0 {
        return Err(usage("revision must be a positive explicit identifier"));
    }
    Ok(())
}

fn validate_destination_alias(value: &str) -> Result<(), RepoComError> {
    DestinationAlias::new(value)
        .map_err(|_| usage("destination_alias must name a configured destination"))?;
    Ok(())
}

fn validate_alias_name(value: &str, field: &'static str) -> Result<(), RepoComError> {
    let mut characters = value.chars();
    let valid = value.len() <= 64
        && characters
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        });
    if !valid {
        return Err(usage(match field {
            "inbound alias" => "inbound alias is not valid",
            _ => "alias is not valid",
        }));
    }
    Ok(())
}

fn validate_event_and_severity(
    event_type: &str,
    severity: &str,
    text: &str,
) -> Result<(), RepoComError> {
    EventType::new(event_type).map_err(|_| usage("event_type is not valid"))?;
    Severity::new(severity).map_err(|_| usage("severity is not valid"))?;
    DraftBody::new(text).map_err(|_| usage("text is not a valid draft body"))?;
    Ok(())
}

fn validate_discord_id(value: &str, field: &'static str) -> Result<(), RepoComError> {
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(usage(match field {
            "bot user id" => "bot_user_id is not a valid Discord identifier",
            _ => "inbound item id is not a valid Discord identifier",
        }));
    }
    Ok(())
}

fn validate_timestamp(value: &str) -> Result<(), RepoComError> {
    rfc3339_to_unix_millis(value)
        .map(|_| ())
        .map_err(|_| usage("timestamp is not valid RFC 3339"))
}

fn validate_timestamp_seconds(value: &str, seconds: u64) -> Result<(), RepoComError> {
    let millis =
        rfc3339_to_unix_millis(value).map_err(|_| usage("created_at is not valid RFC 3339"))?;
    let expected = i64::try_from(seconds.checked_mul(1_000).ok_or_else(|| {
        RepoComError::new(
            ErrorCategory::UsageOrSchema,
            "created_at timestamp is out of range",
        )
    })?)
    .map_err(|_| {
        RepoComError::new(
            ErrorCategory::UsageOrSchema,
            "created_at timestamp is out of range",
        )
    })?;
    if millis != expected {
        return Err(usage(
            "created_at and created_at_unix_seconds must identify the same instant",
        ));
    }
    Ok(())
}

fn protocol_shape_error() -> RepoComError {
    usage("structured input must be a strict protocol-version-1 command object")
}

fn usage(message: impl Into<String>) -> RepoComError {
    RepoComError::new(ErrorCategory::UsageOrSchema, message)
}
