use std::{error::Error, fmt};

use repo_com_audit::{AuditError, AuditEvent};
use repo_com_config::{ResolvedConfig, ResolvedDestination};
use repo_com_draft_model::{AuthorizedReplyReference, DraftMetadata, DraftPreview};
use repo_com_draft_safety::{SecretScanResult, SecretScanner};
use repo_com_foundation::{ErrorCategory, TtyMode};
use repo_com_policy::{PolicyDecision, PolicyError, PolicyTuple, evaluate_from_state};
use repo_com_state::{
    ApprovalInput as StateApprovalInput, AuditEventRecord, Repositories, StateError, StateStore,
    StateTransaction,
};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::r#override::{OverrideAuditMetadata, OverrideReasonCode, SecretOverrideRecord};

pub(crate) const AUDIT_SCHEMA_VERSION: u8 = 1;

/// Maximum approval lifetime after confirmation.
pub const APPROVAL_LIFETIME_SECONDS: u64 = 15 * 60;

/// A deterministic instant supplied by an injected clock.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalInstant {
    unix_seconds: u64,
    utc: String,
}

impl ApprovalInstant {
    /// Creates an instant from injected Unix seconds and canonical UTC audit
    /// text. The audit boundary validates the timestamp before use.
    pub fn new(unix_seconds: u64, utc: impl Into<String>) -> Result<Self, ApprovalError> {
        let utc = utc.into();
        AuditEvent::new(
            "clock-validation",
            "clock-validation",
            "clock",
            "clock",
            "validated",
            utc.clone(),
            "system",
            "valid",
        )
        .validate()
        .map_err(ApprovalError::Audit)?;
        Ok(Self { unix_seconds, utc })
    }

    /// Returns Unix seconds used for every expiry boundary.
    #[must_use]
    pub const fn unix_seconds(&self) -> u64 {
        self.unix_seconds
    }

    /// Returns canonical UTC text used in local audit evidence.
    #[must_use]
    pub fn utc(&self) -> &str {
        &self.utc
    }
}

/// Injected time source. Implementations must not infer authority from ambient
/// process state.
pub trait ApprovalClock {
    /// Returns the current deterministic instant.
    fn now(&self) -> ApprovalInstant;
}

/// A caller-collected exact-preview confirmation.
///
/// The prompt adapter may create this value only after presenting the complete
/// preview. The approval service independently rebuilds current state and
/// compares the bound preview hash before persisting authority.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperatorConfirmation {
    tty_mode: TtyMode,
    preview_hash: String,
}

impl OperatorConfirmation {
    /// Binds an explicit TTY decision to one complete preview. Non-TTY remains
    /// representable so both authority-creating paths can fail closed.
    #[must_use]
    pub fn from_preview(preview: &ApprovalPreview, tty_mode: TtyMode) -> Self {
        Self {
            tty_mode,
            preview_hash: preview.preview_hash().to_owned(),
        }
    }

    /// Creates a confirmation only when the explicit mode is interactive.
    pub fn confirmed(preview: &ApprovalPreview, tty_mode: TtyMode) -> Result<Self, ApprovalError> {
        if !tty_mode.is_tty() {
            return Err(ApprovalError::TtyRequired);
        }
        Ok(Self::from_preview(preview, tty_mode))
    }

    /// Returns the explicit caller-supplied TTY mode.
    #[must_use]
    pub const fn tty_mode(&self) -> TtyMode {
        self.tty_mode
    }

    /// Returns the exact preview hash accepted by the operator.
    #[must_use]
    pub fn preview_hash(&self) -> &str {
        &self.preview_hash
    }

    pub(crate) fn require_for(&self, preview: &ApprovalPreview) -> Result<(), ApprovalError> {
        if !self.tty_mode.is_tty() {
            return Err(ApprovalError::TtyRequired);
        }
        if self.preview_hash != preview.preview_hash {
            return Err(ApprovalError::ConfirmationMismatch);
        }
        Ok(())
    }
}

/// A deterministic current-state inconsistency that blocks approval creation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PreviewInvalidReason {
    /// The preview and current repository/configuration scope differ.
    RepositoryScopeChanged,
    /// The preview is not the draft's current immutable revision.
    CurrentRevisionChanged,
    /// The persisted immutable revision hash differs.
    RevisionContentChanged,
    /// Alias or resolved destination state differs from the revision snapshot.
    DestinationChanged,
    /// The final rendered text differs from the persisted immutable revision.
    ExactTextChanged,
    /// Persisted revision metadata differs from the preview metadata.
    MetadataChanged,
    /// The draft has already transitioned to a terminal remote-sent state.
    DraftAlreadySent,
    /// The draft is expired at the injected current time.
    DraftExpired,
}

/// One exact approval-bound field that no longer matches.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalBindingField {
    Repository,
    Draft,
    Revision,
    RevisionHash,
    ExactText,
    Metadata,
    Config,
    Destination,
    PolicyBasis,
    SafetyScan,
    Expiry,
    Preview,
    ApprovalId,
}

/// A typed reason an existing approval cannot be reused.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "kind", content = "field")]
pub enum ApprovalInvalidReason {
    /// No approval exists for the current exact revision.
    ApprovalMissing,
    /// Approval evidence exists but is not internally coherent.
    PersistenceInconsistent,
    /// The approval was explicitly revoked.
    ApprovalRevoked,
    /// The draft or 15-minute approval boundary has been reached.
    ApprovalExpired,
    /// A current secret finding has no exact TTY override.
    SecretOverrideRequired,
    /// Current state does not form a valid preview.
    PreviewInvalid(PreviewInvalidReason),
    /// A persisted approval hash no longer matches current state.
    BindingChanged(ApprovalBindingField),
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ApprovalDisposition {
    Valid(Box<ApprovalRecord>),
    Invalid(ApprovalInvalidReason),
}

/// A safe, typed approval failure. Error values never retain preview text,
/// metadata values, matched secrets, or authorization values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApprovalError {
    TtyRequired,
    ConfirmationMismatch,
    InvalidPreview(PreviewInvalidReason),
    DraftNotFound,
    RevisionNotFound,
    RevisionOutOfRange,
    SecretOverrideRequired,
    OverrideNotRequired,
    ApprovalRevoked,
    ApprovalConflict,
    Serialization,
    Policy(PolicyError),
    Audit(AuditError),
    State(StateError),
}

impl ApprovalError {
    /// Returns the stable foundation error category.
    #[must_use]
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::TtyRequired | Self::ConfirmationMismatch => ErrorCategory::OperatorActionRequired,
            Self::InvalidPreview(PreviewInvalidReason::DraftExpired)
            | Self::SecretOverrideRequired
            | Self::ApprovalRevoked
            | Self::ApprovalConflict => ErrorCategory::PolicyBlocked,
            Self::InvalidPreview(_)
            | Self::DraftNotFound
            | Self::RevisionNotFound
            | Self::RevisionOutOfRange
            | Self::OverrideNotRequired => ErrorCategory::UsageOrSchema,
            Self::State(_) => ErrorCategory::StorageIntegrity,
            Self::Policy(error) => error.category(),
            Self::Audit(error) if error.code() == "storage-integrity" => {
                ErrorCategory::StorageIntegrity
            }
            Self::Audit(_) | Self::Serialization => ErrorCategory::InternalFailure,
        }
    }
}

impl fmt::Display for ApprovalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TtyRequired => formatter.write_str("interactive approval requires a TTY"),
            Self::ConfirmationMismatch => {
                formatter.write_str("approval confirmation does not match the current preview")
            }
            Self::InvalidPreview(reason) => {
                write!(formatter, "approval preview is not current: {reason}")
            }
            Self::DraftNotFound => formatter.write_str("draft was not found in current state"),
            Self::RevisionNotFound => {
                formatter.write_str("draft revision was not found in current state")
            }
            Self::RevisionOutOfRange => {
                formatter.write_str("draft revision cannot be represented in local state")
            }
            Self::SecretOverrideRequired => {
                formatter.write_str("current secret finding requires an exact TTY override")
            }
            Self::OverrideNotRequired => {
                formatter.write_str("secret override is not valid for a clear scan")
            }
            Self::ApprovalRevoked => formatter.write_str("approval is revoked"),
            Self::ApprovalConflict => {
                formatter.write_str("approval identity is already bound to different evidence")
            }
            Self::Serialization => {
                formatter.write_str("approval evidence could not be serialized safely")
            }
            Self::Policy(error) => write!(formatter, "policy basis evaluation failed: {error}"),
            Self::Audit(error) => write!(formatter, "approval audit boundary failed: {error}"),
            Self::State(error) => write!(formatter, "approval state operation failed: {error}"),
        }
    }
}

impl Error for ApprovalError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Policy(error) => Some(error),
            Self::Audit(error) => Some(error),
            Self::State(error) => Some(error),
            _ => None,
        }
    }
}

impl From<PolicyError> for ApprovalError {
    fn from(error: PolicyError) -> Self {
        Self::Policy(error)
    }
}

impl From<AuditError> for ApprovalError {
    fn from(error: AuditError) -> Self {
        Self::Audit(error)
    }
}

impl From<StateError> for ApprovalError {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}

impl fmt::Display for PreviewInvalidReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::RepositoryScopeChanged => "repository-scope-changed",
            Self::CurrentRevisionChanged => "current-revision-changed",
            Self::RevisionContentChanged => "revision-content-changed",
            Self::ExactTextChanged => "exact-text-changed",
            Self::DestinationChanged => "destination-changed",
            Self::MetadataChanged => "metadata-changed",
            Self::DraftAlreadySent => "draft-already-sent",
            Self::DraftExpired => "draft-expired",
        };
        formatter.write_str(value)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ApprovalPreviewCore {
    repository_id: String,
    draft_id: String,
    revision: u64,
    revision_hash: String,
    config_hash: String,
    destination_hash: String,
    destination_alias: String,
    resolved_destination: ResolvedDestination,
    exact_text: String,
    exact_text_hash: String,
    metadata: DraftMetadata,
    metadata_hash: String,
    event_type: String,
    severity: String,
    reply_reference: Option<AuthorizedReplyReference>,
    policy_basis: PolicyDecision,
    policy_basis_hash: String,
    scan_result: SecretScanResult,
    scan_hash: String,
    draft_created_at_unix_seconds: u64,
    draft_expires_at_unix_seconds: u64,
}

/// Complete, non-mutating facts that an operator must see before confirmation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApprovalPreview {
    #[serde(flatten)]
    core: ApprovalPreviewCore,
    preview_hash: String,
}

impl ApprovalPreview {
    pub fn repository_id(&self) -> &str {
        &self.core.repository_id
    }

    pub fn draft_id(&self) -> &str {
        &self.core.draft_id
    }

    pub const fn revision(&self) -> u64 {
        self.core.revision
    }

    pub fn revision_hash(&self) -> &str {
        &self.core.revision_hash
    }

    pub fn config_hash(&self) -> &str {
        &self.core.config_hash
    }

    pub fn destination_hash(&self) -> &str {
        &self.core.destination_hash
    }

    pub fn destination_alias(&self) -> &str {
        &self.core.destination_alias
    }

    pub fn resolved_destination(&self) -> &ResolvedDestination {
        &self.core.resolved_destination
    }

    pub fn exact_text(&self) -> &str {
        &self.core.exact_text
    }

    pub fn exact_text_hash(&self) -> &str {
        &self.core.exact_text_hash
    }

    pub fn metadata(&self) -> &DraftMetadata {
        &self.core.metadata
    }

    pub fn metadata_hash(&self) -> &str {
        &self.core.metadata_hash
    }

    pub fn event_type(&self) -> &str {
        &self.core.event_type
    }

    pub fn severity(&self) -> &str {
        &self.core.severity
    }

    pub fn reply_reference(&self) -> Option<&AuthorizedReplyReference> {
        self.core.reply_reference.as_ref()
    }

    pub fn policy_basis(&self) -> &PolicyDecision {
        &self.core.policy_basis
    }

    pub fn policy_basis_hash(&self) -> &str {
        &self.core.policy_basis_hash
    }

    pub fn scan_result(&self) -> &SecretScanResult {
        &self.core.scan_result
    }

    pub fn scan_hash(&self) -> &str {
        &self.core.scan_hash
    }

    pub const fn draft_created_at_unix_seconds(&self) -> u64 {
        self.core.draft_created_at_unix_seconds
    }

    pub const fn draft_expires_at_unix_seconds(&self) -> u64 {
        self.core.draft_expires_at_unix_seconds
    }

    pub fn preview_hash(&self) -> &str {
        &self.preview_hash
    }
}

/// All hashes a delivery coordinator must repeat at its atomic boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApprovalRevalidation {
    pub repository_id: String,
    pub draft_id: String,
    pub revision: u64,
    pub revision_hash: String,
    pub exact_text_hash: String,
    pub metadata_hash: String,
    pub config_hash: String,
    pub destination_hash: String,
    pub policy_basis_hash: String,
    pub scan_hash: String,
    pub preview_hash: String,
    pub draft_expires_at_unix_seconds: u64,
    pub approval_hash: Option<String>,
}

impl ApprovalRevalidation {
    fn from_preview(preview: &ApprovalPreview) -> Self {
        Self {
            repository_id: preview.repository_id().to_owned(),
            draft_id: preview.draft_id().to_owned(),
            revision: preview.revision(),
            revision_hash: preview.revision_hash().to_owned(),
            exact_text_hash: preview.exact_text_hash().to_owned(),
            metadata_hash: preview.metadata_hash().to_owned(),
            config_hash: preview.config_hash().to_owned(),
            destination_hash: preview.destination_hash().to_owned(),
            policy_basis_hash: preview.policy_basis_hash().to_owned(),
            scan_hash: preview.scan_hash().to_owned(),
            preview_hash: preview.preview_hash().to_owned(),
            draft_expires_at_unix_seconds: preview.draft_expires_at_unix_seconds(),
            approval_hash: None,
        }
    }
}

/// A read-only, fail-closed current-state approval check.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApprovalCheck {
    disposition: ApprovalDisposition,
    revalidation: ApprovalRevalidation,
}

impl ApprovalCheck {
    pub const fn disposition(&self) -> &ApprovalDisposition {
        &self.disposition
    }

    pub const fn revalidation(&self) -> &ApprovalRevalidation {
        &self.revalidation
    }

    pub fn into_disposition(self) -> ApprovalDisposition {
        self.disposition
    }

    pub fn is_valid(&self) -> bool {
        matches!(self.disposition, ApprovalDisposition::Valid(_))
    }

    pub fn approval(&self) -> Option<&ApprovalRecord> {
        match &self.disposition {
            ApprovalDisposition::Valid(record) => Some(record.as_ref()),
            ApprovalDisposition::Invalid(_) => None,
        }
    }

    fn invalid(revalidation: ApprovalRevalidation, reason: ApprovalInvalidReason) -> Self {
        Self {
            disposition: ApprovalDisposition::Invalid(reason),
            revalidation,
        }
    }
}

/// Exact persisted approval authority and revalidation facts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApprovalRecord {
    pub schema_version: u8,
    pub approval_id: String,
    pub audit_event_id: String,
    pub repository_id: String,
    pub draft_id: String,
    pub revision: u64,
    pub revision_hash: String,
    pub exact_text_hash: String,
    pub metadata_hash: String,
    pub config_hash: String,
    pub destination_hash: String,
    pub policy_basis_hash: String,
    pub scan_hash: String,
    pub preview_hash: String,
    pub draft_expires_at_unix_seconds: u64,
    pub approved_at_unix_seconds: u64,
    pub approved_at_utc: String,
    pub expires_at_unix_seconds: u64,
    pub override_hash: Option<String>,
    pub actor_kind: String,
}

impl ApprovalRecord {
    pub fn hash(&self) -> Result<String, ApprovalError> {
        sha256_json(self)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct ApprovalAuditMetadata {
    schema_version: u8,
    approval: PersistedApproval,
}

impl ApprovalAuditMetadata {
    fn new(approval: ApprovalRecord) -> Self {
        Self {
            schema_version: AUDIT_SCHEMA_VERSION,
            approval: PersistedApproval::from_record(approval),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
struct PersistedApproval {
    schema_version: u8,
    approval_id: String,
    audit_event_id: String,
    repository_id: String,
    draft_id: String,
    revision: u64,
    revision_hash: String,
    exact_text_hash: String,
    metadata_hash: String,
    config_hash: String,
    destination_hash: String,
    policy_basis_hash: String,
    scan_hash: String,
    preview_hash: String,
    draft_expires_at_unix_seconds: u64,
    approved_at_unix_seconds: u64,
    expires_at_unix_seconds: u64,
    override_hash: Option<String>,
    actor_kind: String,
}

impl PersistedApproval {
    fn from_record(record: ApprovalRecord) -> Self {
        Self {
            schema_version: record.schema_version,
            approval_id: record.approval_id,
            audit_event_id: record.audit_event_id,
            repository_id: record.repository_id,
            draft_id: record.draft_id,
            revision: record.revision,
            revision_hash: record.revision_hash,
            exact_text_hash: record.exact_text_hash,
            metadata_hash: record.metadata_hash,
            config_hash: record.config_hash,
            destination_hash: record.destination_hash,
            policy_basis_hash: record.policy_basis_hash,
            scan_hash: record.scan_hash,
            preview_hash: record.preview_hash,
            draft_expires_at_unix_seconds: record.draft_expires_at_unix_seconds,
            approved_at_unix_seconds: record.approved_at_unix_seconds,
            expires_at_unix_seconds: record.expires_at_unix_seconds,
            override_hash: record.override_hash,
            actor_kind: record.actor_kind,
        }
    }

    fn into_record(self, approved_at_utc: String) -> ApprovalRecord {
        ApprovalRecord {
            schema_version: self.schema_version,
            approval_id: self.approval_id,
            audit_event_id: self.audit_event_id,
            repository_id: self.repository_id,
            draft_id: self.draft_id,
            revision: self.revision,
            revision_hash: self.revision_hash,
            exact_text_hash: self.exact_text_hash,
            metadata_hash: self.metadata_hash,
            config_hash: self.config_hash,
            destination_hash: self.destination_hash,
            policy_basis_hash: self.policy_basis_hash,
            scan_hash: self.scan_hash,
            preview_hash: self.preview_hash,
            draft_expires_at_unix_seconds: self.draft_expires_at_unix_seconds,
            approved_at_unix_seconds: self.approved_at_unix_seconds,
            approved_at_utc,
            expires_at_unix_seconds: self.expires_at_unix_seconds,
            override_hash: self.override_hash,
            actor_kind: self.actor_kind,
        }
    }
}

struct CurrentSnapshot {
    core: ApprovalPreviewCore,
    invalid_reason: Option<PreviewInvalidReason>,
}

/// Exact-revision approval service over repository-scoped local state.
pub struct ApprovalService {
    state: StateStore,
}

impl ApprovalService {
    #[must_use]
    pub const fn new(state: StateStore) -> Self {
        Self { state }
    }

    #[must_use]
    pub const fn state(&self) -> &StateStore {
        &self.state
    }

    pub const fn state_mut(&mut self) -> &mut StateStore {
        &mut self.state
    }

    #[must_use]
    pub fn into_state(self) -> StateStore {
        self.state
    }

    /// Builds a complete current preview without mutating state.
    pub fn preview<C: ApprovalClock>(
        &self,
        draft: &DraftPreview,
        config: &ResolvedConfig,
        clock: &C,
    ) -> Result<ApprovalPreview, ApprovalError> {
        self.preview_at(draft, config, &clock.now())
    }

    /// Builds a complete current preview at an explicitly injected instant.
    pub fn preview_at(
        &self,
        draft: &DraftPreview,
        config: &ResolvedConfig,
        now: &ApprovalInstant,
    ) -> Result<ApprovalPreview, ApprovalError> {
        let snapshot = self.current_snapshot(draft, config, now)?;
        if let Some(reason) = snapshot.invalid_reason {
            return Err(ApprovalError::InvalidPreview(reason));
        }
        let preview_hash = sha256_json(&snapshot.core)?;
        Ok(ApprovalPreview {
            core: snapshot.core,
            preview_hash,
        })
    }

    /// Records one exact approval after a TTY-bound complete-preview
    /// confirmation. Replaying the same confirmation is idempotent and never
    /// extends the original expiry.
    pub fn approve<C: ApprovalClock>(
        &mut self,
        draft: &DraftPreview,
        config: &ResolvedConfig,
        confirmation: &OperatorConfirmation,
        clock: &C,
    ) -> Result<ApprovalRecord, ApprovalError> {
        let now = clock.now();
        let preview = self.preview_at(draft, config, &now)?;
        confirmation.require_for(&preview)?;

        let override_hash = if preview.scan_result().is_blocked() {
            let override_record = self
                .load_override(&preview)?
                .ok_or(ApprovalError::SecretOverrideRequired)?;
            Some(override_record.hash()?)
        } else {
            None
        };

        let approval_limit = now.unix_seconds().saturating_add(APPROVAL_LIFETIME_SECONDS);
        let expires_at_unix_seconds = approval_limit.min(preview.draft_expires_at_unix_seconds());
        let record = ApprovalRecord {
            schema_version: AUDIT_SCHEMA_VERSION,
            approval_id: approval_id(&preview),
            audit_event_id: approval_audit_event_id(&preview),
            repository_id: preview.repository_id().to_owned(),
            draft_id: preview.draft_id().to_owned(),
            revision: preview.revision(),
            revision_hash: preview.revision_hash().to_owned(),
            exact_text_hash: preview.exact_text_hash().to_owned(),
            metadata_hash: preview.metadata_hash().to_owned(),
            config_hash: preview.config_hash().to_owned(),
            destination_hash: preview.destination_hash().to_owned(),
            policy_basis_hash: preview.policy_basis_hash().to_owned(),
            scan_hash: preview.scan_hash().to_owned(),
            preview_hash: preview.preview_hash().to_owned(),
            draft_expires_at_unix_seconds: preview.draft_expires_at_unix_seconds(),
            approved_at_unix_seconds: now.unix_seconds(),
            approved_at_utc: now.utc().to_owned(),
            expires_at_unix_seconds,
            override_hash: override_hash.clone(),
            actor_kind: "operator".to_owned(),
        };
        let audit_input = approval_audit_input(&record)?;
        let revision = state_revision(&preview)?;

        let transaction = self.state.begin_transaction()?;
        ensure_current_in_transaction(&transaction.repositories(), &preview)?;

        let mut rebind_existing = false;
        if let Some(existing) = transaction.repositories().approvals().get(
            preview.repository_id(),
            preview.draft_id(),
            revision,
        )? {
            if existing.approval_state == "revoked" {
                return Err(ApprovalError::ApprovalRevoked);
            }
            let event_id = approval_event_id_from_approval_id(&existing.approval_id)
                .ok_or(ApprovalError::ApprovalConflict)?;
            let event = transaction
                .repositories()
                .audit()
                .get(preview.repository_id(), &event_id)
                .map_err(ApprovalError::State)?
                .ok_or(ApprovalError::ApprovalConflict)?;
            let persisted = decode_approval_event(&event)?;
            if !state_approval_matches_record(&existing, &persisted) {
                return Err(ApprovalError::ApprovalConflict);
            }
            if approval_matches_preview(&persisted, &preview, override_hash.as_deref()) {
                transaction.commit()?;
                return Ok(persisted);
            }
            rebind_existing = true;
        }

        if transaction
            .repositories()
            .audit()
            .get(preview.repository_id(), &record.audit_event_id)?
            .is_some()
        {
            return Err(ApprovalError::ApprovalConflict);
        }

        if rebind_existing {
            replace_state_approval(&transaction, &record, revision)?;
        } else {
            let state_input = StateApprovalInput::new(
                &record.repository_id,
                &record.approval_id,
                &record.draft_id,
                revision,
                "operator",
                &record.approved_at_utc,
            );
            transaction
                .repositories()
                .approvals()
                .record(&state_input)?;
        }
        transaction.append_audit_event(&audit_input)?;
        transaction.commit()?;
        Ok(record)
    }

    /// Records a redacted exact-revision secret override after TTY confirmation.
    /// Only the closed reason code and non-secret hashes are audited.
    pub fn override_secret_finding<C: ApprovalClock>(
        &mut self,
        draft: &DraftPreview,
        config: &ResolvedConfig,
        reason_code: OverrideReasonCode,
        confirmation: &OperatorConfirmation,
        clock: &C,
    ) -> Result<SecretOverrideRecord, ApprovalError> {
        let now = clock.now();
        let preview = self.preview_at(draft, config, &now)?;
        confirmation.require_for(&preview)?;
        if !preview.scan_result().is_blocked() {
            return Err(ApprovalError::OverrideNotRequired);
        }

        let event_id = override_audit_event_id(&preview);
        let record = SecretOverrideRecord {
            schema_version: AUDIT_SCHEMA_VERSION,
            event_id: event_id.clone(),
            repository_id: preview.repository_id().to_owned(),
            draft_id: preview.draft_id().to_owned(),
            revision: preview.revision(),
            revision_hash: preview.revision_hash().to_owned(),
            scan_hash: preview.scan_hash().to_owned(),
            preview_hash: preview.preview_hash().to_owned(),
            reason_code,
            created_at_unix_seconds: now.unix_seconds(),
            created_at_utc: now.utc().to_owned(),
        };
        let audit_input = override_audit_input(&record)?;

        let transaction = self.state.begin_transaction()?;
        ensure_current_in_transaction(&transaction.repositories(), &preview)?;
        if let Some(existing) = transaction
            .repositories()
            .audit()
            .get(preview.repository_id(), &event_id)?
        {
            let persisted = decode_override_event(&existing)?;
            if !override_matches_preview(&persisted, &preview, reason_code) {
                return Err(ApprovalError::ApprovalConflict);
            }
            transaction.commit()?;
            return Ok(persisted);
        }
        transaction.append_audit_event(&audit_input)?;
        transaction.commit()?;
        Ok(record)
    }

    /// Re-evaluates an existing approval against current inputs without a TTY
    /// and without creating, consuming, or claiming permission.
    pub fn check_current<C: ApprovalClock>(
        &self,
        draft: &DraftPreview,
        config: &ResolvedConfig,
        clock: &C,
    ) -> Result<ApprovalCheck, ApprovalError> {
        let now = clock.now();
        let snapshot = self.current_snapshot(draft, config, &now)?;
        let preview_hash = sha256_json(&snapshot.core)?;
        let invalid_reason = snapshot.invalid_reason;
        let preview = ApprovalPreview {
            core: snapshot.core,
            preview_hash,
        };
        let mut revalidation = ApprovalRevalidation::from_preview(&preview);
        if let Some(reason) = invalid_reason {
            return Ok(ApprovalCheck::invalid(
                revalidation,
                ApprovalInvalidReason::PreviewInvalid(reason),
            ));
        }

        let revision = match state_revision(&preview) {
            Ok(revision) => revision,
            Err(_) => {
                return Ok(ApprovalCheck::invalid(
                    revalidation,
                    ApprovalInvalidReason::PersistenceInconsistent,
                ));
            }
        };
        let Some(existing) =
            self.state
                .approval(preview.repository_id(), preview.draft_id(), revision)?
        else {
            return Ok(ApprovalCheck::invalid(
                revalidation,
                ApprovalInvalidReason::ApprovalMissing,
            ));
        };
        if existing.approval_state == "revoked" {
            return Ok(ApprovalCheck::invalid(
                revalidation,
                ApprovalInvalidReason::ApprovalRevoked,
            ));
        }
        let event_id = approval_event_id_from_approval_id(&existing.approval_id)
            .ok_or(ApprovalError::ApprovalConflict)?;
        let Some(event) = self.state.audit_event(preview.repository_id(), &event_id)? else {
            return Ok(ApprovalCheck::invalid(
                revalidation,
                ApprovalInvalidReason::PersistenceInconsistent,
            ));
        };
        let persisted = match decode_approval_event(&event) {
            Ok(record) if state_approval_matches_record(&existing, &record) => record,
            _ => {
                return Ok(ApprovalCheck::invalid(
                    revalidation,
                    ApprovalInvalidReason::PersistenceInconsistent,
                ));
            }
        };

        let override_hash = if preview.scan_result().is_blocked() {
            match self.load_override(&preview) {
                Ok(Some(record)) => match record.hash() {
                    Ok(hash) => Some(hash),
                    Err(_) => {
                        return Ok(ApprovalCheck::invalid(
                            revalidation,
                            ApprovalInvalidReason::PersistenceInconsistent,
                        ));
                    }
                },
                Ok(None) => None,
                Err(_) => {
                    return Ok(ApprovalCheck::invalid(
                        revalidation,
                        ApprovalInvalidReason::PersistenceInconsistent,
                    ));
                }
            }
        } else {
            None
        };

        if let Some(field) =
            approval_binding_difference(&persisted, &preview, override_hash.as_deref())
        {
            return Ok(ApprovalCheck::invalid(
                revalidation,
                ApprovalInvalidReason::BindingChanged(field),
            ));
        }
        if !approval_expiry_is_coherent(&persisted) {
            return Ok(ApprovalCheck::invalid(
                revalidation,
                ApprovalInvalidReason::PersistenceInconsistent,
            ));
        }
        if now.unix_seconds() >= persisted.expires_at_unix_seconds {
            return Ok(ApprovalCheck::invalid(
                revalidation,
                ApprovalInvalidReason::ApprovalExpired,
            ));
        }
        if preview.scan_result().is_blocked() && persisted.override_hash.is_none() {
            return Ok(ApprovalCheck::invalid(
                revalidation,
                ApprovalInvalidReason::SecretOverrideRequired,
            ));
        }

        revalidation.approval_hash = Some(persisted.hash()?);
        Ok(ApprovalCheck {
            disposition: ApprovalDisposition::Valid(Box::new(persisted)),
            revalidation,
        })
    }

    fn current_snapshot(
        &self,
        draft: &DraftPreview,
        config: &ResolvedConfig,
        now: &ApprovalInstant,
    ) -> Result<CurrentSnapshot, ApprovalError> {
        let repository = self.state.require_repository(draft.repository_id())?;
        let draft_row = self
            .state
            .draft(draft.repository_id(), draft.draft_id())?
            .ok_or(ApprovalError::DraftNotFound)?;
        let stored_revision = u64::try_from(draft_row.current_revision)
            .map_err(|_| ApprovalError::RevisionOutOfRange)?;
        let revision_row = self
            .state
            .draft_revision(
                draft.repository_id(),
                draft.draft_id(),
                to_i64(draft.revision())?,
            )?
            .ok_or(ApprovalError::RevisionNotFound)?;

        let alias = draft.destination_alias().as_str();
        let current_destination = config.destination(alias).cloned();
        let mut invalid = Vec::new();
        if repository.repository_id != config.config.repository_id
            || repository.workspace_id != config.config.discord.workspace_id
            || draft.repository_id() != config.config.repository_id
        {
            invalid.push(PreviewInvalidReason::RepositoryScopeChanged);
        }
        if stored_revision != draft.revision() {
            invalid.push(PreviewInvalidReason::CurrentRevisionChanged);
        }
        if revision_row.content_hash != draft.content_hash() {
            invalid.push(PreviewInvalidReason::RevisionContentChanged);
        }
        if revision_row.body != draft.exact_text() {
            invalid.push(PreviewInvalidReason::ExactTextChanged);
        }
        if current_destination.as_ref() != Some(draft.resolved_destination()) {
            invalid.push(PreviewInvalidReason::DestinationChanged);
        }
        if draft_row.destination_alias != alias
            || revision_row.destination_alias != alias
            || serde_json::from_str::<ResolvedDestination>(&revision_row.resolved_destination)
                .ok()
                .as_ref()
                != Some(draft.resolved_destination())
        {
            invalid.push(PreviewInvalidReason::DestinationChanged);
        }
        let reply_item_id = draft
            .reply_reference()
            .map(|reference| reference.inbound_item_id());
        if draft_row.reply_to_inbound_item_id.as_deref() != reply_item_id
            || revision_row.reply_to_inbound_item_id.as_deref() != reply_item_id
        {
            invalid.push(PreviewInvalidReason::MetadataChanged);
        }
        if serde_json::from_str::<DraftMetadata>(&revision_row.metadata_json)
            .ok()
            .as_ref()
            != Some(draft.metadata())
        {
            invalid.push(PreviewInvalidReason::MetadataChanged);
        }
        if draft_row.status == "sent" || revision_row.lifecycle_state == "sent" {
            invalid.push(PreviewInvalidReason::DraftAlreadySent);
        }
        if draft_row.status == "expired"
            || revision_row.lifecycle_state == "expired"
            || draft.expiry().is_expired(now.unix_seconds())
        {
            invalid.push(PreviewInvalidReason::DraftExpired);
        }

        let policy_tuple = PolicyTuple::new(
            draft.event_type().as_str().to_owned(),
            alias.to_owned(),
            draft.severity().as_str().to_owned(),
        );
        let policy_basis = evaluate_from_state(&self.state, &config.config, &policy_tuple)?;
        let policy_basis_hash = sha256_json(&policy_basis)?;
        let scan_result = SecretScanner::new().scan_preview(draft);
        let scan_hash = sha256_json(&scan_result)?;
        let destination_hash = sha256_json(&DestinationBinding {
            current: current_destination.as_ref(),
            revision: draft.resolved_destination(),
        })?;
        let core = ApprovalPreviewCore {
            repository_id: draft.repository_id().to_owned(),
            draft_id: draft.draft_id().to_owned(),
            revision: draft.revision(),
            revision_hash: draft.content_hash().to_owned(),
            config_hash: config.canonical_hash(),
            destination_hash,
            destination_alias: alias.to_owned(),
            resolved_destination: current_destination
                .unwrap_or_else(|| draft.resolved_destination().clone()),
            exact_text: draft.exact_text().to_owned(),
            exact_text_hash: sha256_bytes(draft.exact_text().as_bytes()),
            metadata: draft.metadata().clone(),
            metadata_hash: sha256_json(draft.metadata())?,
            event_type: draft.event_type().as_str().to_owned(),
            severity: draft.severity().as_str().to_owned(),
            reply_reference: draft.reply_reference().cloned(),
            policy_basis,
            policy_basis_hash,
            scan_hash,
            scan_result,
            draft_created_at_unix_seconds: draft.expiry().created_at_unix_seconds(),
            draft_expires_at_unix_seconds: draft.expiry().expires_at_unix_seconds(),
        };
        Ok(CurrentSnapshot {
            core,
            invalid_reason: invalid.into_iter().next(),
        })
    }

    fn load_override(
        &self,
        preview: &ApprovalPreview,
    ) -> Result<Option<SecretOverrideRecord>, ApprovalError> {
        let event_id = override_audit_event_id(preview);
        let Some(event) = self.state.audit_event(preview.repository_id(), &event_id)? else {
            return Ok(None);
        };
        let record = decode_override_event(&event)?;
        if record.repository_id != preview.repository_id()
            || record.draft_id != preview.draft_id()
            || record.revision != preview.revision()
            || record.revision_hash != preview.revision_hash()
            || record.scan_hash != preview.scan_hash()
            || record.preview_hash != preview.preview_hash()
        {
            return Err(ApprovalError::ApprovalConflict);
        }
        Ok(Some(record))
    }
}

#[derive(Serialize)]
struct DestinationBinding<'a> {
    current: Option<&'a ResolvedDestination>,
    revision: &'a ResolvedDestination,
}

fn ensure_current_in_transaction(
    repositories: &Repositories<'_>,
    preview: &ApprovalPreview,
) -> Result<(), ApprovalError> {
    let revision = to_i64(preview.revision())?;
    let draft = repositories
        .drafts()
        .get(preview.repository_id(), preview.draft_id())?
        .ok_or(ApprovalError::DraftNotFound)?;
    let stored = repositories
        .drafts()
        .revision(preview.repository_id(), preview.draft_id(), revision)?
        .ok_or(ApprovalError::RevisionNotFound)?;
    if u64::try_from(draft.current_revision).ok() != Some(preview.revision())
        || stored.content_hash != preview.revision_hash()
    {
        return Err(ApprovalError::InvalidPreview(
            PreviewInvalidReason::CurrentRevisionChanged,
        ));
    }
    if stored.body != preview.exact_text() {
        return Err(ApprovalError::InvalidPreview(
            PreviewInvalidReason::ExactTextChanged,
        ));
    }
    Ok(())
}

fn replace_state_approval(
    transaction: &StateTransaction<'_>,
    record: &ApprovalRecord,
    revision: i64,
) -> Result<(), ApprovalError> {
    let changed = transaction
        .execute(
            "UPDATE approvals
             SET approval_id = ?3, approval_state = 'approved', actor_kind = ?4,
                 operator_reference = NULL, approved_at = ?5, revoked_at = NULL
             WHERE repository_id = ?1 AND draft_id = ?2 AND revision = ?6",
            params![
                record.repository_id,
                record.draft_id,
                record.approval_id,
                record.actor_kind,
                record.approved_at_utc,
                revision,
            ],
        )
        .map_err(|_| {
            ApprovalError::State(StateError::Transaction {
                message: "approval binding update failed".to_owned(),
            })
        })?;
    if changed != 1 {
        return Err(ApprovalError::ApprovalConflict);
    }
    Ok(())
}

fn approval_matches_preview(
    record: &ApprovalRecord,
    preview: &ApprovalPreview,
    override_hash: Option<&str>,
) -> bool {
    approval_binding_difference(record, preview, override_hash).is_none()
}

fn approval_binding_difference(
    record: &ApprovalRecord,
    preview: &ApprovalPreview,
    override_hash: Option<&str>,
) -> Option<ApprovalBindingField> {
    let comparisons = [
        (
            ApprovalBindingField::Repository,
            record.repository_id == preview.repository_id(),
        ),
        (
            ApprovalBindingField::Draft,
            record.draft_id == preview.draft_id(),
        ),
        (
            ApprovalBindingField::Revision,
            record.revision == preview.revision(),
        ),
        (
            ApprovalBindingField::RevisionHash,
            record.revision_hash == preview.revision_hash(),
        ),
        (
            ApprovalBindingField::ExactText,
            record.exact_text_hash == preview.exact_text_hash(),
        ),
        (
            ApprovalBindingField::Metadata,
            record.metadata_hash == preview.metadata_hash(),
        ),
        (
            ApprovalBindingField::Config,
            record.config_hash == preview.config_hash(),
        ),
        (
            ApprovalBindingField::Destination,
            record.destination_hash == preview.destination_hash(),
        ),
        (
            ApprovalBindingField::PolicyBasis,
            record.policy_basis_hash == preview.policy_basis_hash(),
        ),
        (
            ApprovalBindingField::SafetyScan,
            record.scan_hash == preview.scan_hash()
                && record.override_hash.as_deref() == override_hash,
        ),
        (
            ApprovalBindingField::Expiry,
            record.draft_expires_at_unix_seconds == preview.draft_expires_at_unix_seconds(),
        ),
        (
            ApprovalBindingField::Preview,
            record.preview_hash == preview.preview_hash(),
        ),
        (
            ApprovalBindingField::ApprovalId,
            record.approval_id == approval_id(preview),
        ),
    ];
    comparisons
        .into_iter()
        .find_map(|(field, matches)| (!matches).then_some(field))
}

fn approval_expiry_is_coherent(record: &ApprovalRecord) -> bool {
    let limit = record
        .approved_at_unix_seconds
        .saturating_add(APPROVAL_LIFETIME_SECONDS);
    record.expires_at_unix_seconds == limit.min(record.draft_expires_at_unix_seconds)
}

fn state_approval_matches_record(
    state: &repo_com_state::ApprovalRecord,
    record: &ApprovalRecord,
) -> bool {
    state.repository_id == record.repository_id
        && state.approval_id == record.approval_id
        && state.draft_id == record.draft_id
        && u64::try_from(state.revision).ok() == Some(record.revision)
        && state.approval_state == "approved"
        && state.actor_kind == record.actor_kind
        && state.approved_at == record.approved_at_utc
        && state.revoked_at.is_none()
}

fn state_revision(preview: &ApprovalPreview) -> Result<i64, ApprovalError> {
    to_i64(preview.revision())
}

fn to_i64(revision: u64) -> Result<i64, ApprovalError> {
    i64::try_from(revision).map_err(|_| ApprovalError::RevisionOutOfRange)
}

fn approval_id(preview: &ApprovalPreview) -> String {
    format!("approval-{}", preview.preview_hash())
}

fn approval_audit_event_id(preview: &ApprovalPreview) -> String {
    format!("approval-recorded-{}", preview.preview_hash())
}

fn approval_event_id_from_approval_id(approval_id: &str) -> Option<String> {
    approval_id
        .strip_prefix("approval-")
        .map(|hash| format!("approval-recorded-{hash}"))
}

fn override_audit_event_id(preview: &ApprovalPreview) -> String {
    format!("secret-override-{}", preview.preview_hash())
}

fn approval_audit_input(
    record: &ApprovalRecord,
) -> Result<repo_com_state::AuditEventInput, ApprovalError> {
    let event = AuditEvent::with_metadata(
        &record.repository_id,
        &record.audit_event_id,
        "draft",
        &record.draft_id,
        "approved",
        &record.approved_at_utc,
        "operator",
        "approved",
        serde_json::to_value(ApprovalAuditMetadata::new(record.clone()))
            .map_err(|_| ApprovalError::Serialization)?,
    );
    Ok(event.to_state_input()?)
}

fn override_audit_input(
    record: &SecretOverrideRecord,
) -> Result<repo_com_state::AuditEventInput, ApprovalError> {
    let event = AuditEvent::with_metadata(
        &record.repository_id,
        &record.event_id,
        "draft",
        &record.draft_id,
        "secret_finding_overridden",
        &record.created_at_utc,
        "operator",
        "overridden",
        serde_json::to_value(OverrideAuditMetadata::new(record.clone()))
            .map_err(|_| ApprovalError::Serialization)?,
    );
    Ok(event.to_state_input()?)
}

fn decode_approval_event(event: &AuditEventRecord) -> Result<ApprovalRecord, ApprovalError> {
    if event.transition != "approved"
        || event.actor_kind != "operator"
        || event.outcome != "approved"
    {
        return Err(ApprovalError::ApprovalConflict);
    }
    let metadata: ApprovalAuditMetadata =
        serde_json::from_str(&event.metadata_json).map_err(|_| ApprovalError::Serialization)?;
    if metadata.schema_version != AUDIT_SCHEMA_VERSION
        || metadata.approval.schema_version != AUDIT_SCHEMA_VERSION
        || metadata.approval.repository_id != event.repository_id
        || metadata.approval.draft_id != event.object_id
        || metadata.approval.audit_event_id != event.event_id
        || metadata.approval.actor_kind != event.actor_kind
        || metadata.approval.approval_id != format!("approval-{}", metadata.approval.preview_hash)
        || metadata.approval.audit_event_id
            != format!("approval-recorded-{}", metadata.approval.preview_hash)
    {
        return Err(ApprovalError::ApprovalConflict);
    }
    Ok(metadata.approval.into_record(event.occurred_at.clone()))
}

fn decode_override_event(event: &AuditEventRecord) -> Result<SecretOverrideRecord, ApprovalError> {
    if event.transition != "secret_finding_overridden"
        || event.actor_kind != "operator"
        || event.outcome != "overridden"
    {
        return Err(ApprovalError::ApprovalConflict);
    }
    let metadata: OverrideAuditMetadata =
        serde_json::from_str(&event.metadata_json).map_err(|_| ApprovalError::Serialization)?;
    if metadata.schema_version != AUDIT_SCHEMA_VERSION
        || metadata.override_record.schema_version != AUDIT_SCHEMA_VERSION
        || metadata.override_record.repository_id != event.repository_id
        || metadata.override_record.draft_id != event.object_id
        || metadata.override_record.event_id != event.event_id
        || metadata.override_record.event_id
            != format!("secret-override-{}", metadata.override_record.preview_hash)
    {
        return Err(ApprovalError::ApprovalConflict);
    }
    Ok(metadata
        .override_record
        .into_record(event.occurred_at.clone()))
}

fn override_matches_preview(
    record: &SecretOverrideRecord,
    preview: &ApprovalPreview,
    reason_code: OverrideReasonCode,
) -> bool {
    record.repository_id == preview.repository_id()
        && record.draft_id == preview.draft_id()
        && record.revision == preview.revision()
        && record.revision_hash == preview.revision_hash()
        && record.scan_hash == preview.scan_hash()
        && record.preview_hash == preview.preview_hash()
        && record.reason_code == reason_code
}

pub(crate) fn sha256_json<T: Serialize>(value: &T) -> Result<String, ApprovalError> {
    let bytes = serde_json::to_vec(value).map_err(|_| ApprovalError::Serialization)?;
    Ok(sha256_bytes(&bytes))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
