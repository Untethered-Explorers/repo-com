//! Strict protocol-version-1 input types for the operations command layer.
//!
//! This module owns envelope shape, identifier syntax, page bounds, and the
//! explicit boundaries required before a command reaches a domain service. It
//! does not resolve configuration, inspect state, activate policy, or delete
//! local rows.

use std::path::PathBuf;

use repo_com_audit_query::{AuditCursor, AuditFilter};
use repo_com_foundation::{ErrorCategory, PROTOCOL_VERSION, RepoComError};
use repo_com_policy::{PolicyTuple, is_sha256_hex};
use repo_com_purge::PurgeCutoff;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

/// The command names understood by the operations handler crate.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationsCommand {
    /// Validate one explicit repository configuration.
    ConfigValidate,
    /// Read the status of one exact policy tuple.
    PolicyStatus,
    /// Preview and activate one exact policy tuple.
    PolicyActivate,
    /// Verify one existing local state database without mutation.
    StateVerify,
    /// Inspect one bounded lifecycle object.
    LifecycleInspect,
    /// Query one bounded local audit page.
    AuditQuery,
    /// Build one non-mutating local purge plan.
    PurgePlan,
    /// Confirm and execute one exact local purge plan.
    PurgeExecute,
}

impl OperationsCommand {
    /// Returns the canonical dotted protocol spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ConfigValidate => "config.validate",
            Self::PolicyStatus => "policy.status",
            Self::PolicyActivate => "policy.activate",
            Self::StateVerify => "state.verify",
            Self::LifecycleInspect => "lifecycle.inspect",
            Self::AuditQuery => "audit.query",
            Self::PurgePlan => "purge.plan",
            Self::PurgeExecute => "purge.execute",
        }
    }

    /// Parses a canonical or shell-style command spelling.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "config.validate" | "config-validate" | "config_validate" | "config" => {
                Some(Self::ConfigValidate)
            }
            "policy.status" | "policy-status" | "policy_status" => Some(Self::PolicyStatus),
            "policy.activate" | "policy-activate" | "policy_activate" => Some(Self::PolicyActivate),
            "state.verify" | "state-verify" | "state_verify" | "state" => Some(Self::StateVerify),
            "lifecycle.inspect" | "lifecycle-inspect" | "lifecycle_inspect" | "state.inspect"
            | "state-inspect" => Some(Self::LifecycleInspect),
            "audit.query" | "audit-query" | "audit_query" | "audit" => Some(Self::AuditQuery),
            "purge.plan" | "purge-plan" | "purge_plan" => Some(Self::PurgePlan),
            "purge.execute" | "purge-execute" | "purge_execute" | "purge" => {
                Some(Self::PurgeExecute)
            }
            _ => None,
        }
    }
}

impl std::fmt::Display for OperationsCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The strict outer protocol envelope.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationsEnvelope {
    /// The only accepted protocol version.
    pub protocol_version: u8,
    /// A command name; no command is selected by default.
    pub command: String,
    /// Command-specific structured input.
    pub input: Value,
}

/// Input for `config.validate`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigValidationInput {
    /// Exact repository scope to validate.
    pub repository_id: String,
    /// Optional explicit configuration path supplied by the process owner.
    #[serde(default)]
    pub config_path: Option<String>,
}

impl ConfigValidationInput {
    /// Returns the explicit path as a filesystem path for a domain port.
    #[must_use]
    pub fn path(&self) -> Option<PathBuf> {
        self.config_path.as_ref().map(PathBuf::from)
    }
}

/// Input for `policy.status`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyStatusInput {
    /// Exact repository scope.
    pub repository_id: String,
    /// Exact event type.
    pub event_type: String,
    /// Exact configured destination alias, never a raw channel ID.
    pub destination_alias: String,
    /// Exact severity.
    pub severity: String,
}

impl PolicyStatusInput {
    /// Converts the wire tuple into the policy owner's exact tuple type.
    #[must_use]
    pub fn tuple(&self) -> PolicyTuple {
        PolicyTuple::new(
            self.event_type.clone(),
            self.destination_alias.clone(),
            self.severity.clone(),
        )
    }
}

/// Input for `policy.activate`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyActivationInput {
    /// Exact repository scope.
    pub repository_id: String,
    /// Exact event type.
    pub event_type: String,
    /// Exact configured destination alias, never a raw channel ID.
    pub destination_alias: String,
    /// Exact severity.
    pub severity: String,
    /// Optional caller-selected stable activation identifier.
    #[serde(default)]
    pub activation_id: Option<String>,
    /// Canonical UTC activation timestamp supplied explicitly by the caller.
    pub activated_at: String,
}

impl PolicyActivationInput {
    /// Returns the exact policy tuple.
    #[must_use]
    pub fn tuple(&self) -> PolicyTuple {
        PolicyTuple::new(
            self.event_type.clone(),
            self.destination_alias.clone(),
            self.severity.clone(),
        )
    }
}

/// Input for `state.verify`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StateVerifyInput {
    /// Exact repository scope to verify in the existing database.
    pub repository_id: String,
    /// Existing local SQLite database path. The verifier never creates it.
    pub database_path: String,
    /// Optional expected `user_version`; the current schema version is used
    /// when the caller intentionally leaves it unspecified.
    #[serde(default)]
    pub expected_migration: Option<i64>,
}

/// Input for `lifecycle.inspect`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LifecycleInspectInput {
    /// Exact repository scope.
    pub repository_id: String,
    /// Stable object family, such as `draft` or `inbound_item`.
    pub object_type: String,
    /// Exact object identifier; omitted only for the repository object.
    #[serde(default)]
    pub object_id: Option<String>,
    /// Positive immutable revision for a `draft_revision` object.
    #[serde(default)]
    pub revision: Option<i64>,
    /// Explicit bounded page size. `limit` is an accepted protocol alias.
    #[serde(default)]
    pub page_size: Option<usize>,
    /// Alias for `page_size`; exactly one of the two must be supplied.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Opaque continuation returned by a prior lifecycle page.
    #[serde(default)]
    pub after: Option<String>,
    /// Explicit opt-in for retained content in a draft revision or inbound item.
    #[serde(default)]
    pub include_retained_content: bool,
}

impl LifecycleInspectInput {
    /// Returns the one explicit page bound.
    pub fn page_size(&self) -> Result<usize, RepoComError> {
        explicit_page_size(self.page_size, self.limit, "lifecycle")
    }
}

/// Input for `audit.query`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuditQueryInput {
    /// Exact repository scope.
    pub repository_id: String,
    /// Optional inclusive lower UTC time bound.
    #[serde(default)]
    pub occurred_from: Option<String>,
    /// Optional exclusive upper UTC time bound.
    #[serde(default)]
    pub occurred_before: Option<String>,
    /// Optional exact object family.
    #[serde(default)]
    pub object_type: Option<String>,
    /// Optional exact object identifier.
    #[serde(default)]
    pub object_id: Option<String>,
    /// Optional exact transition.
    #[serde(default)]
    pub transition: Option<String>,
    /// Explicit bounded page size. `limit` is an accepted protocol alias.
    #[serde(default)]
    pub page_size: Option<usize>,
    /// Alias for `page_size`; exactly one of the two must be supplied.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Optional continuation in `repository|occurred_at|audit_id` form.
    #[serde(default)]
    pub cursor: Option<String>,
}

impl AuditQueryInput {
    /// Returns the one explicit page bound.
    pub fn page_size(&self) -> Result<usize, RepoComError> {
        explicit_page_size(self.page_size, self.limit, "audit")
    }
}

/// Input for `purge.plan`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PurgePlanInput {
    /// Exact repository scope.
    pub repository_id: String,
    /// Local category: `content`, `metadata`, or `all`.
    pub scope: String,
    /// Canonical UTC cutoff, or the explicit Unix/UTC pair below.
    #[serde(default)]
    pub cutoff: Option<String>,
    /// Unix seconds for the cutoff.
    #[serde(default)]
    pub cutoff_unix_seconds: Option<u64>,
    /// Canonical UTC text for the cutoff.
    #[serde(default)]
    pub cutoff_utc: Option<String>,
    /// Optional expected current configuration hash.
    #[serde(default)]
    pub expected_config_hash: Option<String>,
}

impl PurgePlanInput {
    /// Converts the validated wire cutoff into the domain cutoff type.
    pub fn cutoff_value(&self) -> Result<PurgeCutoff, RepoComError> {
        cutoff_from_parts(
            self.cutoff.as_deref(),
            self.cutoff_unix_seconds,
            self.cutoff_utc.as_deref(),
        )
    }
}

/// Input for `purge.execute`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PurgeExecuteInput {
    /// Exact repository scope.
    pub repository_id: String,
    /// Local category to execute.
    pub scope: String,
    /// Canonical UTC cutoff, or the explicit Unix/UTC pair below.
    #[serde(default)]
    pub cutoff: Option<String>,
    /// Unix seconds for the cutoff.
    #[serde(default)]
    pub cutoff_unix_seconds: Option<u64>,
    /// Canonical UTC text for the cutoff.
    #[serde(default)]
    pub cutoff_utc: Option<String>,
    /// Configuration hash shown in the exact plan.
    pub config_hash: String,
    /// Deterministic plan hash confirmed by the operator.
    pub plan_hash: String,
    /// Canonical UTC execution timestamp supplied explicitly by the caller.
    pub executed_at: String,
}

impl PurgeExecuteInput {
    /// Converts the validated wire cutoff into the domain cutoff type.
    pub fn cutoff_value(&self) -> Result<PurgeCutoff, RepoComError> {
        cutoff_from_parts(
            self.cutoff.as_deref(),
            self.cutoff_unix_seconds,
            self.cutoff_utc.as_deref(),
        )
    }
}

/// A parsed and semantically validated operations command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OperationsInput {
    /// Configuration validation input.
    ConfigValidate(ConfigValidationInput),
    /// Exact policy status input.
    PolicyStatus(PolicyStatusInput),
    /// Exact policy activation input.
    PolicyActivate(PolicyActivationInput),
    /// Read-only state verification input.
    StateVerify(StateVerifyInput),
    /// Read-only lifecycle inspection input.
    LifecycleInspect(LifecycleInspectInput),
    /// Bounded audit query input.
    AuditQuery(AuditQueryInput),
    /// Non-mutating purge plan input.
    PurgePlan(PurgePlanInput),
    /// Confirmed purge execution input.
    PurgeExecute(PurgeExecuteInput),
}

impl OperationsInput {
    /// Returns the canonical command name.
    #[must_use]
    pub const fn command(&self) -> OperationsCommand {
        match self {
            Self::ConfigValidate(_) => OperationsCommand::ConfigValidate,
            Self::PolicyStatus(_) => OperationsCommand::PolicyStatus,
            Self::PolicyActivate(_) => OperationsCommand::PolicyActivate,
            Self::StateVerify(_) => OperationsCommand::StateVerify,
            Self::LifecycleInspect(_) => OperationsCommand::LifecycleInspect,
            Self::AuditQuery(_) => OperationsCommand::AuditQuery,
            Self::PurgePlan(_) => OperationsCommand::PurgePlan,
            Self::PurgeExecute(_) => OperationsCommand::PurgeExecute,
        }
    }
}

/// Parses a complete protocol-version-1 operations command.
pub fn parse(bytes: &[u8]) -> Result<OperationsInput, RepoComError> {
    let envelope: OperationsEnvelope =
        serde_json::from_slice(bytes).map_err(|_| protocol_shape_error())?;
    if envelope.protocol_version != PROTOCOL_VERSION {
        return Err(usage("unsupported operations protocol version"));
    }
    let Some(command) = OperationsCommand::parse(&envelope.command) else {
        return Err(usage("unknown operations command"));
    };
    let input = match command {
        OperationsCommand::ConfigValidate => {
            OperationsInput::ConfigValidate(decode(envelope.input)?)
        }
        OperationsCommand::PolicyStatus => OperationsInput::PolicyStatus(decode(envelope.input)?),
        OperationsCommand::PolicyActivate => {
            OperationsInput::PolicyActivate(decode(envelope.input)?)
        }
        OperationsCommand::StateVerify => OperationsInput::StateVerify(decode(envelope.input)?),
        OperationsCommand::LifecycleInspect => {
            OperationsInput::LifecycleInspect(decode(envelope.input)?)
        }
        OperationsCommand::AuditQuery => OperationsInput::AuditQuery(decode(envelope.input)?),
        OperationsCommand::PurgePlan => OperationsInput::PurgePlan(decode(envelope.input)?),
        OperationsCommand::PurgeExecute => OperationsInput::PurgeExecute(decode(envelope.input)?),
    };
    validate_input(&input)?;
    Ok(input)
}

/// Parses a complete command from UTF-8 text.
pub fn parse_str(value: &str) -> Result<OperationsInput, RepoComError> {
    parse(value.as_bytes())
}

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, RepoComError> {
    serde_json::from_value(value).map_err(|_| protocol_shape_error())
}

fn validate_input(input: &OperationsInput) -> Result<(), RepoComError> {
    match input {
        OperationsInput::ConfigValidate(value) => {
            validate_repository_id(&value.repository_id)?;
            validate_optional_path(value.config_path.as_deref(), "config_path")?;
        }
        OperationsInput::PolicyStatus(value) => {
            validate_repository_id(&value.repository_id)?;
            validate_tuple(&value.tuple())?;
        }
        OperationsInput::PolicyActivate(value) => {
            validate_repository_id(&value.repository_id)?;
            validate_tuple(&value.tuple())?;
            if let Some(activation_id) = &value.activation_id {
                validate_identifier(activation_id, "activation_id")?;
            }
            validate_timestamp(&value.activated_at, "activated_at")?;
        }
        OperationsInput::StateVerify(value) => {
            validate_repository_id(&value.repository_id)?;
            validate_path(&value.database_path, "database_path")?;
            if value.expected_migration.is_some_and(|version| version < 0) {
                return Err(usage("expected_migration must not be negative"));
            }
        }
        OperationsInput::LifecycleInspect(value) => {
            validate_repository_id(&value.repository_id)?;
            validate_lifecycle_input(value)?;
        }
        OperationsInput::AuditQuery(value) => {
            validate_repository_id(&value.repository_id)?;
            let page_size = value.page_size()?;
            validate_page_size(page_size, "audit")?;
            let cursor = value
                .cursor
                .as_deref()
                .map(|cursor| parse_audit_cursor(cursor, &value.repository_id))
                .transpose()?;
            let mut filter = AuditFilter::new(&value.repository_id).with_page_size(page_size);
            filter.occurred_from = value.occurred_from.clone();
            filter.occurred_before = value.occurred_before.clone();
            filter.object_type = value.object_type.clone();
            filter.object_id = value.object_id.clone();
            filter.transition = value.transition.clone();
            if let Some(cursor) = cursor {
                filter = filter.with_cursor(cursor);
            }
            filter
                .validate()
                .map_err(|_| usage("audit query filter is invalid"))?;
        }
        OperationsInput::PurgePlan(value) => {
            validate_repository_id(&value.repository_id)?;
            validate_scope(&value.scope)?;
            value.cutoff_value()?;
            if let Some(hash) = &value.expected_config_hash {
                validate_hash(hash, "expected_config_hash")?;
            }
        }
        OperationsInput::PurgeExecute(value) => {
            validate_repository_id(&value.repository_id)?;
            validate_scope(&value.scope)?;
            value.cutoff_value()?;
            validate_hash(&value.config_hash, "config_hash")?;
            validate_hash(&value.plan_hash, "plan_hash")?;
            validate_timestamp(&value.executed_at, "executed_at")?;
        }
    }
    Ok(())
}

fn validate_lifecycle_input(value: &LifecycleInspectInput) -> Result<(), RepoComError> {
    let object_type = value.object_type.as_str();
    if !matches!(
        object_type,
        "repository"
            | "draft"
            | "draft_revision"
            | "delivery_attempt"
            | "inbound_item"
            | "acknowledgement"
            | "archive"
            | "reply_link"
            | "audit_transition"
    ) {
        return Err(usage("lifecycle object_type is not supported"));
    }
    if object_type == "repository" || object_type == "audit_transition" {
        if value.object_id.is_some() || value.revision.is_some() {
            return Err(usage(format!(
                "{object_type} lifecycle inspection takes no object identifier"
            )));
        }
    } else {
        let object_id = value
            .object_id
            .as_deref()
            .ok_or_else(|| usage("an explicit lifecycle object_id is required"))?;
        validate_identifier(object_id, "object_id")?;
    }
    if object_type == "draft_revision" {
        if !value.revision.is_some_and(|revision| revision > 0) {
            return Err(usage(
                "draft_revision requires a positive explicit revision",
            ));
        }
    } else if value.revision.is_some() {
        return Err(usage("revision is only valid for draft_revision"));
    }
    let page_size = value.page_size()?;
    validate_page_size(page_size, "lifecycle")?;
    if let Some(after) = &value.after
        && (after.is_empty() || after.len() > 1_024 || after.chars().any(char::is_control))
    {
        return Err(usage("lifecycle continuation is invalid"));
    }
    Ok(())
}

fn explicit_page_size(
    page_size: Option<usize>,
    limit: Option<usize>,
    surface: &'static str,
) -> Result<usize, RepoComError> {
    match (page_size, limit) {
        (Some(_), Some(_)) => Err(usage(match surface {
            "audit" => "audit page_size and limit are mutually exclusive",
            _ => "lifecycle page_size and limit are mutually exclusive",
        })),
        (Some(value), None) | (None, Some(value)) => Ok(value),
        (None, None) => Err(usage(match surface {
            "audit" => "audit page_size is required",
            _ => "lifecycle page_size is required",
        })),
    }
}

fn validate_page_size(value: usize, surface: &'static str) -> Result<(), RepoComError> {
    if value == 0 || value > 100 {
        return Err(usage(match surface {
            "audit" => "audit page_size must be between 1 and 100",
            _ => "lifecycle page_size must be between 1 and 100",
        }));
    }
    Ok(())
}

fn validate_repository_id(value: &str) -> Result<(), RepoComError> {
    validate_identifier(value, "repository_id")
}

fn validate_identifier(value: &str, field: &'static str) -> Result<(), RepoComError> {
    let valid = !value.is_empty()
        && value.len() <= 512
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(
                    character,
                    '-' | '_' | '.' | '/' | ':' | '@' | '+' | '#' | '='
                )
        });
    if valid {
        Ok(())
    } else {
        Err(usage(match field {
            "repository_id" => "repository_id is not a valid explicit identifier",
            "activation_id" => "activation_id is not a valid explicit identifier",
            _ => "object_id is not a valid explicit identifier",
        }))
    }
}

fn validate_tuple(tuple: &PolicyTuple) -> Result<(), RepoComError> {
    tuple
        .validate()
        .map_err(|_| usage("policy tuple must contain exact non-wildcard components"))?;
    validate_identifier(&tuple.destination_alias, "destination_alias")
}

fn validate_scope(value: &str) -> Result<(), RepoComError> {
    match value {
        "content" | "metadata" | "all" => Ok(()),
        _ => Err(usage("purge scope must be content, metadata, or all")),
    }
}

fn validate_hash(value: &str, field: &'static str) -> Result<(), RepoComError> {
    if is_sha256_hex(value) {
        Ok(())
    } else {
        Err(usage(match field {
            "config_hash" => "config_hash must be a SHA-256 hash",
            "expected_config_hash" => "expected_config_hash must be a SHA-256 hash",
            _ => "plan_hash must be a SHA-256 hash",
        }))
    }
}

fn validate_timestamp(value: &str, field: &'static str) -> Result<(), RepoComError> {
    PurgeCutoff::from_rfc3339(value).map(|_| ()).map_err(|_| {
        usage(match field {
            "activated_at" => "activated_at must be a canonical UTC RFC 3339 timestamp",
            _ => "executed_at must be a canonical UTC RFC 3339 timestamp",
        })
    })
}

fn validate_optional_path(value: Option<&str>, field: &'static str) -> Result<(), RepoComError> {
    if let Some(value) = value {
        validate_path(value, field)?;
    }
    Ok(())
}

fn validate_path(value: &str, field: &'static str) -> Result<(), RepoComError> {
    if value.is_empty() || value.len() > 4_096 || value.contains('\0') {
        return Err(usage(match field {
            "config_path" => "config_path is not a valid explicit path",
            _ => "database_path is not a valid explicit path",
        }));
    }
    if value.chars().any(char::is_control) {
        return Err(usage(
            "an explicit filesystem path contains a control character",
        ));
    }
    Ok(())
}

fn cutoff_from_parts(
    cutoff: Option<&str>,
    unix_seconds: Option<u64>,
    utc: Option<&str>,
) -> Result<PurgeCutoff, RepoComError> {
    match (cutoff, unix_seconds, utc) {
        (Some(value), None, None) => {
            PurgeCutoff::from_rfc3339(value).map_err(|_| usage("purge cutoff is invalid"))
        }
        (None, Some(seconds), Some(text)) => PurgeCutoff::new(seconds, text)
            .map_err(|_| usage("purge cutoff representations do not agree")),
        _ => Err(usage(
            "exactly one canonical purge cutoff representation is required",
        )),
    }
}

fn parse_audit_cursor(value: &str, repository_id: &str) -> Result<AuditCursor, RepoComError> {
    let parts: Vec<&str> = value.split('|').collect();
    if parts.len() != 3 || parts[0] != repository_id {
        return Err(usage("audit cursor is invalid for this repository"));
    }
    let audit_id = parts[2]
        .parse::<i64>()
        .map_err(|_| usage("audit cursor is invalid"))?;
    if audit_id <= 0 {
        return Err(usage("audit cursor is invalid"));
    }
    AuditCursor::new(parts[0], parts[1], audit_id).map_err(|_| usage("audit cursor is invalid"))
}

fn protocol_shape_error() -> RepoComError {
    usage("structured input must be a strict protocol-version-1 command object")
}

fn usage(message: impl Into<String>) -> RepoComError {
    RepoComError::new(ErrorCategory::UsageOrSchema, message)
}
