//! Owned, side-effect-free projections for outbound terminal presentation.
//!
//! These types are deliberately presentation projections.  They copy only the
//! safe facts needed to show a preview, approval, policy decision, safety
//! finding, or delivery result; they never retain a domain service or perform
//! I/O.  The optional `From` implementations make it straightforward for a
//! command adapter to project the existing typed domain results without
//! reimplementing their decisions.

use std::fmt;

use serde::{Deserialize, Serialize};

use repo_com_approval::{
    ApprovalBindingField, ApprovalCheck, ApprovalDisposition, ApprovalInvalidReason,
    ApprovalPreview, ApprovalRecord, PreviewInvalidReason, SecretOverrideRecord,
};
use repo_com_config::{ResolvedDestination, ResolvedMention};
use repo_com_delivery::{DeliveryAttempt, DeliveryState};
use repo_com_delivery_retry::{
    ReconciliationDecision, RetryDecision, RetryDelayKind, TransportOutcome,
};
use repo_com_draft_model::{AuthorizedReplyReference, DraftMetadata, DraftPreview};
use repo_com_draft_safety::{SecretFinding, SecretScanResult, SecretScanStatus};
use repo_com_foundation::ErrorCategory;
pub use repo_com_policy::PolicyTuple;
use repo_com_policy::{ActivationSnapshot, PolicyDecision, PolicyStatus};
use repo_com_send_eligibility::{EligibilityBlocker, EligibilityDecision};

/// The minimum approval lifetime is owned by the approval domain; this value
/// is only a display-friendly copy used when a domain result omits a separate
/// approval expiry.
pub const DEFAULT_APPROVAL_EXPIRY_SECONDS: u64 = 15 * 60;

/// Describes where a displayed fact came from without implying remote truth.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Provenance {
    /// A pure local projection of repository state or a domain decision.
    LocalDecision,
    /// A durable local delivery record.
    LocalDeliveryRecord,
    /// A transport observation, normally supplied by a mocked or remote adapter.
    RemoteObservation,
    /// A read-only reconciliation observation.
    ReconciliationObservation,
    /// A local error or integrity result.
    LocalError,
}

impl Provenance {
    /// Returns the stable text label used in human output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalDecision => "local decision",
            Self::LocalDeliveryRecord => "local delivery record",
            Self::RemoteObservation => "remote observation",
            Self::ReconciliationObservation => "read-only reconciliation observation",
            Self::LocalError => "local error",
        }
    }
}

impl fmt::Display for Provenance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A stable, fully resolved destination projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DestinationView {
    /// Repository-local alias selected by the draft.
    pub alias: String,
    /// The one configured workspace.
    pub workspace_id: String,
    /// The one resolved channel.
    pub channel_id: String,
    /// Named mentions allowed by the destination, in deterministic alias order.
    pub allowed_mentions: Vec<String>,
}

impl DestinationView {
    /// Creates a destination projection.
    #[must_use]
    pub fn new(
        alias: impl Into<String>,
        workspace_id: impl Into<String>,
        channel_id: impl Into<String>,
        allowed_mentions: Vec<String>,
    ) -> Self {
        Self {
            alias: alias.into(),
            workspace_id: workspace_id.into(),
            channel_id: channel_id.into(),
            allowed_mentions,
        }
    }

    /// Returns a complete one-line destination label.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{} (workspace {}, channel {})",
            self.alias, self.workspace_id, self.channel_id
        )
    }
}

impl From<&ResolvedDestination> for DestinationView {
    fn from(destination: &ResolvedDestination) -> Self {
        Self {
            alias: destination.alias.clone(),
            workspace_id: destination.workspace_id.clone(),
            channel_id: destination.channel_id.clone(),
            allowed_mentions: destination
                .allowed_mentions
                .iter()
                .map(resolved_mention_label)
                .collect(),
        }
    }
}

impl From<ResolvedDestination> for DestinationView {
    fn from(destination: ResolvedDestination) -> Self {
        Self::from(&destination)
    }
}

fn resolved_mention_label(mention: &ResolvedMention) -> String {
    format!("{}={}", mention.alias, mention.target)
}

/// Bounded draft metadata copied for a human-readable preview.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct MetadataView {
    /// Optional repository label.
    pub repository_label: Option<String>,
    /// Optional branch name.
    pub branch: Option<String>,
    /// Optional commit identifier.
    pub commit: Option<String>,
}

impl From<&DraftMetadata> for MetadataView {
    fn from(metadata: &DraftMetadata) -> Self {
        Self {
            repository_label: metadata.repository_label().map(str::to_owned),
            branch: metadata.branch().map(str::to_owned),
            commit: metadata.commit().map(str::to_owned),
        }
    }
}

impl From<DraftMetadata> for MetadataView {
    fn from(metadata: DraftMetadata) -> Self {
        Self::from(&metadata)
    }
}

/// Safe fields from a validated threaded reply reference.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReplyReferenceView {
    /// Repository containing the inbound item.
    pub repository_id: String,
    /// Workspace containing the referenced message.
    pub workspace_id: String,
    /// Channel containing the referenced message.
    pub channel_id: String,
    /// Local inbound item identity.
    pub inbound_item_id: String,
    /// Remote message identity.
    pub message_id: String,
    /// Local authorization reference.
    pub authorization_reference: String,
    /// Hash of the validated inbound snapshot.
    pub validated_snapshot_hash: String,
}

impl From<&AuthorizedReplyReference> for ReplyReferenceView {
    fn from(reference: &AuthorizedReplyReference) -> Self {
        Self {
            repository_id: reference.repository_id().to_owned(),
            workspace_id: reference.workspace_id().to_owned(),
            channel_id: reference.channel_id().to_owned(),
            inbound_item_id: reference.inbound_item_id().to_owned(),
            message_id: reference.message_id().to_owned(),
            authorization_reference: reference.authorization_reference().to_owned(),
            validated_snapshot_hash: reference.validated_snapshot_hash().to_owned(),
        }
    }
}

/// Stable state of the pre-send secret scan and any exact override.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum SafetyState {
    /// No high-confidence finding was observed.
    #[default]
    Clear,
    /// One or more redacted findings block sending by default.
    Finding,
    /// A finding is present and an exact TTY override has not been recorded.
    OverrideRequired,
    /// An exact redacted override is recorded for the preview.
    OverrideRecorded,
    /// The revision or authority is expired.
    Expired,
    /// The safety basis was not evaluated by the supplied projection.
    NotEvaluated,
}

impl SafetyState {
    /// Returns the stable text form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Clear => "clear",
            Self::Finding => "finding",
            Self::OverrideRequired => "override-required",
            Self::OverrideRecorded => "override-recorded",
            Self::Expired => "expired",
            Self::NotEvaluated => "not-evaluated",
        }
    }
}

impl fmt::Display for SafetyState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One redacted safety finding.  It intentionally contains no matched value or
/// source excerpt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SafetyFindingView {
    /// Stable scanner reason code.
    pub reason_code: String,
    /// Safe source label.
    pub source: String,
    /// Optional metadata field label.
    pub metadata_field: Option<String>,
    /// Inclusive byte start offset.
    pub start: usize,
    /// Exclusive byte end offset.
    pub end: usize,
    /// Safe display location.
    pub location: String,
}

impl From<&SecretFinding> for SafetyFindingView {
    fn from(finding: &SecretFinding) -> Self {
        let location = finding.location();
        Self {
            reason_code: finding.code().to_owned(),
            source: location.source().code().to_owned(),
            metadata_field: location.field().map(|field| field.code().to_owned()),
            start: location.start(),
            end: location.end(),
            location: location.to_string(),
        }
    }
}

impl From<SecretFinding> for SafetyFindingView {
    fn from(finding: SecretFinding) -> Self {
        Self::from(&finding)
    }
}

/// Complete safe safety/override projection.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SafetyView {
    /// Current safety state.
    pub state: SafetyState,
    /// Hash of the redacted scan result.
    pub scan_hash: Option<String>,
    /// Findings in deterministic scanner order.
    pub findings: Vec<SafetyFindingView>,
    /// Hash of an exact redacted override, when one exists.
    pub override_hash: Option<String>,
    /// Closed reason code for an exact override, when one exists.
    pub override_reason: Option<String>,
}

impl SafetyView {
    /// Creates a clear scan projection.
    #[must_use]
    pub fn clear(scan_hash: impl Into<String>) -> Self {
        Self {
            state: SafetyState::Clear,
            scan_hash: Some(scan_hash.into()),
            ..Self::default()
        }
    }

    /// Creates a scan projection from a domain scanner result.
    #[must_use]
    pub fn from_scan(scan_hash: impl Into<String>, result: &SecretScanResult) -> Self {
        let state = match result.status() {
            SecretScanStatus::Clear => SafetyState::Clear,
            SecretScanStatus::Finding => SafetyState::OverrideRequired,
        };
        Self {
            state,
            scan_hash: Some(scan_hash.into()),
            findings: result.iter().map(SafetyFindingView::from).collect(),
            ..Self::default()
        }
    }

    /// Adds the non-secret fields from an exact override record.
    #[must_use]
    pub fn with_override(mut self, record: &SecretOverrideRecord) -> Self {
        self.state = SafetyState::OverrideRecorded;
        self.override_hash = record.hash().ok();
        self.override_reason = Some(record.reason_code.code().to_owned());
        self
    }
}

/// State of an exact human approval projection.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalState {
    /// No approval record was supplied.
    #[default]
    Missing,
    /// A current exact approval record was supplied.
    Valid,
    /// The supplied approval is invalid for a reason other than expiry.
    Invalid,
    /// The supplied approval or draft is expired.
    Expired,
    /// A binding changed and the old approval cannot be reused.
    Stale,
    /// The approval was explicitly revoked.
    Revoked,
    /// A current approval exists but its safety basis is unresolved.
    OverrideRequired,
    /// The approval was not evaluated by this projection.
    NotEvaluated,
}

impl ApprovalState {
    /// Returns the stable text form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Valid => "valid",
            Self::Invalid => "invalid",
            Self::Expired => "expired",
            Self::Stale => "stale",
            Self::Revoked => "revoked",
            Self::OverrideRequired => "override-required",
            Self::NotEvaluated => "not-evaluated",
        }
    }
}

impl fmt::Display for ApprovalState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Complete safe approval fields, including every hash needed for revalidation.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ApprovalView {
    /// Current approval state.
    pub state: ApprovalState,
    /// Stable approval identifier.
    pub approval_id: Option<String>,
    /// Hash of the exact approval record.
    pub approval_hash: Option<String>,
    /// Hash of the complete preview reviewed by the operator.
    pub preview_hash: Option<String>,
    /// Immutable revision hash.
    pub revision_hash: Option<String>,
    /// Hash of exact final text.
    pub exact_text_hash: Option<String>,
    /// Hash of metadata.
    pub metadata_hash: Option<String>,
    /// Hash of current configuration.
    pub config_hash: Option<String>,
    /// Hash of current destination binding.
    pub destination_hash: Option<String>,
    /// Hash of the policy basis.
    pub policy_basis_hash: Option<String>,
    /// Hash of the redacted safety scan.
    pub scan_hash: Option<String>,
    /// Draft expiry boundary.
    pub draft_expires_at_unix_seconds: Option<u64>,
    /// Approval expiry boundary.
    pub expires_at_unix_seconds: Option<u64>,
    /// Override hash, when an exact safety override is bound.
    pub override_hash: Option<String>,
    /// Non-secret actor kind.
    pub actor_kind: Option<String>,
    /// Safe invalid/stale reason.
    pub reason: Option<String>,
}

impl ApprovalView {
    /// Creates a not-evaluated projection carrying preview binding hashes.
    #[must_use]
    pub fn from_preview(preview: &ApprovalPreview) -> Self {
        Self {
            state: if preview.scan_result().is_blocked() {
                ApprovalState::OverrideRequired
            } else {
                ApprovalState::NotEvaluated
            },
            preview_hash: Some(preview.preview_hash().to_owned()),
            revision_hash: Some(preview.revision_hash().to_owned()),
            exact_text_hash: Some(preview.exact_text_hash().to_owned()),
            metadata_hash: Some(preview.metadata_hash().to_owned()),
            config_hash: Some(preview.config_hash().to_owned()),
            destination_hash: Some(preview.destination_hash().to_owned()),
            policy_basis_hash: Some(preview.policy_basis_hash().to_owned()),
            scan_hash: Some(preview.scan_hash().to_owned()),
            draft_expires_at_unix_seconds: Some(preview.draft_expires_at_unix_seconds()),
            ..Self::default()
        }
    }

    /// Creates a projection from an exact approval record.
    #[must_use]
    pub fn from_record(record: &ApprovalRecord) -> Self {
        Self {
            state: ApprovalState::Valid,
            approval_id: Some(record.approval_id.clone()),
            approval_hash: record.hash().ok(),
            preview_hash: Some(record.preview_hash.clone()),
            revision_hash: Some(record.revision_hash.clone()),
            exact_text_hash: Some(record.exact_text_hash.clone()),
            metadata_hash: Some(record.metadata_hash.clone()),
            config_hash: Some(record.config_hash.clone()),
            destination_hash: Some(record.destination_hash.clone()),
            policy_basis_hash: Some(record.policy_basis_hash.clone()),
            scan_hash: Some(record.scan_hash.clone()),
            draft_expires_at_unix_seconds: Some(record.draft_expires_at_unix_seconds),
            expires_at_unix_seconds: Some(record.expires_at_unix_seconds),
            override_hash: record.override_hash.clone(),
            actor_kind: Some(record.actor_kind.clone()),
            reason: None,
        }
    }

    /// Creates a projection from a read-only current-state approval check.
    #[must_use]
    pub fn from_check(check: &ApprovalCheck) -> Self {
        let mut view = Self {
            state: match check.disposition() {
                ApprovalDisposition::Valid(_) => ApprovalState::Valid,
                ApprovalDisposition::Invalid(reason) => Self::state_for_invalid_reason(reason),
            },
            preview_hash: Some(check.revalidation().preview_hash.clone()),
            revision_hash: Some(check.revalidation().revision_hash.clone()),
            exact_text_hash: Some(check.revalidation().exact_text_hash.clone()),
            metadata_hash: Some(check.revalidation().metadata_hash.clone()),
            config_hash: Some(check.revalidation().config_hash.clone()),
            destination_hash: Some(check.revalidation().destination_hash.clone()),
            policy_basis_hash: Some(check.revalidation().policy_basis_hash.clone()),
            scan_hash: Some(check.revalidation().scan_hash.clone()),
            draft_expires_at_unix_seconds: Some(check.revalidation().draft_expires_at_unix_seconds),
            ..Self::default()
        };
        if let ApprovalDisposition::Valid(record) = check.disposition() {
            view.approval_id = Some(record.approval_id.clone());
            view.approval_hash = record.hash().ok();
            view.expires_at_unix_seconds = Some(record.expires_at_unix_seconds);
            view.override_hash = record.override_hash.clone();
            view.actor_kind = Some(record.actor_kind.clone());
        } else if let ApprovalDisposition::Invalid(reason) = check.disposition() {
            view.reason = Some(Self::reason_text(reason));
        }
        view
    }

    fn state_for_invalid_reason(reason: &ApprovalInvalidReason) -> ApprovalState {
        match reason {
            ApprovalInvalidReason::ApprovalExpired
            | ApprovalInvalidReason::PreviewInvalid(PreviewInvalidReason::DraftExpired) => {
                ApprovalState::Expired
            }
            ApprovalInvalidReason::ApprovalRevoked => ApprovalState::Revoked,
            ApprovalInvalidReason::SecretOverrideRequired => ApprovalState::OverrideRequired,
            ApprovalInvalidReason::BindingChanged(_) => ApprovalState::Stale,
            ApprovalInvalidReason::ApprovalMissing
            | ApprovalInvalidReason::PersistenceInconsistent
            | ApprovalInvalidReason::PreviewInvalid(_) => ApprovalState::Invalid,
        }
    }

    fn reason_text(reason: &ApprovalInvalidReason) -> String {
        match reason {
            ApprovalInvalidReason::BindingChanged(field) => {
                format!("binding changed: {}", approval_binding_field_text(*field))
            }
            ApprovalInvalidReason::PreviewInvalid(reason) => {
                format!("preview invalid: {reason}")
            }
            other => format!("{other:?}"),
        }
    }
}

fn approval_binding_field_text(field: ApprovalBindingField) -> &'static str {
    match field {
        ApprovalBindingField::Repository => "repository",
        ApprovalBindingField::Draft => "draft",
        ApprovalBindingField::Revision => "revision",
        ApprovalBindingField::RevisionHash => "revision-hash",
        ApprovalBindingField::ExactText => "exact-text",
        ApprovalBindingField::Metadata => "metadata",
        ApprovalBindingField::Config => "config",
        ApprovalBindingField::Destination => "destination",
        ApprovalBindingField::PolicyBasis => "policy-basis",
        ApprovalBindingField::SafetyScan => "safety-scan",
        ApprovalBindingField::Expiry => "expiry",
        ApprovalBindingField::Preview => "preview",
        ApprovalBindingField::ApprovalId => "approval-id",
    }
}

/// State of an exact auto-send policy projection.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolicyState {
    /// No policy decision was evaluated.
    #[default]
    NotEvaluated,
    /// The exact tuple is configured and not activated.
    NotActivated,
    /// The exact tuple is not configured.
    NotConfigured,
    /// One current exact activation exists.
    Active,
    /// An activation exists but its hash binding is stale.
    Stale,
    /// The matching activation was explicitly deactivated.
    Deactivated,
    /// More than one matching activation exists.
    Ambiguous,
}

impl PolicyState {
    /// Returns the stable text form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotEvaluated => "not-evaluated",
            Self::NotActivated => "not-activated",
            Self::NotConfigured => "not-configured",
            Self::Active => "active",
            Self::Stale => "stale",
            Self::Deactivated => "deactivated",
            Self::Ambiguous => "ambiguous",
        }
    }
}

impl fmt::Display for PolicyState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One complete activation snapshot preserved in an ambiguous policy status.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PolicyActivationView {
    /// Stable activation identifier.
    pub activation_id: String,
    /// Exact policy tuple.
    pub tuple: PolicyTuple,
    /// Configuration hash recorded by the activation.
    pub recorded_config_hash: String,
    /// Tuple hash recorded by the activation.
    pub recorded_tuple_hash: String,
    /// Current configuration hash.
    pub current_config_hash: String,
    /// Current tuple hash.
    pub current_tuple_hash: String,
    /// Activation timestamp.
    pub activated_at: String,
    /// Deactivation timestamp, when present.
    pub deactivated_at: Option<String>,
    /// Whether the stored row is active.
    pub active: bool,
    /// Safe stale reason.
    pub stale_reason: Option<String>,
}

impl From<&ActivationSnapshot> for PolicyActivationView {
    fn from(snapshot: &ActivationSnapshot) -> Self {
        Self {
            activation_id: snapshot.activation_id.clone(),
            tuple: snapshot.tuple.clone(),
            recorded_config_hash: snapshot.recorded_config_hash.clone(),
            recorded_tuple_hash: snapshot.recorded_tuple_hash.clone(),
            current_config_hash: snapshot.current_config_hash.clone(),
            current_tuple_hash: snapshot.current_tuple_hash.clone(),
            activated_at: snapshot.activated_at.clone(),
            deactivated_at: snapshot.deactivated_at.clone(),
            active: snapshot.active,
            stale_reason: snapshot.stale_reason.map(|reason| format!("{reason:?}")),
        }
    }
}

/// Complete policy status projection, including exact tuple and hash bindings.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct PolicyView {
    /// Current policy state.
    pub state: PolicyState,
    /// Exact event/destination/severity tuple.
    pub tuple: Option<PolicyTuple>,
    /// Current complete configuration hash.
    pub config_hash: Option<String>,
    /// Current exact tuple hash.
    pub tuple_hash: Option<String>,
    /// Stable activation identifier.
    pub activation_id: Option<String>,
    /// Configuration hash recorded by the activation.
    pub recorded_config_hash: Option<String>,
    /// Tuple hash recorded by the activation.
    pub recorded_tuple_hash: Option<String>,
    /// Current hash used for comparison.
    pub current_config_hash: Option<String>,
    /// Current tuple hash used for comparison.
    pub current_tuple_hash: Option<String>,
    /// Activation timestamp.
    pub activated_at: Option<String>,
    /// Deactivation timestamp.
    pub deactivated_at: Option<String>,
    /// Safe stale reason.
    pub stale_reason: Option<String>,
    /// Exact policy decision text supplied by the domain, if any.
    pub basis: Option<String>,
    /// Every matching activation snapshot, in deterministic activation order.
    pub activations: Vec<PolicyActivationView>,
}

impl PolicyView {
    /// Projects a policy status while preserving every activation snapshot.
    #[must_use]
    pub fn from_status(status: &PolicyStatus) -> Self {
        match status {
            PolicyStatus::NotConfigured => Self {
                state: PolicyState::NotConfigured,
                basis: Some("no exact configured policy".to_owned()),
                ..Self::default()
            },
            PolicyStatus::NotActivated => Self {
                state: PolicyState::NotActivated,
                basis: Some("configured policy has no activation".to_owned()),
                ..Self::default()
            },
            PolicyStatus::Active(snapshot) => Self::from_snapshot(PolicyState::Active, snapshot),
            PolicyStatus::Stale(snapshot) => Self::from_snapshot(PolicyState::Stale, snapshot),
            PolicyStatus::Deactivated(snapshot) => {
                Self::from_snapshot(PolicyState::Deactivated, snapshot)
            }
            PolicyStatus::Ambiguous { activations } => Self {
                state: PolicyState::Ambiguous,
                tuple: activations.first().map(|snapshot| snapshot.tuple.clone()),
                config_hash: activations
                    .first()
                    .map(|snapshot| snapshot.current_config_hash.clone()),
                tuple_hash: activations
                    .first()
                    .map(|snapshot| snapshot.current_tuple_hash.clone()),
                activation_id: activations
                    .first()
                    .map(|snapshot| snapshot.activation_id.clone()),
                recorded_config_hash: activations
                    .first()
                    .map(|snapshot| snapshot.recorded_config_hash.clone()),
                recorded_tuple_hash: activations
                    .first()
                    .map(|snapshot| snapshot.recorded_tuple_hash.clone()),
                current_config_hash: activations
                    .first()
                    .map(|snapshot| snapshot.current_config_hash.clone()),
                current_tuple_hash: activations
                    .first()
                    .map(|snapshot| snapshot.current_tuple_hash.clone()),
                activated_at: activations
                    .first()
                    .map(|snapshot| snapshot.activated_at.clone()),
                deactivated_at: activations
                    .first()
                    .and_then(|snapshot| snapshot.deactivated_at.clone()),
                stale_reason: activations
                    .first()
                    .and_then(|snapshot| snapshot.stale_reason.map(|reason| format!("{reason:?}"))),
                basis: Some(format!("{} matching activations", activations.len())),
                activations: activations.iter().map(PolicyActivationView::from).collect(),
            },
        }
    }

    /// Projects a policy decision into the same status-oriented view.
    #[must_use]
    pub fn from_decision(decision: &PolicyDecision) -> Self {
        match decision {
            PolicyDecision::Eligible(snapshot) => {
                Self::from_snapshot(PolicyState::Active, snapshot)
            }
            PolicyDecision::Stale(snapshot) => Self::from_snapshot(PolicyState::Stale, snapshot),
            PolicyDecision::NotActivated => Self {
                state: PolicyState::NotActivated,
                basis: Some("configured policy has no activation".to_owned()),
                ..Self::default()
            },
            PolicyDecision::NoExactMatch | PolicyDecision::NotConfigured => Self {
                state: PolicyState::NotConfigured,
                basis: Some("no exact configured policy".to_owned()),
                ..Self::default()
            },
            PolicyDecision::Deactivated(snapshot) => {
                Self::from_snapshot(PolicyState::Deactivated, snapshot)
            }
            PolicyDecision::Ambiguous { activations } => {
                Self::from_status(&PolicyStatus::Ambiguous {
                    activations: activations.clone(),
                })
            }
        }
    }

    fn from_snapshot(state: PolicyState, snapshot: &ActivationSnapshot) -> Self {
        Self {
            state,
            tuple: Some(snapshot.tuple.clone()),
            config_hash: Some(snapshot.current_config_hash.clone()),
            tuple_hash: Some(snapshot.current_tuple_hash.clone()),
            activation_id: Some(snapshot.activation_id.clone()),
            recorded_config_hash: Some(snapshot.recorded_config_hash.clone()),
            recorded_tuple_hash: Some(snapshot.recorded_tuple_hash.clone()),
            current_config_hash: Some(snapshot.current_config_hash.clone()),
            current_tuple_hash: Some(snapshot.current_tuple_hash.clone()),
            activated_at: Some(snapshot.activated_at.clone()),
            deactivated_at: snapshot.deactivated_at.clone(),
            stale_reason: snapshot.stale_reason.map(|reason| format!("{reason:?}")),
            basis: Some(match state {
                PolicyState::Active => "one current exact activation".to_owned(),
                PolicyState::Stale => "activation hash binding is stale".to_owned(),
                PolicyState::Deactivated => "activation was explicitly deactivated".to_owned(),
                _ => "policy state projected".to_owned(),
            }),
            activations: vec![PolicyActivationView::from(snapshot)],
        }
    }
}

/// Complete exact outbound preview data.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OutboundPreview {
    /// Repository identity.
    pub repository_id: String,
    /// Draft identity.
    pub draft_id: String,
    /// Immutable revision number.
    pub revision: u64,
    /// Immutable revision hash.
    pub revision_hash: String,
    /// Resolved destination snapshot.
    pub destination: DestinationView,
    /// Exact final outbound text, including any deterministic nonce footer.
    pub exact_text: String,
    /// Hash of the exact final outbound text.
    pub exact_text_hash: String,
    /// Bounded metadata.
    pub metadata: MetadataView,
    /// Event type used by policy.
    pub event_type: String,
    /// Severity used by policy.
    pub severity: String,
    /// Optional validated threaded reply reference.
    pub reply_reference: Option<ReplyReferenceView>,
    /// Draft creation time.
    pub created_at_unix_seconds: u64,
    /// Exclusive draft expiry boundary.
    pub expires_at_unix_seconds: u64,
    /// Exact approval projection.
    pub approval: ApprovalView,
    /// Exact policy projection.
    pub policy: PolicyView,
    /// Exact redacted safety projection.
    pub safety: SafetyView,
    /// Hash of the complete preview.
    pub preview_hash: String,
    /// Provenance of the preview.
    pub provenance: Provenance,
}

impl OutboundPreview {
    /// Projects the complete current approval preview into terminal data.
    #[must_use]
    pub fn from_approval_preview(preview: &ApprovalPreview) -> Self {
        Self {
            repository_id: preview.repository_id().to_owned(),
            draft_id: preview.draft_id().to_owned(),
            revision: preview.revision(),
            revision_hash: preview.revision_hash().to_owned(),
            destination: DestinationView::from(preview.resolved_destination()),
            exact_text: preview.exact_text().to_owned(),
            exact_text_hash: preview.exact_text_hash().to_owned(),
            metadata: MetadataView::from(preview.metadata()),
            event_type: preview.event_type().to_owned(),
            severity: preview.severity().to_owned(),
            reply_reference: preview.reply_reference().map(ReplyReferenceView::from),
            created_at_unix_seconds: preview.draft_created_at_unix_seconds(),
            expires_at_unix_seconds: preview.draft_expires_at_unix_seconds(),
            approval: ApprovalView::from_preview(preview),
            policy: PolicyView::from_decision(preview.policy_basis()),
            safety: SafetyView::from_scan(preview.scan_hash(), preview.scan_result()),
            preview_hash: preview.preview_hash().to_owned(),
            provenance: Provenance::LocalDecision,
        }
    }

    /// Projects a pure draft preview when approval, policy, and safety services
    /// have not yet supplied their facts.
    #[must_use]
    pub fn from_draft_preview(preview: &DraftPreview) -> Self {
        Self {
            repository_id: preview.repository_id().to_owned(),
            draft_id: preview.draft_id().to_owned(),
            revision: preview.revision(),
            revision_hash: preview.content_hash().to_owned(),
            destination: DestinationView::from(preview.resolved_destination()),
            exact_text: preview.exact_text().to_owned(),
            exact_text_hash: String::new(),
            metadata: MetadataView::from(preview.metadata()),
            event_type: preview.event_type().as_str().to_owned(),
            severity: preview.severity().as_str().to_owned(),
            reply_reference: preview.reply_reference().map(ReplyReferenceView::from),
            created_at_unix_seconds: preview.expiry().created_at_unix_seconds(),
            expires_at_unix_seconds: preview.expiry().expires_at_unix_seconds(),
            approval: ApprovalView {
                state: ApprovalState::NotEvaluated,
                ..ApprovalView::default()
            },
            policy: PolicyView::default(),
            safety: SafetyView {
                state: SafetyState::NotEvaluated,
                ..SafetyView::default()
            },
            preview_hash: String::new(),
            provenance: Provenance::LocalDecision,
        }
    }

    /// Returns whether the supplied Unix time has reached the exclusive expiry.
    #[must_use]
    pub const fn is_expired_at(&self, now_unix_seconds: u64) -> bool {
        now_unix_seconds >= self.expires_at_unix_seconds
    }

    /// Returns whether either the draft or its exact approval authority has
    /// reached an exclusive expiry boundary.
    #[must_use]
    pub fn is_prompt_expired_at(&self, now_unix_seconds: u64) -> bool {
        self.is_expired_at(now_unix_seconds)
            || self
                .approval
                .expires_at_unix_seconds
                .is_some_and(|expires| now_unix_seconds >= expires)
    }
}

/// A stable delivery result suitable for a complete terminal outcome view.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "outcome", rename_all = "kebab-case")]
pub enum DeliveryOutcome {
    /// No local claim exists.
    Unclaimed,
    /// A local claim exists but no transport result is present.
    Claimed,
    /// Discord returned a validated message identifier.
    Accepted {
        /// Remote message identifier.
        message_id: String,
    },
    /// The request was definitively rejected.
    Failed {
        /// Stable redacted error code.
        code: String,
    },
    /// A bounded retry wait is recorded.
    RetryWait {
        /// Next attempt number.
        next_attempt: u8,
        /// Wait in seconds, when known.
        delay_seconds: Option<u64>,
        /// Safe delay-kind label.
        delay_kind: String,
    },
    /// Dispatch may have reached the remote service and is not known.
    Unknown {
        /// Stable redacted reason.
        reason: String,
    },
    /// Reconciliation has not yet reached a safe absence or match decision.
    ReconciliationUnknown {
        /// Stable redacted reason.
        reason: String,
        /// Complete successful reads accumulated so far.
        successful_reads: u32,
        /// Observation-window start.
        observation_started_at_unix_seconds: u64,
        /// Most recent complete read timestamp.
        last_successful_read_at_unix_seconds: Option<u64>,
    },
    /// Reconciliation found one exact matching message.
    ReconciledAccepted {
        /// Remote message identifier.
        message_id: String,
        /// Number of complete successful reads, when retained by the caller.
        successful_reads: Option<u32>,
        /// Observation timestamp, when retained by the caller.
        observed_at_unix_seconds: Option<u64>,
    },
    /// Reconciliation conservatively proved absence.
    ReconciledAbsent {
        /// Number of complete successful reads, when retained by the caller.
        successful_reads: Option<u32>,
        /// Observation-window start, when retained by the caller.
        observation_started_at_unix_seconds: Option<u64>,
        /// Timestamp at which the absence gate completed, when retained by the caller.
        observed_at_unix_seconds: Option<u64>,
    },
    /// Reconciliation could not safely decide.
    Unresolved {
        /// Stable redacted reason.
        reason: String,
        /// Successful reads retained for later observation, when available.
        successful_reads: Option<u32>,
    },
    /// A pure eligibility decision rejected the send.
    EligibilityRejected {
        /// Stable eligibility blocker.
        blocker: String,
    },
    /// The exact draft or approval expired.
    Expired,
    /// A previously valid authority is stale or revoked.
    StaleAuthority {
        /// Safe stale reason.
        reason: String,
    },
    /// An operational or integrity error prevented a safe send result.
    Error {
        /// Stable foundation error category.
        category: String,
        /// Redacted detail.
        detail: String,
    },
}

impl DeliveryOutcome {
    /// Returns a concise stable text label.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Unclaimed => "unclaimed",
            Self::Claimed => "claimed",
            Self::Accepted { .. } => "accepted",
            Self::Failed { .. } => "failed",
            Self::RetryWait { .. } => "retry-wait",
            Self::Unknown { .. } | Self::ReconciliationUnknown { .. } => "unknown",
            Self::ReconciledAccepted { .. } => "reconciled-accepted",
            Self::ReconciledAbsent { .. } => "reconciled-absent",
            Self::Unresolved { .. } => "unresolved",
            Self::EligibilityRejected { .. } => "eligibility-rejected",
            Self::Expired => "expired",
            Self::StaleAuthority { .. } => "stale-authority",
            Self::Error { .. } => "error",
        }
    }

    /// Returns the stable foundation category for a non-success or blocked
    /// outcome, when one applies.  The renderer never invents a process result;
    /// it only exposes the category the command layer can use for its envelope.
    #[must_use]
    pub fn error_category(&self) -> Option<ErrorCategory> {
        match self {
            Self::Accepted { .. }
            | Self::Claimed
            | Self::Unclaimed
            | Self::ReconciledAccepted { .. } => None,
            Self::Failed { code } => {
                Some(ErrorCategory::from_code(code).unwrap_or(ErrorCategory::InternalFailure))
            }
            Self::RetryWait { .. } => Some(ErrorCategory::ConnectivityRateLimit),
            Self::Unknown { .. } | Self::ReconciliationUnknown { .. } | Self::Unresolved { .. } => {
                Some(ErrorCategory::UnknownDelivery)
            }
            Self::EligibilityRejected { blocker } => {
                Some(if blocker == "operator-action-required" {
                    ErrorCategory::OperatorActionRequired
                } else {
                    ErrorCategory::PolicyBlocked
                })
            }
            Self::Expired | Self::StaleAuthority { .. } => Some(ErrorCategory::PolicyBlocked),
            Self::ReconciledAbsent { .. } => Some(ErrorCategory::UnknownDelivery),
            Self::Error { category, .. } => {
                Some(ErrorCategory::from_code(category).unwrap_or(ErrorCategory::InternalFailure))
            }
        }
    }

    /// Returns the deterministic process exit code when the outcome maps to a
    /// stable error category.
    #[must_use]
    pub fn exit_code(&self) -> Option<i32> {
        self.error_category().map(ErrorCategory::exit_code)
    }

    /// Projects a transport outcome without inventing identity fields.
    #[must_use]
    pub fn from_transport(outcome: &TransportOutcome) -> Self {
        match outcome {
            TransportOutcome::Accepted { message_id } => Self::Accepted {
                message_id: message_id.clone(),
            },
            TransportOutcome::DefinitiveFailure { code } => Self::Failed { code: code.clone() },
            TransportOutcome::RateLimited { retry_after } => Self::RetryWait {
                next_attempt: 2,
                delay_seconds: retry_after.map(|delay| delay.as_secs()),
                delay_kind: "discord-directed".to_owned(),
            },
            TransportOutcome::PreDispatch { failure } => Self::RetryWait {
                next_attempt: 2,
                delay_seconds: None,
                delay_kind: format!("pre-dispatch-{}", failure_code(failure)),
            },
            TransportOutcome::Unknown { reason } => Self::Unknown {
                reason: reason.code().to_owned(),
            },
        }
    }

    /// Projects a retry classification without inventing a remote result.
    #[must_use]
    pub fn from_retry_decision(decision: &RetryDecision) -> Self {
        match decision {
            RetryDecision::Accepted { message_id } => Self::Accepted {
                message_id: message_id.clone(),
            },
            RetryDecision::Failed { code } => Self::Failed { code: code.clone() },
            RetryDecision::Wait {
                next_attempt,
                delay,
                kind,
            } => Self::RetryWait {
                next_attempt: *next_attempt,
                delay_seconds: Some(delay.as_secs()),
                delay_kind: retry_delay_kind_text(*kind).to_owned(),
            },
            RetryDecision::Unknown { reason } => Self::Unknown {
                reason: reason.code().to_owned(),
            },
            RetryDecision::Stop { reason, code } => Self::Failed {
                code: format!("{code}:{reason:?}"),
            },
        }
    }
}

/// Complete identity, basis, safety, and delivery result projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeliveryView {
    /// Repository identity.
    pub repository_id: String,
    /// Draft identity.
    pub draft_id: String,
    /// Immutable revision number.
    pub revision: u64,
    /// Immutable revision hash.
    pub revision_hash: String,
    /// Resolved destination.
    pub destination: DestinationView,
    /// Exact outbound content when the domain result retains it.
    pub exact_text: Option<String>,
    /// Hash of exact outbound content.
    pub exact_text_hash: Option<String>,
    /// Durable per-attempt request nonce, when a claimed attempt is available.
    pub request_nonce: Option<String>,
    /// Deterministic content nonce rendered into the exact message.
    pub content_nonce: Option<String>,
    /// Bounded metadata when supplied by the caller.
    pub metadata: MetadataView,
    /// Draft expiry.
    pub expires_at_unix_seconds: u64,
    /// Approval basis.
    pub approval: ApprovalView,
    /// Policy basis.
    pub policy: PolicyView,
    /// Safety basis.
    pub safety: SafetyView,
    /// Attempt identifier, when claimed.
    pub attempt_id: Option<String>,
    /// Attempt number, when claimed.
    pub attempt_number: Option<u64>,
    /// Current delivery result.
    pub outcome: DeliveryOutcome,
    /// Provenance of the result.
    pub provenance: Provenance,
    /// Explicit next action.
    pub next_action: String,
}

impl DeliveryView {
    /// Creates a delivery projection with safe defaults for optional facts.
    #[must_use]
    pub fn new(
        repository_id: impl Into<String>,
        draft_id: impl Into<String>,
        revision: u64,
        destination: DestinationView,
        outcome: DeliveryOutcome,
    ) -> Self {
        let next_action = default_next_action(&outcome);
        Self {
            repository_id: repository_id.into(),
            draft_id: draft_id.into(),
            revision,
            revision_hash: String::new(),
            destination,
            exact_text: None,
            exact_text_hash: None,
            request_nonce: None,
            content_nonce: None,
            metadata: MetadataView::default(),
            expires_at_unix_seconds: 0,
            approval: ApprovalView::default(),
            policy: PolicyView::default(),
            safety: SafetyView::default(),
            attempt_id: None,
            attempt_number: None,
            outcome,
            provenance: Provenance::LocalDeliveryRecord,
            next_action,
        }
    }

    /// Projects a durable local delivery attempt without performing I/O.
    #[must_use]
    pub fn from_attempt(attempt: &DeliveryAttempt) -> Self {
        let outcome = match attempt.state {
            DeliveryState::Unclaimed => DeliveryOutcome::Unclaimed,
            DeliveryState::Claimed => DeliveryOutcome::Claimed,
            DeliveryState::Accepted => DeliveryOutcome::Accepted {
                message_id: attempt
                    .remote_message_id
                    .clone()
                    .unwrap_or_else(|| "not-recorded".to_owned()),
            },
            DeliveryState::Failed => DeliveryOutcome::Failed {
                code: attempt
                    .error_code
                    .clone()
                    .unwrap_or_else(|| "delivery-failed".to_owned()),
            },
            DeliveryState::RetryWait => DeliveryOutcome::RetryWait {
                next_attempt: u8::try_from(attempt.attempt_number.saturating_add(1))
                    .unwrap_or(u8::MAX),
                delay_seconds: None,
                delay_kind: "recorded-retry-wait".to_owned(),
            },
            DeliveryState::Unknown => DeliveryOutcome::Unknown {
                reason: attempt
                    .error_code
                    .clone()
                    .unwrap_or_else(|| "delivery-outcome-unknown".to_owned()),
            },
            DeliveryState::ReconciledAccepted => DeliveryOutcome::ReconciledAccepted {
                message_id: attempt
                    .remote_message_id
                    .clone()
                    .unwrap_or_else(|| "not-recorded".to_owned()),
                successful_reads: None,
                observed_at_unix_seconds: None,
            },
            DeliveryState::ReconciledAbsent => DeliveryOutcome::ReconciledAbsent {
                successful_reads: None,
                observation_started_at_unix_seconds: None,
                observed_at_unix_seconds: None,
            },
            DeliveryState::Unresolved => DeliveryOutcome::Unresolved {
                reason: attempt
                    .error_code
                    .clone()
                    .unwrap_or_else(|| "reconciliation-unresolved".to_owned()),
                successful_reads: None,
            },
        };
        let mut view = Self::new(
            attempt.repository_id.clone(),
            attempt.draft_id.clone(),
            attempt.revision,
            DestinationView::from(&attempt.resolved_destination),
            outcome,
        );
        view.revision_hash = attempt.revision_hash.clone();
        view.exact_text = Some(attempt.exact_content.clone());
        view.exact_text_hash = None;
        view.request_nonce = Some(attempt.request_nonce.clone());
        view.content_nonce = Some(attempt.content_nonce.clone());
        view.expires_at_unix_seconds = 0;
        view.approval = ApprovalView {
            state: ApprovalState::NotEvaluated,
            revision_hash: Some(attempt.revision_hash.clone()),
            destination_hash: Some(attempt.destination_hash.clone()),
            scan_hash: Some(attempt.scan_hash.clone()),
            ..ApprovalView::default()
        };
        view.safety = SafetyView {
            state: SafetyState::NotEvaluated,
            scan_hash: Some(attempt.scan_hash.clone()),
            ..SafetyView::default()
        };
        view.attempt_id = Some(attempt.attempt_id.clone());
        view.attempt_number = Some(attempt.attempt_number);
        view.provenance = Provenance::LocalDeliveryRecord;
        view.next_action = default_next_action(&view.outcome);
        view
    }

    /// Projects a read-only reconciliation result into a delivery view.
    #[must_use]
    pub fn from_reconciliation(
        repository_id: impl Into<String>,
        draft_id: impl Into<String>,
        revision: u64,
        destination: DestinationView,
        decision: &ReconciliationDecision,
    ) -> Self {
        let outcome = match decision {
            ReconciliationDecision::Accepted {
                message_id,
                successful_reads,
                observed_at_unix_seconds,
            } => DeliveryOutcome::ReconciledAccepted {
                message_id: message_id.clone(),
                successful_reads: Some(*successful_reads),
                observed_at_unix_seconds: Some(*observed_at_unix_seconds),
            },
            ReconciliationDecision::Unknown {
                successful_reads,
                observation_started_at_unix_seconds,
                last_successful_read_at_unix_seconds,
                reason,
            } => DeliveryOutcome::ReconciliationUnknown {
                reason: reason.code().to_owned(),
                successful_reads: *successful_reads,
                observation_started_at_unix_seconds: *observation_started_at_unix_seconds,
                last_successful_read_at_unix_seconds: *last_successful_read_at_unix_seconds,
            },
            ReconciliationDecision::ReconciledAbsent {
                successful_reads,
                observation_started_at_unix_seconds,
                observed_at_unix_seconds,
            } => DeliveryOutcome::ReconciledAbsent {
                successful_reads: Some(*successful_reads),
                observation_started_at_unix_seconds: Some(*observation_started_at_unix_seconds),
                observed_at_unix_seconds: Some(*observed_at_unix_seconds),
            },
            ReconciliationDecision::Unresolved {
                reason,
                successful_reads,
            } => DeliveryOutcome::Unresolved {
                reason: reason.code().to_owned(),
                successful_reads: Some(*successful_reads),
            },
        };
        let mut view = Self::new(repository_id, draft_id, revision, destination, outcome);
        view.provenance = Provenance::ReconciliationObservation;
        view.next_action = default_next_action(&view.outcome);
        view
    }

    /// Projects a blocked pure eligibility decision.
    #[must_use]
    pub fn from_eligibility(
        decision: &EligibilityDecision,
        repository_id: impl Into<String>,
        draft_id: impl Into<String>,
        revision: u64,
        destination: DestinationView,
    ) -> Self {
        let outcome = match decision {
            EligibilityDecision::Eligible { .. } => DeliveryOutcome::Claimed,
            EligibilityDecision::Blocked { blocker, .. } => match blocker {
                EligibilityBlocker::DraftExpired | EligibilityBlocker::ApprovalExpired => {
                    DeliveryOutcome::Expired
                }
                EligibilityBlocker::StalePolicyActivation
                | EligibilityBlocker::ApprovalStateChanged
                | EligibilityBlocker::ConfigChanged
                | EligibilityBlocker::DestinationChanged => DeliveryOutcome::StaleAuthority {
                    reason: blocker.code().to_owned(),
                },
                other => DeliveryOutcome::EligibilityRejected {
                    blocker: other.code().to_owned(),
                },
            },
        };
        let mut view = Self::new(repository_id, draft_id, revision, destination, outcome);
        view.revision_hash = decision.revalidation().revision_hash.clone();
        view.approval = ApprovalView {
            state: match decision {
                EligibilityDecision::Eligible { .. } => ApprovalState::Valid,
                EligibilityDecision::Blocked { blocker, .. } => match blocker {
                    EligibilityBlocker::ApprovalExpired => ApprovalState::Expired,
                    EligibilityBlocker::ApprovalStateChanged => ApprovalState::Stale,
                    EligibilityBlocker::UnresolvedSecretFinding => ApprovalState::OverrideRequired,
                    _ => ApprovalState::Invalid,
                },
            },
            revision_hash: Some(decision.revalidation().revision_hash.clone()),
            exact_text_hash: Some(decision.revalidation().exact_text_hash.clone()),
            metadata_hash: Some(decision.revalidation().metadata_hash.clone()),
            config_hash: Some(decision.revalidation().config_hash.clone()),
            destination_hash: Some(decision.revalidation().destination_hash.clone()),
            policy_basis_hash: Some(decision.revalidation().policy_basis_hash.clone()),
            scan_hash: Some(decision.revalidation().scan_hash.clone()),
            preview_hash: Some(decision.revalidation().approval_preview_hash.clone()),
            draft_expires_at_unix_seconds: Some(
                decision.revalidation().draft_expires_at_unix_seconds,
            ),
            ..ApprovalView::default()
        };
        view.policy = PolicyView {
            state: match decision {
                EligibilityDecision::Eligible { .. } => PolicyState::Active,
                EligibilityDecision::Blocked { blocker, .. } => match blocker {
                    EligibilityBlocker::StalePolicyActivation
                    | EligibilityBlocker::PolicyAmbiguous => PolicyState::Stale,
                    EligibilityBlocker::PolicyNotActive => PolicyState::NotActivated,
                    _ => PolicyState::NotEvaluated,
                },
            },
            ..PolicyView::default()
        };
        view.safety = SafetyView {
            state: match decision {
                EligibilityDecision::Blocked {
                    blocker: EligibilityBlocker::UnresolvedSecretFinding,
                    ..
                } => SafetyState::OverrideRequired,
                _ => SafetyState::NotEvaluated,
            },
            scan_hash: Some(decision.revalidation().scan_hash.clone()),
            ..SafetyView::default()
        };
        view.provenance = Provenance::LocalDecision;
        view.next_action = default_next_action(&view.outcome);
        view
    }
}

fn failure_code(failure: &repo_com_discord_message::PreDispatchFailure) -> &'static str {
    match failure {
        repo_com_discord_message::PreDispatchFailure::ConnectFailed => "connect-failed",
        repo_com_discord_message::PreDispatchFailure::ConnectTimeout => "connect-timeout",
    }
}

fn retry_delay_kind_text(kind: RetryDelayKind) -> &'static str {
    match kind {
        RetryDelayKind::PreDispatchJitter => "pre-dispatch-jitter",
        RetryDelayKind::DiscordDirected => "discord-directed",
    }
}

fn default_next_action(outcome: &DeliveryOutcome) -> String {
    match outcome {
        DeliveryOutcome::Unclaimed => {
            "Run a current eligibility check; no delivery attempt is recorded.".to_owned()
        }
        DeliveryOutcome::Claimed => {
            "Perform exactly one transport attempt through the delivery owner.".to_owned()
        }
        DeliveryOutcome::Accepted { .. } => {
            "Keep the immutable local record; do not infer recipient attention or a response."
                .to_owned()
        }
        DeliveryOutcome::Failed { .. } => {
            "Inspect the redacted failure and create a new explicitly authorized attempt if safe."
                .to_owned()
        }
        DeliveryOutcome::RetryWait { .. } => {
            "Honor the bounded wait and let the delivery owner claim the next attempt.".to_owned()
        }
        DeliveryOutcome::Unknown { .. } | DeliveryOutcome::ReconciliationUnknown { .. } => {
            "Reconcile the exact destination, bot author, nonce, and content before any resend."
                .to_owned()
        }
        DeliveryOutcome::ReconciledAccepted { .. } => {
            "Keep the reconciled local record; do not infer recipient attention or a response."
                .to_owned()
        }
        DeliveryOutcome::ReconciledAbsent { .. } => {
            "Review the absence evidence and require a fresh domain decision before any resend."
                .to_owned()
        }
        DeliveryOutcome::Unresolved { .. } => {
            "Keep the send blocked and inspect the redacted reconciliation evidence.".to_owned()
        }
        DeliveryOutcome::EligibilityRejected { .. } => {
            "Resolve the exact approval, policy, safety, destination, or revision blocker."
                .to_owned()
        }
        DeliveryOutcome::Expired => {
            "Create a new immutable draft revision; an expired revision cannot be sent.".to_owned()
        }
        DeliveryOutcome::StaleAuthority { .. } => {
            "Rebuild current approval or policy authority before attempting delivery.".to_owned()
        }
        DeliveryOutcome::Error { .. } => {
            "Inspect the stable error category and redacted detail before retrying.".to_owned()
        }
    }
}

/// A redacted local error projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ErrorView {
    /// Optional repository scope.
    pub repository_id: Option<String>,
    /// Optional draft identity.
    pub draft_id: Option<String>,
    /// Optional revision.
    pub revision: Option<u64>,
    /// Optional resolved destination.
    pub destination: Option<DestinationView>,
    /// Stable foundation category code.
    pub category: String,
    /// Redacted detail.
    pub detail: String,
    /// Explicit operator next action.
    pub next_action: String,
    /// Provenance label.
    pub provenance: Provenance,
}

impl ErrorView {
    /// Creates a local error projection.
    #[must_use]
    pub fn new(
        category: impl Into<String>,
        detail: impl Into<String>,
        next_action: impl Into<String>,
    ) -> Self {
        Self {
            repository_id: None,
            draft_id: None,
            revision: None,
            destination: None,
            category: category.into(),
            detail: detail.into(),
            next_action: next_action.into(),
            provenance: Provenance::LocalError,
        }
    }

    /// Resolves the stable foundation category, failing closed to an internal
    /// error when a caller supplies an unknown category string.
    #[must_use]
    pub fn category_code(&self) -> ErrorCategory {
        ErrorCategory::from_code(&self.category).unwrap_or(ErrorCategory::InternalFailure)
    }

    /// Returns the stable process exit code for this error.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        self.category_code().exit_code()
    }

    /// Converts the safe projection into a foundation protocol error.
    #[must_use]
    pub fn to_repo_com_error(&self) -> repo_com_foundation::RepoComError {
        repo_com_foundation::RepoComError::new(
            self.category_code(),
            format!("{}; next action: {}", self.detail, self.next_action),
        )
    }
}

impl From<repo_com_foundation::RepoComError> for ErrorView {
    fn from(error: repo_com_foundation::RepoComError) -> Self {
        Self::new(
            error.code.code(),
            error.message,
            "Inspect the stable error category and redacted detail before retrying.",
        )
    }
}
