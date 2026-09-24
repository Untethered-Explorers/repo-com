//! Pure, deterministic operations terminal rendering.
//!
//! This module owns presentation only.  It copies safe projections from
//! configuration, policy, lifecycle, audit, inbound, retention, and purge
//! results; it never opens a store, performs a mutation, calls Discord, or
//! decides whether an operator action is authorized.  Every view is linear,
//! labeled, and complete at the effective width.

use std::env;

use repo_com_audit_query::{AuditPage, AuditQueryError};
use repo_com_config::{ConfigError, ResolvedConfig};
use repo_com_foundation::{ColorChoice, CommandOutcome, ErrorCategory, RepoComError, TtyMode};
use repo_com_inbox_fetch::{FetchError, UntrustedInboundEnvelope};
use repo_com_inbox_state::PageCommitResult;
use repo_com_lifecycle::StateVerificationReport;
use repo_com_lifecycle::inspect::{
    AcknowledgementProjection, ArchiveProjection, AuditTransitionsProjection,
    DeliveryAttemptProjection, DraftProjection, DraftRevisionProjection, InboundItemProjection,
    LifecycleProjection, ReplyClaimStatus as DomainReplyClaimStatus, ReplyLinkProjection,
    RepositoryProjection,
};
use repo_com_policy::{ActivationPreview, ActivationSnapshot, PolicyStatus};
use repo_com_purge::PurgeError;
use repo_com_retention::{RetentionError, RetentionPolicy, SweepCounts, SweepResult};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::purge::{PurgeExecutionState, PurgeExecutionView, PurgePlanView};
use crate::width::{
    MIN_TERMINAL_WIDTH, TerminalWidth, block_lines, display_width, field_lines, strip_ansi,
};

pub use repo_com_policy::PolicyTuple as ExactPolicyTuple;

/// Presentation options for human operations output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RenderOptions {
    /// Explicit color request.
    pub color: ColorChoice,
    /// Effective output width, never below 80 columns.
    pub width: TerminalWidth,
    /// Explicit stream mode supplied by the command layer.
    pub tty_mode: TtyMode,
}

impl Default for RenderOptions {
    fn default() -> Self {
        Self::plain_text()
    }
}

impl RenderOptions {
    /// Creates the safe plain-text 80-column default.
    #[must_use]
    pub const fn plain_text() -> Self {
        Self {
            color: ColorChoice::Never,
            width: TerminalWidth::eighty(),
            tty_mode: TtyMode::NonTty,
        }
    }

    /// Alias for [`Self::plain_text`].
    #[must_use]
    pub const fn no_color() -> Self {
        Self::plain_text()
    }

    /// Creates plain-text options with a caller-selected minimum width.
    #[must_use]
    pub const fn with_width(width: usize) -> Self {
        Self {
            color: ColorChoice::Never,
            width: TerminalWidth::new(width),
            tty_mode: TtyMode::NonTty,
        }
    }

    /// Creates options with an explicit color request and stream mode.
    #[must_use]
    pub const fn with_color(color: ColorChoice, tty_mode: TtyMode) -> Self {
        Self {
            color,
            width: TerminalWidth::eighty(),
            tty_mode,
        }
    }

    /// Resolves color without consulting ambient state.
    #[must_use]
    pub const fn resolve_color(
        color: ColorChoice,
        tty_mode: TtyMode,
        no_color_present: bool,
    ) -> ColorChoice {
        if no_color_present || tty_mode.is_non_tty() {
            ColorChoice::Never
        } else {
            color
        }
    }

    /// Resolves an explicit color request while honoring `NO_COLOR`.
    #[must_use]
    pub fn from_environment(color: ColorChoice, tty_mode: TtyMode) -> Self {
        let color = Self::resolve_color(color, tty_mode, env::var_os("NO_COLOR").is_some());
        Self {
            color,
            width: TerminalWidth::eighty(),
            tty_mode,
        }
    }

    /// Returns a copy with a different effective width.
    #[must_use]
    pub const fn width(self, width: usize) -> Self {
        Self {
            width: TerminalWidth::new(width),
            ..self
        }
    }

    /// Returns whether ANSI styling is permitted.
    #[must_use]
    pub const fn ansi_enabled(self) -> bool {
        matches!(self.color, ColorChoice::Always) && self.tty_mode.is_tty()
    }

    /// Returns the effective width.
    #[must_use]
    pub const fn columns(self) -> usize {
        self.width.columns()
    }
}

/// Describes where a displayed fact came from without implying current remote
/// truth.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Provenance {
    /// A local state or local decision value.
    LocalState,
    /// A value recorded by a prior fetch, not current remote truth.
    LastFetchedRemote,
    /// A local validation or inspection decision.
    LocalDecision,
    /// A local error or blocked operation.
    LocalError,
    /// A controlled remote observation supplied by a caller or test adapter.
    MockedRemoteObservation,
}

impl Provenance {
    /// Returns the stable text label used in human output.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalState => "local state",
            Self::LastFetchedRemote => "last-fetched remote state",
            Self::LocalDecision => "local decision",
            Self::LocalError => "local error",
            Self::MockedRemoteObservation => "controlled remote observation",
        }
    }
}

impl std::fmt::Display for Provenance {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A stable text field used to preserve projection details in reading order.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DetailField {
    /// Complete field label.
    pub label: String,
    /// Complete field value.
    pub value: String,
}

impl DetailField {
    /// Creates a detail field.
    #[must_use]
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

/// Configuration validation status.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigStatus {
    /// The configuration resolved and validated.
    Valid,
    /// No configuration was found.
    Missing,
    /// The schema version is unsupported.
    UnsupportedSchema,
    /// A safe validation or resolution error occurred.
    Invalid,
    /// A secret-like field was rejected.
    SecretField,
}

impl ConfigStatus {
    /// Returns the stable text form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Missing => "missing",
            Self::UnsupportedSchema => "unsupported-schema",
            Self::Invalid => "invalid",
            Self::SecretField => "secret-field-rejected",
        }
    }
}

impl std::fmt::Display for ConfigStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Safe configuration status projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigStatusView {
    /// Repository identity, when safely available.
    pub repository_id: Option<String>,
    /// Normalized configuration path, when available.
    pub config_path: Option<String>,
    /// Schema version, when available.
    pub schema_version: Option<u32>,
    /// Workspace identity, when available.
    pub workspace_id: Option<String>,
    /// Canonical configuration hash, when available.
    pub config_hash: Option<String>,
    /// Resolved outbound aliases in deterministic order.
    pub destination_aliases: Vec<String>,
    /// Resolved inbound aliases in deterministic order.
    pub inbound_aliases: Vec<String>,
    /// Number of exact auto-send entries.
    pub auto_send_entries: usize,
    /// Validation status.
    pub status: ConfigStatus,
    /// Stable validation code.
    pub validation_code: Option<String>,
    /// Safe redacted validation detail.
    pub detail: Option<String>,
    /// Provenance.
    pub provenance: Provenance,
    /// Explicit next action.
    pub next_action: String,
}

impl ConfigStatusView {
    /// Projects a resolved configuration without reading it again.
    #[must_use]
    pub fn from_resolved(config: &ResolvedConfig) -> Self {
        Self {
            repository_id: Some(config.config.repository_id.clone()),
            config_path: Some(config.path.to_string_lossy().into_owned()),
            schema_version: Some(config.config.schema_version),
            workspace_id: Some(config.config.discord.workspace_id.clone()),
            config_hash: Some(config.canonical_hash()),
            destination_aliases: config.destinations.keys().cloned().collect(),
            inbound_aliases: config.inbound.keys().cloned().collect(),
            auto_send_entries: config.config.auto_send.len(),
            status: ConfigStatus::Valid,
            validation_code: Some("config-valid".to_owned()),
            detail: Some("configuration resolved and validated".to_owned()),
            provenance: Provenance::LocalDecision,
            next_action:
                "Use the exact repository and alias identifiers for the requested operation."
                    .to_owned(),
        }
    }

    /// Projects a safe configuration error without retaining source content.
    #[must_use]
    pub fn from_error(error: &ConfigError) -> Self {
        let status = match error.code() {
            "config-not-found" | "multiple-config-candidates" => ConfigStatus::Missing,
            "unsupported-schema-version" => ConfigStatus::UnsupportedSchema,
            "secret-field" => ConfigStatus::SecretField,
            _ => ConfigStatus::Invalid,
        };
        Self {
            repository_id: None,
            config_path: error.path().map(|path| path.to_string_lossy().into_owned()),
            schema_version: None,
            workspace_id: None,
            config_hash: None,
            destination_aliases: Vec::new(),
            inbound_aliases: Vec::new(),
            auto_send_entries: 0,
            status,
            validation_code: Some(error.code().to_owned()),
            detail: Some(error.message().to_owned()),
            provenance: Provenance::LocalError,
            next_action:
                "Correct the reported configuration class and retry; no secret value is displayed."
                    .to_owned(),
        }
    }
}

/// Exact policy status labels used by operations presentation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolicyState {
    /// No exact policy is configured.
    NotConfigured,
    /// The exact policy is configured but has no activation.
    NotActivated,
    /// One current exact activation exists.
    Active,
    /// An activation exists but its hash binding is stale.
    Stale,
    /// The matching activation was deactivated.
    Deactivated,
    /// Multiple matching activations make authority ambiguous.
    Ambiguous,
    /// The status was not evaluated.
    NotEvaluated,
}

impl PolicyState {
    /// Returns the stable text form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotConfigured => "not-configured",
            Self::NotActivated => "not-activated",
            Self::Active => "active",
            Self::Stale => "stale",
            Self::Deactivated => "deactivated",
            Self::Ambiguous => "ambiguous",
            Self::NotEvaluated => "not-evaluated",
        }
    }
}

impl std::fmt::Display for PolicyState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One activation snapshot shown in policy status.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PolicyActivationSnapshotView {
    /// Stable activation ID.
    pub activation_id: String,
    /// Exact repository scope.
    pub repository_id: String,
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
    /// Deactivation timestamp, if present.
    pub deactivated_at: Option<String>,
    /// Whether the local row is active.
    pub active: bool,
    /// Safe stale reason, if present.
    pub stale_reason: Option<String>,
}

impl From<&ActivationSnapshot> for PolicyActivationSnapshotView {
    fn from(snapshot: &ActivationSnapshot) -> Self {
        Self {
            activation_id: snapshot.activation_id.clone(),
            repository_id: snapshot.repository_id.clone(),
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

/// Safe policy status projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PolicyStatusView {
    /// Exact repository scope.
    pub repository_id: String,
    /// Exact policy tuple, when one is known.
    pub tuple: Option<PolicyTuple>,
    /// Current policy state.
    pub state: PolicyState,
    /// Current configuration hash.
    pub config_hash: Option<String>,
    /// Current tuple hash.
    pub tuple_hash: Option<String>,
    /// Selected activation ID.
    pub activation_id: Option<String>,
    /// All matching activation snapshots in deterministic order.
    pub activations: Vec<PolicyActivationSnapshotView>,
    /// Human-readable safe policy basis.
    pub basis: String,
    /// Provenance.
    pub provenance: Provenance,
    /// Explicit next action.
    pub next_action: String,
}

impl PolicyStatusView {
    /// Projects a read-only policy status.
    #[must_use]
    pub fn from_status(repository_id: impl Into<String>, status: &PolicyStatus) -> Self {
        let repository_id = repository_id.into();
        let (state, tuple, config_hash, tuple_hash, activation_id, activations, basis) =
            match status {
                PolicyStatus::NotConfigured => (
                    PolicyState::NotConfigured,
                    None,
                    None,
                    None,
                    None,
                    Vec::new(),
                    "no exact configured policy".to_owned(),
                ),
                PolicyStatus::NotActivated => (
                    PolicyState::NotActivated,
                    None,
                    None,
                    None,
                    None,
                    Vec::new(),
                    "configured policy has no activation".to_owned(),
                ),
                PolicyStatus::Active(snapshot) => (
                    PolicyState::Active,
                    Some(snapshot.tuple.clone()),
                    Some(snapshot.current_config_hash.clone()),
                    Some(snapshot.current_tuple_hash.clone()),
                    Some(snapshot.activation_id.clone()),
                    vec![snapshot.into()],
                    "one current exact activation".to_owned(),
                ),
                PolicyStatus::Stale(snapshot) => (
                    PolicyState::Stale,
                    Some(snapshot.tuple.clone()),
                    Some(snapshot.current_config_hash.clone()),
                    Some(snapshot.current_tuple_hash.clone()),
                    Some(snapshot.activation_id.clone()),
                    vec![snapshot.into()],
                    "activation hash binding is stale".to_owned(),
                ),
                PolicyStatus::Deactivated(snapshot) => (
                    PolicyState::Deactivated,
                    Some(snapshot.tuple.clone()),
                    Some(snapshot.current_config_hash.clone()),
                    Some(snapshot.current_tuple_hash.clone()),
                    Some(snapshot.activation_id.clone()),
                    vec![snapshot.into()],
                    "activation was explicitly deactivated".to_owned(),
                ),
                PolicyStatus::Ambiguous { activations } => {
                    let first = activations.first();
                    (
                        PolicyState::Ambiguous,
                        first.map(|snapshot| snapshot.tuple.clone()),
                        first.map(|snapshot| snapshot.current_config_hash.clone()),
                        first.map(|snapshot| snapshot.current_tuple_hash.clone()),
                        first.map(|snapshot| snapshot.activation_id.clone()),
                        activations
                            .iter()
                            .map(PolicyActivationSnapshotView::from)
                            .collect(),
                        format!("{} matching activations", activations.len()),
                    )
                }
            };
        let next_action = match state {
            PolicyState::Active => "Keep this exact tuple and hash binding in the current decision; do not widen it.",
            PolicyState::Stale => "Reactivate only the current exact configuration and tuple through the TTY boundary.",
            PolicyState::NotActivated => "Request an explicit TTY activation for the exact tuple if policy use is intended.",
            PolicyState::NotConfigured => "Configure an exact policy tuple before requesting activation.",
            PolicyState::Deactivated => "Do not treat this tuple as active; create a new explicit activation if needed.",
            PolicyState::Ambiguous => "Resolve ambiguous local activation records before granting policy authority.",
            PolicyState::NotEvaluated => "Run the read-only policy status inspection.",
        }
        .to_owned();
        Self {
            repository_id,
            tuple,
            state,
            config_hash,
            tuple_hash,
            activation_id,
            activations,
            basis,
            provenance: Provenance::LocalState,
            next_action,
        }
    }
}

/// Exact policy activation preview used before a keyboard confirmation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActivationPreviewView {
    /// Repository scope.
    pub repository_id: String,
    /// Exact tuple.
    pub tuple: PolicyTuple,
    /// Current canonical configuration hash.
    pub config_hash: String,
    /// Current exact tuple hash.
    pub tuple_hash: String,
    /// Proposed activation ID.
    pub activation_id: String,
    /// Provenance.
    pub provenance: Provenance,
    /// Explicit next action.
    pub next_action: String,
}

impl From<&ActivationPreview> for ActivationPreviewView {
    fn from(preview: &ActivationPreview) -> Self {
        Self {
            repository_id: preview.repository_id.clone(),
            tuple: preview.tuple.clone(),
            config_hash: preview.config_hash.clone(),
            tuple_hash: preview.tuple_hash.clone(),
            activation_id: preview.activation_id.clone(),
            provenance: Provenance::LocalDecision,
            next_action: "Confirm the exact repository, tuple, and hashes on a TTY; activation is not yet recorded."
                .to_owned(),
        }
    }
}

/// Stable presentation status for one local verification check.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CheckState {
    /// The check ran and passed.
    Passed,
    /// The check ran and found a failure.
    Failed,
    /// The check could not run safely.
    Unavailable,
}

impl CheckState {
    /// Returns the stable text form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Unavailable => "unavailable",
        }
    }
}

impl std::fmt::Display for CheckState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One local state verification check.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StateCheckView {
    /// Check status.
    pub status: CheckState,
    /// Convenience pass flag.
    pub passed: bool,
    /// Stable check code.
    pub code: String,
    /// Complete safe check details.
    pub details: Vec<DetailField>,
}

impl StateCheckView {
    fn new(status: CheckState, passed: bool, code: &str, details: Vec<DetailField>) -> Self {
        Self {
            status,
            passed,
            code: code.to_owned(),
            details,
        }
    }
}

/// Complete read-only local state verification projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct StateVerificationView {
    /// Repository scope.
    pub repository_id: String,
    /// Whether all required checks passed.
    pub healthy: bool,
    /// Whether the verifier was read-only.
    pub read_only: bool,
    /// Whether the connection was query-only.
    pub connection_read_only: bool,
    /// Required v1 storage privacy disclosure.
    pub privacy_disclosure: String,
    /// Quick-check result.
    pub quick_check: StateCheckView,
    /// Foreign-key result.
    pub foreign_keys: StateCheckView,
    /// Migration result.
    pub migration: StateCheckView,
    /// Repository-scope result.
    pub repository_scope: StateCheckView,
    /// Filesystem-permission result.
    pub filesystem_permissions: StateCheckView,
    /// Safe issue codes and summaries.
    pub issues: Vec<DetailField>,
    /// Safe remediation steps.
    pub remediation: Vec<String>,
    /// Provenance.
    pub provenance: Provenance,
    /// Explicit next action.
    pub next_action: String,
}

impl StateVerificationView {
    /// Projects a read-only verification report without reopening state.
    #[must_use]
    pub fn from_report(report: &StateVerificationReport) -> Self {
        let quick_check = StateCheckView::new(
            map_check_status(report.quick_check.status),
            report.quick_check.passed,
            report.quick_check.code,
            Vec::new(),
        );
        let foreign_keys = StateCheckView::new(
            map_check_status(report.foreign_keys.status),
            report.foreign_keys.passed,
            report.foreign_keys.code,
            vec![
                DetailField::new("Enabled", report.foreign_keys.enabled.to_string()),
                DetailField::new(
                    "Violation count",
                    report.foreign_keys.violation_count.to_string(),
                ),
                DetailField::new("Bounded", report.foreign_keys.bounded.to_string()),
            ],
        );
        let migration = StateCheckView::new(
            map_check_status(report.migration.status),
            report.migration.passed,
            report.migration.code,
            vec![
                DetailField::new("Expected", report.migration.expected.to_string()),
                DetailField::new(
                    "Found",
                    report
                        .migration
                        .found
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "(unavailable)".to_owned()),
                ),
            ],
        );
        let repository_scope = StateCheckView::new(
            map_check_status(report.repository_scope.status),
            report.repository_scope.passed,
            report.repository_scope.code,
            vec![DetailField::new(
                "Repository found",
                report.repository_scope.repository_found.to_string(),
            )],
        );
        let filesystem_permissions = StateCheckView::new(
            map_check_status(report.filesystem_permissions.status),
            report.filesystem_permissions.passed,
            report.filesystem_permissions.code,
            vec![
                DetailField::new("Permission model", &report.filesystem_permissions.model),
                DetailField::new(
                    "File present",
                    report.filesystem_permissions.file_present.to_string(),
                ),
                DetailField::new(
                    "File user-only",
                    report.filesystem_permissions.file_user_only.to_string(),
                ),
                DetailField::new(
                    "Parent user-only",
                    report.filesystem_permissions.parent_user_only.to_string(),
                ),
            ],
        );
        let issues = report
            .issues
            .iter()
            .map(|issue| DetailField::new(issue.code, issue.summary))
            .collect();
        let next_action = if report.healthy {
            "Use the verified local state for read-only inspection; repair any later issue before mutation."
        } else {
            "Review every listed issue and remediation; do not mutate local state until integrity checks pass."
        };
        Self {
            repository_id: report.repository_id.clone(),
            healthy: report.healthy,
            read_only: report.read_only,
            connection_read_only: report.connection_read_only,
            privacy_disclosure: report.privacy_disclosure.clone(),
            quick_check,
            foreign_keys,
            migration,
            repository_scope,
            filesystem_permissions,
            issues,
            remediation: report.remediation.clone(),
            provenance: Provenance::LocalState,
            next_action: next_action.to_owned(),
        }
    }
}

/// One redacted local audit event.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuditEventView {
    /// Stable local sequence.
    pub audit_id: i64,
    /// Stable event ID.
    pub event_id: String,
    /// Object family.
    pub object_type: String,
    /// Object ID.
    pub object_id: String,
    /// Transition.
    pub transition: String,
    /// Occurrence timestamp.
    pub occurred_at: String,
    /// Actor kind.
    pub actor_kind: String,
    /// Outcome.
    pub outcome: String,
    /// Defensively redacted metadata.
    pub metadata: Value,
}

impl AuditEventView {
    fn from_event(event: &repo_com_audit_query::AuditQueryEvent) -> Self {
        Self {
            audit_id: event.audit_id,
            event_id: event.event_id.clone(),
            object_type: event.object_type.clone(),
            object_id: event.object_id.clone(),
            transition: event.transition.clone(),
            occurred_at: event.occurred_at.clone(),
            actor_kind: event.actor_kind.clone(),
            outcome: event.outcome.clone(),
            metadata: event.metadata.clone(),
        }
    }
}

/// Bounded local audit page projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuditView {
    /// Repository scope.
    pub repository_id: String,
    /// Requested page bound.
    pub page_size: usize,
    /// Returned event count.
    pub event_count: usize,
    /// Whether more local events exist.
    pub has_more: bool,
    /// Opaque continuation, if any.
    pub next_cursor: Option<String>,
    /// Redacted events in stable order.
    pub events: Vec<AuditEventView>,
    /// Audit evidence is local.
    pub provenance: Provenance,
    /// Explicitly false: this view performs no remote fetch.
    pub remote_fetch_performed: bool,
    /// Explicit next action.
    pub next_action: String,
}

impl AuditView {
    /// Projects a bounded local audit page.
    #[must_use]
    pub fn from_page(page: &AuditPage) -> Self {
        Self {
            repository_id: page.repository_id.clone(),
            page_size: page.page_size,
            event_count: page.events.len(),
            has_more: page.truncated,
            next_cursor: page.next_cursor.as_ref().map(|cursor| {
                format!(
                    "{}|{}|{}",
                    cursor.repository_id, cursor.occurred_at, cursor.audit_id
                )
            }),
            events: page.events.iter().map(AuditEventView::from_event).collect(),
            provenance: Provenance::LocalState,
            remote_fetch_performed: false,
            next_action: if page.truncated {
                "Use the returned continuation to request the next bounded local audit page."
            } else {
                "This bounded local audit page is complete; no remote fetch was performed."
            }
            .to_owned(),
        }
    }
}

/// Remote value explicitly recorded by a prior local fetch or point check.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RemoteStateView {
    /// Observation timestamp.
    pub observed_at: String,
    /// Recorded remote state.
    pub state: String,
    /// Always false for a local last-fetched projection.
    pub current_remote_truth: bool,
    /// Provenance.
    pub provenance: Provenance,
}

impl RemoteStateView {
    fn last_fetched(observed_at: impl Into<String>, state: impl Into<String>) -> Self {
        Self {
            observed_at: observed_at.into(),
            state: state.into(),
            current_remote_truth: false,
            provenance: Provenance::LastFetchedRemote,
        }
    }
}

/// Read-receipt status deliberately unavailable in this product.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReadReceiptStatus {
    /// The product never collects or infers read receipts.
    Unavailable,
}

impl ReadReceiptStatus {
    /// Returns stable text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        "unavailable; never inferred"
    }
}

impl std::fmt::Display for ReadReceiptStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Conservative local reply evidence status.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReplyClaimStatus {
    /// No accepted reply evidence exists.
    NotClaimed,
    /// Durable accepted-delivery evidence exists locally.
    AcceptedEvidence,
    /// Evidence could not be verified.
    Unverified,
}

impl ReplyClaimStatus {
    /// Returns stable text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotClaimed => "not-claimed",
            Self::AcceptedEvidence => "accepted-evidence",
            Self::Unverified => "unverified",
        }
    }
}

impl std::fmt::Display for ReplyClaimStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One bounded lifecycle record with local and remote evidence separated.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LifecycleView {
    /// Repository scope.
    pub repository_id: String,
    /// Object family.
    pub object_type: String,
    /// Exact object ID.
    pub object_id: String,
    /// Optional immutable revision.
    pub revision: Option<i64>,
    /// Current local state.
    pub state: String,
    /// Explicit local-state summary.
    pub local_state: String,
    /// Last-fetched remote evidence, if the projection has one.
    pub last_fetched_remote: Option<RemoteStateView>,
    /// Whether this is untrusted inbound data.
    pub untrusted: bool,
    /// Read-receipt status.
    pub read_receipt: ReadReceiptStatus,
    /// Reply-claim status.
    pub reply_claim: ReplyClaimStatus,
    /// Whether a remote fetch was performed.
    pub remote_fetch_performed: bool,
    /// Complete safe detail fields in stable order.
    pub details: Vec<DetailField>,
    /// Provenance.
    pub provenance: Provenance,
    /// Explicit next action.
    pub next_action: String,
}

impl LifecycleView {
    /// Projects one typed lifecycle projection without performing I/O.
    #[must_use]
    pub fn from_projection(projection: &LifecycleProjection) -> Self {
        match projection {
            LifecycleProjection::Repository(value) => Self::from_repository(value),
            LifecycleProjection::Draft(value) => Self::from_draft(value),
            LifecycleProjection::DraftRevision(value) => Self::from_draft_revision(value),
            LifecycleProjection::DeliveryAttempt(value) => Self::from_delivery_attempt(value),
            LifecycleProjection::InboundItem(value) => Self::from_inbound(value),
            LifecycleProjection::Acknowledgement(value) => Self::from_acknowledgement(value),
            LifecycleProjection::Archive(value) => Self::from_archive(value),
            LifecycleProjection::ReplyLink(value) => Self::from_reply_link(value),
            LifecycleProjection::AuditTransitions(value) => Self::from_audit_transitions(value),
        }
    }

    fn base(
        repository_id: impl Into<String>,
        object_type: &str,
        object_id: impl Into<String>,
    ) -> Self {
        Self {
            repository_id: repository_id.into(),
            object_type: object_type.to_owned(),
            object_id: object_id.into(),
            revision: None,
            state: "unknown".to_owned(),
            local_state: "local state recorded".to_owned(),
            last_fetched_remote: None,
            untrusted: false,
            read_receipt: ReadReceiptStatus::Unavailable,
            reply_claim: ReplyClaimStatus::NotClaimed,
            remote_fetch_performed: false,
            details: Vec::new(),
            provenance: Provenance::LocalState,
            next_action: "Inspect the local record; no remote mutation is implied.".to_owned(),
        }
    }

    fn from_repository(value: &RepositoryProjection) -> Self {
        let mut view = Self::base(
            value.repository_id.clone(),
            "repository",
            value.repository_id.clone(),
        );
        view.state = "configured".to_owned();
        view.details = vec![
            DetailField::new("Workspace", &value.workspace_id),
            DetailField::new("Configuration hash", &value.config_hash),
            DetailField::new("Created at", &value.created_at),
            DetailField::new("Updated at", &value.updated_at),
        ];
        view.next_action =
            "Use the exact repository scope for local operations; no remote state was inspected."
                .to_owned();
        view
    }

    fn from_draft(value: &DraftProjection) -> Self {
        let mut view = Self::base(value.repository_id.clone(), "draft", value.draft_id.clone());
        view.revision = Some(value.current_revision);
        view.state = value.status.clone();
        view.local_state = format!(
            "current revision {} (destination alias {})",
            value.current_revision, value.destination_alias
        );
        view.details = vec![
            DetailField::new("Event type", &value.event_type),
            DetailField::new("Destination alias", &value.destination_alias),
            DetailField::new("Expiry", value.expiry_at.as_deref().unwrap_or("(none)")),
            DetailField::new(
                "Reply target",
                value
                    .reply_to_inbound_item_id
                    .as_deref()
                    .unwrap_or("(none)"),
            ),
            DetailField::new("Created at", &value.created_at),
            DetailField::new("Updated at", &value.updated_at),
        ];
        view.next_action = "Inspect the local lifecycle record; this operations view does not render outbound text."
            .to_owned();
        view
    }

    fn from_draft_revision(value: &DraftRevisionProjection) -> Self {
        let mut view = Self::base(
            value.repository_id.clone(),
            "draft_revision",
            format!("{}~{}", value.draft_id, value.revision),
        );
        view.revision = Some(value.revision);
        view.state = value.lifecycle_state.clone();
        view.local_state = format!("immutable local revision {}", value.revision);
        view.details = vec![
            DetailField::new("Draft ID", &value.draft_id),
            DetailField::new("Revision hash", &value.content_hash),
            DetailField::new("Destination alias", &value.destination_alias),
            DetailField::new("Resolved destination", &value.resolved_destination),
            DetailField::new("Expiry", value.expiry_at.as_deref().unwrap_or("(none)")),
            DetailField::new(
                "Reply target",
                value
                    .reply_to_inbound_item_id
                    .as_deref()
                    .unwrap_or("(none)"),
            ),
            DetailField::new(
                "Remote fetch performed",
                value.remote_fetch_performed.to_string(),
            ),
        ];
        view.next_action = "Use the local revision evidence for lifecycle inspection; no outbound content, send, or approval is rendered."
            .to_owned();
        view
    }

    fn from_delivery_attempt(value: &DeliveryAttemptProjection) -> Self {
        let mut view = Self::base(
            value.repository_id.clone(),
            "delivery_attempt",
            value.attempt_id.clone(),
        );
        view.revision = Some(value.revision);
        view.state = value.state.clone();
        view.local_state = format!("local delivery state {}", value.state);
        view.read_receipt = ReadReceiptStatus::Unavailable;
        view.reply_claim = ReplyClaimStatus::NotClaimed;
        view.details = vec![
            DetailField::new("Draft ID", &value.draft_id),
            DetailField::new("Attempt number", value.attempt_number.to_string()),
            DetailField::new("Claim nonce", &value.claim_nonce),
            DetailField::new("Claimed at", &value.claimed_at),
            DetailField::new(
                "Completed at",
                value.completed_at.as_deref().unwrap_or("(none)"),
            ),
            DetailField::new(
                "Remote message ID",
                value.remote_message_id.as_deref().unwrap_or("(none)"),
            ),
            DetailField::new(
                "Failure code",
                value.failure_code.as_deref().unwrap_or("(none)"),
            ),
        ];
        if let Some(remote) = &value.last_recorded_remote {
            view.last_fetched_remote = Some(RemoteStateView::last_fetched(
                remote.observed_at.as_deref().unwrap_or("(not recorded)"),
                "remote message ID recorded",
            ));
            view.details.push(DetailField::new(
                "Last-fetched remote message ID",
                &remote.message_id,
            ));
        }
        view.next_action = "Use local delivery evidence only; accepted delivery is not a read receipt or reply claim."
            .to_owned();
        view
    }

    fn from_inbound(value: &InboundItemProjection) -> Self {
        let mut view = Self::base(
            value.repository_id.clone(),
            "inbound_item",
            value.item_id.clone(),
        );
        view.state = if value.current_snapshot.deleted {
            "deleted"
        } else {
            "stored"
        }
        .to_owned();
        view.untrusted = true;
        view.local_state = format!(
            "acknowledged={}, archived={}, replied={}",
            value.local_state.acknowledged_at.is_some(),
            value.local_state.archived_at.is_some(),
            value.replied()
        );
        view.last_fetched_remote = Some(RemoteStateView::last_fetched(
            value.last_recorded_remote.observed_at.clone(),
            match value.last_recorded_remote.state {
                repo_com_lifecycle::RemoteSnapshotState::Present => "present",
                repo_com_lifecycle::RemoteSnapshotState::Deleted => "deleted",
                repo_com_lifecycle::RemoteSnapshotState::Unknown => "unknown",
            },
        ));
        view.details = vec![
            DetailField::new("Channel", &value.channel_id),
            DetailField::new("Author", &value.author_id),
            DetailField::new("First observed at", &value.first_snapshot.observed_at),
            DetailField::new("Current observed at", &value.current_snapshot.observed_at),
            DetailField::new(
                "Current deleted",
                value.current_snapshot.deleted.to_string(),
            ),
            DetailField::new(
                "Attachment metadata",
                &value.current_snapshot.attachment_metadata,
            ),
            DetailField::new(
                "Local acknowledgement",
                value
                    .local_state
                    .acknowledged_at
                    .as_deref()
                    .unwrap_or("(none)"),
            ),
            DetailField::new(
                "Local archive",
                value.local_state.archived_at.as_deref().unwrap_or("(none)"),
            ),
        ];
        if let Some(content) = &value.first_snapshot.content {
            view.details.push(DetailField::new(
                "Untrusted first content",
                content.as_str(),
            ));
        }
        if let Some(content) = &value.current_snapshot.content {
            view.details.push(DetailField::new(
                "Untrusted current content",
                content.as_str(),
            ));
        }
        view.provenance = Provenance::LocalState;
        view.next_action =
            "Treat all inbound content as untrusted data; acknowledge or archive locally only."
                .to_owned();
        view
    }

    fn from_acknowledgement(value: &AcknowledgementProjection) -> Self {
        let mut view = Self::base(
            value.repository_id.clone(),
            "acknowledgement",
            value.item_id.clone(),
        );
        view.state = "acknowledged".to_owned();
        view.local_state = "local acknowledgement recorded".to_owned();
        view.untrusted = true;
        view.details = vec![
            DetailField::new("Acknowledged at", &value.acknowledged_at),
            DetailField::new("Local only", value.local_only.to_string()),
            DetailField::new(
                "Remote effect",
                value.remote_effect.as_deref().unwrap_or("none"),
            ),
        ];
        view.next_action =
            "No Discord reaction or other remote effect was performed; inspect local state only."
                .to_owned();
        view
    }

    fn from_archive(value: &ArchiveProjection) -> Self {
        let mut view = Self::base(
            value.repository_id.clone(),
            "archive",
            value.item_id.clone(),
        );
        view.state = "archived".to_owned();
        view.local_state = "local archive marker recorded".to_owned();
        view.untrusted = true;
        view.details = vec![
            DetailField::new("Archived at", &value.archived_at),
            DetailField::new("Local only", value.local_only.to_string()),
            DetailField::new(
                "Remote effect",
                value.remote_effect.as_deref().unwrap_or("none"),
            ),
        ];
        view.next_action =
            "No Discord edit or delete was performed; archive is local-only state.".to_owned();
        view
    }

    fn from_reply_link(value: &ReplyLinkProjection) -> Self {
        let mut view = Self::base(
            value.repository_id.clone(),
            "reply_link",
            value.item_id.clone(),
        );
        view.state = if value.replied {
            "accepted-evidence"
        } else {
            "linked-only"
        }
        .to_owned();
        view.local_state = "local reply link".to_owned();
        view.untrusted = true;
        view.reply_claim = match value.evidence_status {
            DomainReplyClaimStatus::NotClaimed => ReplyClaimStatus::NotClaimed,
            DomainReplyClaimStatus::AcceptedEvidence => ReplyClaimStatus::AcceptedEvidence,
            DomainReplyClaimStatus::Unverified => ReplyClaimStatus::Unverified,
        };
        view.read_receipt = ReadReceiptStatus::Unavailable;
        view.details = vec![
            DetailField::new("Reply draft ID", &value.reply_draft_id),
            DetailField::new("Linked at", &value.linked_at),
            DetailField::new(
                "Accepted delivery ID",
                value.accepted_delivery_id.as_deref().unwrap_or("(none)"),
            ),
            DetailField::new(
                "Accepted remote message ID",
                value
                    .accepted_remote_message_id
                    .as_deref()
                    .unwrap_or("(none)"),
            ),
            DetailField::new("Local-only link", value.local_only_link.to_string()),
            DetailField::new(
                "Audit event ID",
                value.audit_event_id.as_deref().unwrap_or("(none)"),
            ),
        ];
        view.next_action = "A linked draft is not a read receipt; require separate accepted-delivery evidence before claiming a reply."
            .to_owned();
        view
    }

    fn from_audit_transitions(value: &AuditTransitionsProjection) -> Self {
        let mut view = Self::base(
            value.page.repository_id.clone(),
            "audit_transition",
            "bounded-page",
        );
        view.state = if value.page.truncated {
            "more-local-events"
        } else {
            "complete-local-page"
        }
        .to_owned();
        view.local_state = format!("{} redacted local events", value.page.events.len());
        view.details = vec![
            DetailField::new("Page size", value.page.page_size.to_string()),
            DetailField::new("Has more", value.page.truncated.to_string()),
            DetailField::new(
                "Remote fetch performed",
                value.remote_fetch_performed.to_string(),
            ),
        ];
        view.next_action = "Use the bounded continuation for more local audit evidence; no remote fetch was performed."
            .to_owned();
        view
    }
}

impl From<&LifecycleProjection> for LifecycleView {
    fn from(projection: &LifecycleProjection) -> Self {
        Self::from_projection(projection)
    }
}

/// A bounded page of lifecycle projections.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LifecyclePageView {
    /// Repository scope.
    pub repository_id: String,
    /// Object family.
    pub object_type: String,
    /// Returned records in stable order.
    pub items: Vec<LifecycleView>,
    /// Requested page bound.
    pub page_size: usize,
    /// Whether more rows exist.
    pub truncated: bool,
    /// Opaque continuation.
    pub next_after: Option<String>,
    /// Provenance.
    pub provenance: Provenance,
    /// Whether remote fetch was performed.
    pub remote_fetch_performed: bool,
    /// Explicit next action.
    pub next_action: String,
}

impl LifecyclePageView {
    /// Creates a bounded page view from already-projected records.
    #[must_use]
    pub fn new(
        repository_id: impl Into<String>,
        object_type: impl Into<String>,
        items: Vec<LifecycleView>,
        page_size: usize,
        truncated: bool,
        next_after: Option<String>,
    ) -> Self {
        Self {
            repository_id: repository_id.into(),
            object_type: object_type.into(),
            items,
            page_size,
            truncated,
            next_after,
            provenance: Provenance::LocalState,
            remote_fetch_performed: false,
            next_action: if truncated {
                "Request the next bounded local page using the returned continuation."
            } else {
                "This bounded local lifecycle page is complete; no remote fetch was performed."
            }
            .to_owned(),
        }
    }
}

/// Result of one local inbound page commit.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InboundCommitView {
    /// Repository scope.
    pub repository_id: String,
    /// Configured inbound alias.
    pub alias: String,
    /// Authoritative local cursor after commit.
    pub cursor: String,
    /// Number of untrusted items stored locally.
    pub stored_items: usize,
    /// Number of reconciliation transitions stored locally.
    pub stored_transitions: usize,
    /// Local commit provenance.
    pub provenance: Provenance,
    /// Explicit next action.
    pub next_action: String,
}

impl InboundCommitView {
    /// Projects an atomic local commit result without retaining message content.
    #[must_use]
    pub fn from_commit(commit: &PageCommitResult) -> Self {
        Self {
            repository_id: commit.repository_id().to_owned(),
            alias: commit.alias().to_owned(),
            cursor: commit.cursor().to_owned(),
            stored_items: commit.stored_items,
            stored_transitions: commit.stored_transitions,
            provenance: Provenance::LocalState,
            next_action: "Inspect stored untrusted items locally; the commit did not grant permission or trigger a send."
                .to_owned(),
        }
    }
}

impl From<&PageCommitResult> for InboundCommitView {
    fn from(commit: &PageCommitResult) -> Self {
        Self::from_commit(commit)
    }
}

/// Dedicated inbound item view with an explicit trust boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InboundItemView {
    /// Repository scope.
    pub repository_id: String,
    /// Remote message/item ID.
    pub item_id: String,
    /// Configured channel ID.
    pub channel_id: String,
    /// Remote author ID.
    pub author_id: String,
    /// Exact untrusted message text, when explicitly retained.
    pub content: Option<String>,
    /// Whether the remote item was marked deleted.
    pub deleted: bool,
    /// First observation timestamp.
    pub first_observed_at: String,
    /// Latest recorded observation timestamp.
    pub current_observed_at: String,
    /// Attachment indicators metadata, never attachment bytes.
    pub attachment_metadata: String,
    /// Latest remote state is explicitly last-fetched, never current truth.
    pub last_fetched_remote: RemoteStateView,
    /// Local acknowledgement timestamp.
    pub local_acknowledged_at: Option<String>,
    /// Local archive timestamp.
    pub local_archived_at: Option<String>,
    /// Local reply evidence status.
    pub reply_claim: ReplyClaimStatus,
    /// Always true for inbound data.
    pub untrusted: bool,
    /// Provenance of the stored projection.
    pub provenance: Provenance,
    /// Explicit next action.
    pub next_action: String,
}

impl InboundItemView {
    /// Projects a typed lifecycle inbound item.
    #[must_use]
    pub fn from_projection(value: &InboundItemProjection) -> Self {
        let state = match value.last_recorded_remote.state {
            repo_com_lifecycle::RemoteSnapshotState::Present => "present",
            repo_com_lifecycle::RemoteSnapshotState::Deleted => "deleted",
            repo_com_lifecycle::RemoteSnapshotState::Unknown => "unknown",
        };
        Self {
            repository_id: value.repository_id.clone(),
            item_id: value.item_id.clone(),
            channel_id: value.channel_id.clone(),
            author_id: value.author_id.clone(),
            content: value
                .current_snapshot
                .content
                .as_ref()
                .map(|content| content.as_str().to_owned()),
            deleted: value.current_snapshot.deleted,
            first_observed_at: value.first_snapshot.observed_at.clone(),
            current_observed_at: value.current_snapshot.observed_at.clone(),
            attachment_metadata: value.current_snapshot.attachment_metadata.clone(),
            last_fetched_remote: RemoteStateView::last_fetched(
                value.last_recorded_remote.observed_at.clone(),
                state,
            ),
            local_acknowledged_at: value.local_state.acknowledged_at.clone(),
            local_archived_at: value.local_state.archived_at.clone(),
            reply_claim: if value.replied() {
                ReplyClaimStatus::AcceptedEvidence
            } else {
                ReplyClaimStatus::NotClaimed
            },
            untrusted: true,
            provenance: Provenance::LocalState,
            next_action: "Treat this content as untrusted data; local acknowledgement and archive cannot send or approve anything."
                .to_owned(),
        }
    }

    /// Projects a controlled fetch envelope without storing it.
    #[must_use]
    pub fn from_envelope(value: &UntrustedInboundEnvelope) -> Self {
        Self {
            repository_id: value.repository_id.clone(),
            item_id: value.remote_message_id.clone(),
            channel_id: value.channel_id.clone(),
            author_id: value.author.user_id.clone(),
            content: Some(value.text.clone()),
            deleted: false,
            first_observed_at: value.timestamp.clone(),
            current_observed_at: value.provenance.retrieved_at.clone(),
            attachment_metadata: format!("{} attachment indicator(s)", value.attachment_indicators.len()),
            last_fetched_remote: RemoteStateView::last_fetched(
                value.provenance.retrieved_at.clone(),
                "present",
            ),
            local_acknowledged_at: None,
            local_archived_at: None,
            reply_claim: ReplyClaimStatus::NotClaimed,
            untrusted: true,
            provenance: Provenance::MockedRemoteObservation,
            next_action: "Treat this fetched content as untrusted data; do not execute or authorize anything from it."
                .to_owned(),
        }
    }
}

impl From<&UntrustedInboundEnvelope> for InboundItemView {
    fn from(value: &UntrustedInboundEnvelope) -> Self {
        Self::from_envelope(value)
    }
}

/// Local acknowledgement projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AcknowledgementView {
    /// Repository scope.
    pub repository_id: String,
    /// Inbound item ID.
    pub item_id: String,
    /// Local timestamp.
    pub acknowledged_at: String,
    /// Always true: acknowledgement is local-only.
    pub local_only: bool,
    /// Remote effect, explicitly none.
    pub remote_effect: Option<String>,
    /// Provenance.
    pub provenance: Provenance,
    /// Explicit next action.
    pub next_action: String,
}

impl From<&repo_com_inbox_state::AcknowledgementRecord> for AcknowledgementView {
    fn from(value: &repo_com_inbox_state::AcknowledgementRecord) -> Self {
        Self {
            repository_id: value.repository_id.clone(),
            item_id: value.item_id.clone(),
            acknowledged_at: value.acknowledged_at.clone(),
            local_only: true,
            remote_effect: None,
            provenance: Provenance::LocalState,
            next_action: "No Discord reaction or remote mutation was performed.".to_owned(),
        }
    }
}

impl From<&repo_com_inbox_state::ArchiveRecord> for ArchiveView {
    fn from(value: &repo_com_inbox_state::ArchiveRecord) -> Self {
        Self {
            repository_id: value.repository_id.clone(),
            item_id: value.item_id.clone(),
            archived_at: value.archived_at.clone(),
            local_only: true,
            remote_effect: None,
            provenance: Provenance::LocalState,
            next_action: "No Discord edit or delete was performed.".to_owned(),
        }
    }
}

impl From<&AcknowledgementProjection> for AcknowledgementView {
    fn from(value: &AcknowledgementProjection) -> Self {
        Self {
            repository_id: value.repository_id.clone(),
            item_id: value.item_id.clone(),
            acknowledged_at: value.acknowledged_at.clone(),
            local_only: value.local_only,
            remote_effect: value.remote_effect.clone(),
            provenance: Provenance::LocalState,
            next_action: "No Discord reaction or remote mutation was performed.".to_owned(),
        }
    }
}

/// Local archive projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ArchiveView {
    /// Repository scope.
    pub repository_id: String,
    /// Inbound item ID.
    pub item_id: String,
    /// Local timestamp.
    pub archived_at: String,
    /// Always true: archival is local-only.
    pub local_only: bool,
    /// Remote effect, explicitly none.
    pub remote_effect: Option<String>,
    /// Provenance.
    pub provenance: Provenance,
    /// Explicit next action.
    pub next_action: String,
}

impl From<&ArchiveProjection> for ArchiveView {
    fn from(value: &ArchiveProjection) -> Self {
        Self {
            repository_id: value.repository_id.clone(),
            item_id: value.item_id.clone(),
            archived_at: value.archived_at.clone(),
            local_only: value.local_only,
            remote_effect: value.remote_effect.clone(),
            provenance: Provenance::LocalState,
            next_action: "No Discord edit or delete was performed.".to_owned(),
        }
    }
}

/// Local reply-link projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ReplyLinkView {
    /// Repository scope.
    pub repository_id: String,
    /// Inbound item ID.
    pub item_id: String,
    /// Local reply draft ID.
    pub reply_draft_id: String,
    /// Link timestamp.
    pub linked_at: String,
    /// Conservative reply evidence.
    pub reply_claim: ReplyClaimStatus,
    /// Accepted delivery ID, when verified.
    pub accepted_delivery_id: Option<String>,
    /// Accepted remote message ID, when verified.
    pub accepted_remote_message_id: Option<String>,
    /// Read receipt status.
    pub read_receipt: ReadReceiptStatus,
    /// Whether this link is local-only until accepted evidence exists.
    pub local_only_link: bool,
    /// Provenance.
    pub provenance: Provenance,
    /// Explicit next action.
    pub next_action: String,
}

impl From<&ReplyLinkProjection> for ReplyLinkView {
    fn from(value: &ReplyLinkProjection) -> Self {
        let reply_claim = match value.evidence_status {
            DomainReplyClaimStatus::NotClaimed => ReplyClaimStatus::NotClaimed,
            DomainReplyClaimStatus::AcceptedEvidence => ReplyClaimStatus::AcceptedEvidence,
            DomainReplyClaimStatus::Unverified => ReplyClaimStatus::Unverified,
        };
        Self {
            repository_id: value.repository_id.clone(),
            item_id: value.item_id.clone(),
            reply_draft_id: value.reply_draft_id.clone(),
            linked_at: value.linked_at.clone(),
            reply_claim,
            accepted_delivery_id: value.accepted_delivery_id.clone(),
            accepted_remote_message_id: value.accepted_remote_message_id.clone(),
            read_receipt: ReadReceiptStatus::Unavailable,
            local_only_link: value.local_only_link,
            provenance: Provenance::LocalState,
            next_action: "A reply link is not a read receipt; require accepted-delivery evidence before claiming a reply."
                .to_owned(),
        }
    }
}

/// Retention status.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum RetentionStatus {
    /// A valid policy is configured; no sweep result was supplied.
    Configured,
    /// A sweep committed count-only changes.
    Swept,
    /// A local integrity issue blocks mutation.
    Blocked,
    /// Policy or cutoff input is invalid.
    Invalid,
}

impl RetentionStatus {
    /// Returns stable text.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::Swept => "swept",
            Self::Blocked => "blocked",
            Self::Invalid => "invalid",
        }
    }
}

impl std::fmt::Display for RetentionStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Count-only retention sweep projection.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct RetentionCountsView {
    /// Content rows redacted.
    pub content_rows_redacted: usize,
    /// Metadata rows removed.
    pub metadata_rows_removed: usize,
}

impl From<&SweepCounts> for RetentionCountsView {
    fn from(counts: &SweepCounts) -> Self {
        Self {
            content_rows_redacted: counts.content_rows_redacted,
            metadata_rows_removed: counts.metadata_rows_removed,
        }
    }
}

/// Retention policy/status projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RetentionStatusView {
    /// Repository scope.
    pub repository_id: String,
    /// Content retention days.
    pub content_days: u32,
    /// Metadata retention days.
    pub metadata_days: u32,
    /// Current policy status.
    pub status: RetentionStatus,
    /// As-of timestamp, when supplied.
    pub as_of: Option<String>,
    /// Content cutoff, when supplied.
    pub content_cutoff: Option<String>,
    /// Metadata cutoff, when supplied.
    pub metadata_cutoff: Option<String>,
    /// Count-only sweep result, when supplied.
    pub counts: Option<RetentionCountsView>,
    /// Count-only local audit event ID, when supplied.
    pub audit_event_id: Option<String>,
    /// Stable local error code, when blocked or invalid.
    pub error_code: Option<String>,
    /// Redacted local error detail, when blocked or invalid.
    pub error_detail: Option<String>,
    /// Provenance.
    pub provenance: Provenance,
    /// Explicit next action.
    pub next_action: String,
}

impl RetentionStatusView {
    /// Projects a validated policy without running a sweep.
    #[must_use]
    pub fn from_policy(repository_id: impl Into<String>, policy: &RetentionPolicy) -> Self {
        Self {
            repository_id: repository_id.into(),
            content_days: policy.content_days,
            metadata_days: policy.metadata_days,
            status: RetentionStatus::Configured,
            as_of: None,
            content_cutoff: None,
            metadata_cutoff: None,
            counts: None,
            audit_event_id: None,
            error_code: None,
            error_detail: None,
            provenance: Provenance::LocalDecision,
            next_action:
                "Review the configured local retention periods; no retention mutation has run."
                    .to_owned(),
        }
    }

    /// Projects a committed count-only sweep.
    #[must_use]
    pub fn from_sweep(result: &SweepResult) -> Self {
        Self {
            repository_id: result.context.repository_id.clone(),
            content_days: 0,
            metadata_days: 0,
            status: RetentionStatus::Swept,
            as_of: Some(result.context.as_of.clone()),
            content_cutoff: Some(result.context.content_cutoff.clone()),
            metadata_cutoff: Some(result.context.metadata_cutoff.clone()),
            counts: Some(RetentionCountsView::from(&result.counts)),
            audit_event_id: Some(result.audit_event_id.clone()),
            error_code: None,
            error_detail: None,
            provenance: Provenance::LocalState,
            next_action: "Inspect count-only local evidence; retention does not mutate Discord."
                .to_owned(),
        }
    }

    /// Projects a blocked or invalid retention result.
    #[must_use]
    pub fn from_error(repository_id: impl Into<String>, error: &RetentionError) -> Self {
        let blocked = error.code() == "storage-integrity";
        Self {
            repository_id: repository_id.into(),
            content_days: 0,
            metadata_days: 0,
            status: if blocked {
                RetentionStatus::Blocked
            } else {
                RetentionStatus::Invalid
            },
            as_of: None,
            content_cutoff: None,
            metadata_cutoff: None,
            counts: None,
            audit_event_id: None,
            error_code: Some(error.code().to_owned()),
            error_detail: Some(error.to_string()),
            provenance: Provenance::LocalError,
            next_action: if blocked {
                "Mutation is blocked by local storage integrity; repair state before another sweep."
                    .to_owned()
            } else {
                "Correct the retention policy or cutoff input before retrying.".to_owned()
            },
        }
    }
}

/// Stable broad class for a local operations error.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LocalErrorKind {
    /// The local store is locked or busy.
    Locked,
    /// Local evidence is corrupt or structurally invalid.
    Corrupt,
    /// The requested local operation or version is unsupported.
    Unsupported,
    /// A required local object or file is missing.
    Missing,
    /// Another local operational failure occurred.
    Operational,
}

impl LocalErrorKind {
    /// Classifies a stable local code without retaining its detail.
    #[must_use]
    pub fn from_code(code: &str) -> Self {
        if code.contains("lock") || code.contains("busy") {
            Self::Locked
        } else if code.contains("corrupt") || code.contains("integrity") {
            Self::Corrupt
        } else if code.contains("unsupported") || code.contains("schema") {
            Self::Unsupported
        } else if code.contains("not-found") || code.contains("missing") {
            Self::Missing
        } else {
            Self::Operational
        }
    }

    /// Returns the stable text form.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Locked => "locked",
            Self::Corrupt => "corrupt",
            Self::Unsupported => "unsupported",
            Self::Missing => "missing",
            Self::Operational => "operational",
        }
    }
}

impl std::fmt::Display for LocalErrorKind {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// A safe local operations error.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LocalErrorView {
    /// Broad local error class.
    pub kind: LocalErrorKind,
    /// Repository scope, when known.
    pub repository_id: Option<String>,
    /// Object family, when known.
    pub object_type: Option<String>,
    /// Stable foundation error category.
    pub category: ErrorCategory,
    /// Stable domain/error code.
    pub code: String,
    /// Redacted detail.
    pub detail: String,
    /// Provenance.
    pub provenance: Provenance,
    /// Explicit next action.
    pub next_action: String,
}

impl LocalErrorView {
    /// Creates a safe error projection.
    #[must_use]
    pub fn new(
        category: ErrorCategory,
        code: impl Into<String>,
        detail: impl Into<String>,
        next_action: impl Into<String>,
    ) -> Self {
        Self {
            kind: LocalErrorKind::Operational,
            repository_id: None,
            object_type: None,
            category,
            code: code.into(),
            detail: detail.into(),
            provenance: Provenance::LocalError,
            next_action: next_action.into(),
        }
    }

    /// Sets the broad local error class.
    #[must_use]
    pub const fn with_kind(mut self, kind: LocalErrorKind) -> Self {
        self.kind = kind;
        self
    }

    /// Adds repository scope to a safe error projection.
    #[must_use]
    pub fn with_repository(mut self, repository_id: impl Into<String>) -> Self {
        self.repository_id = Some(repository_id.into());
        self
    }

    /// Adds object family to a safe error projection.
    #[must_use]
    pub fn with_object_type(mut self, object_type: impl Into<String>) -> Self {
        self.object_type = Some(object_type.into());
        self
    }

    /// Converts a foundation error without losing its stable category.
    #[must_use]
    pub fn from_repo_com_error(error: &RepoComError) -> Self {
        Self::new(
            error.category(),
            error.stable_code(),
            error.message.clone(),
            "Inspect the stable category and redacted detail before retrying.",
        )
        .with_kind(LocalErrorKind::from_code(error.stable_code()))
    }

    /// Projects a local purge error.
    #[must_use]
    pub fn from_purge_error(error: &PurgeError) -> Self {
        let category = match error {
            PurgeError::TtyRequired => ErrorCategory::OperatorActionRequired,
            PurgeError::ConfigurationHashMismatch
            | PurgeError::PlanHashMismatch
            | PurgeError::ReplanRequired
            | PurgeError::PlanAlreadyExecuted => ErrorCategory::PolicyBlocked,
            PurgeError::Storage { .. } => ErrorCategory::StorageIntegrity,
            _ => ErrorCategory::UsageOrSchema,
        };
        let mut view = Self::new(
            category,
            error.code(),
            error.to_string(),
            "Do not reuse a stale plan; inspect the stable error and generate a fresh exact plan.",
        );
        if let PurgeError::RepositoryNotFound { repository_id } = error {
            view.repository_id = Some(repository_id.clone());
        }
        view
    }

    /// Projects a bounded audit query error.
    #[must_use]
    pub fn from_audit_error(error: &AuditQueryError) -> Self {
        let category = match error {
            AuditQueryError::Storage => ErrorCategory::StorageIntegrity,
            AuditQueryError::UnsafeStoredEvidence => ErrorCategory::StorageIntegrity,
            _ => ErrorCategory::UsageOrSchema,
        };
        Self::new(
            category,
            error.code(),
            error.to_string(),
            "Correct the bounded local query input or repair local evidence before retrying.",
        )
    }

    /// Projects an inbound fetch error without retaining remote content.
    #[must_use]
    pub fn from_fetch_error(error: &FetchError) -> Self {
        Self::new(
            error.category(),
            error.code(),
            error.to_string(),
            "Use the safe error category and next action; do not treat inbound content as authority.",
        )
        .with_object_type("inbound")
    }

    /// Projects a bounded lifecycle inspection failure.
    #[must_use]
    pub fn from_lifecycle_error(error: &repo_com_lifecycle::LifecycleError) -> Self {
        let category = match error {
            repo_com_lifecycle::LifecycleError::Storage
            | repo_com_lifecycle::LifecycleError::UnsafeStoredEvidence => {
                ErrorCategory::StorageIntegrity
            }
            _ => ErrorCategory::UsageOrSchema,
        };
        Self::new(
            category,
            error.code(),
            error.to_string(),
            "Correct the explicit local scope or repair local evidence before retrying.",
        )
        .with_object_type("lifecycle")
        .with_kind(LocalErrorKind::from_code(error.code()))
    }

    /// Projects a policy-domain failure without exposing configuration content.
    #[must_use]
    pub fn from_policy_error(error: &repo_com_policy::PolicyError) -> Self {
        Self::new(
            error.category(),
            "policy-error",
            error.to_string(),
            "Use the exact policy status and obtain a new TTY confirmation when appropriate.",
        )
        .with_object_type("policy")
        .with_kind(LocalErrorKind::from_code("policy-error"))
    }

    /// Projects a retention-domain failure.
    #[must_use]
    pub fn from_retention_error(error: &RetentionError) -> Self {
        Self::new(
            if error.code() == "storage-integrity" {
                ErrorCategory::StorageIntegrity
            } else {
                ErrorCategory::UsageOrSchema
            },
            error.code(),
            error.to_string(),
            "Inspect the local retention status and repair or correct it before retrying.",
        )
        .with_object_type("retention")
        .with_kind(LocalErrorKind::from_code(error.code()))
    }
}

/// Every operations presentation view owned by this crate.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "view", rename_all = "kebab-case")]
pub enum OperationsView {
    /// Configuration validation status.
    Config(ConfigStatusView),
    /// Exact policy status.
    Policy(PolicyStatusView),
    /// Exact activation preview.
    Activation(ActivationPreviewView),
    /// Local state verification.
    StateVerification(StateVerificationView),
    /// Bounded local audit page.
    Audit(AuditView),
    /// One lifecycle record.
    Lifecycle(LifecycleView),
    /// Bounded lifecycle page.
    LifecyclePage(LifecyclePageView),
    /// Untrusted inbound item.
    Inbound(InboundItemView),
    /// Local inbound page commit.
    InboundCommit(InboundCommitView),
    /// Local acknowledgement.
    Acknowledgement(AcknowledgementView),
    /// Local archive.
    Archive(ArchiveView),
    /// Local reply link.
    ReplyLink(ReplyLinkView),
    /// Retention policy or sweep status.
    Retention(RetentionStatusView),
    /// Non-mutating purge plan.
    PurgePlan(PurgePlanView),
    /// Confirmed purge result or blocked result.
    PurgeExecution(PurgeExecutionView),
    /// Local operational error.
    Error(LocalErrorView),
}

/// Compatibility alias emphasizing the operations surface.
pub type OperationView = OperationsView;

impl OperationsView {
    /// Creates a configuration view.
    #[must_use]
    pub fn config(view: ConfigStatusView) -> Self {
        Self::Config(view)
    }

    /// Creates a policy view.
    #[must_use]
    pub fn policy(view: PolicyStatusView) -> Self {
        Self::Policy(view)
    }

    /// Creates an activation preview view.
    #[must_use]
    pub fn activation(view: ActivationPreviewView) -> Self {
        Self::Activation(view)
    }

    /// Creates a state verification view.
    #[must_use]
    pub fn state_verification(view: StateVerificationView) -> Self {
        Self::StateVerification(view)
    }

    /// Creates an audit view.
    #[must_use]
    pub fn audit(view: AuditView) -> Self {
        Self::Audit(view)
    }

    /// Creates a lifecycle record view.
    #[must_use]
    pub fn lifecycle(view: LifecycleView) -> Self {
        Self::Lifecycle(view)
    }

    /// Creates a bounded lifecycle page view.
    #[must_use]
    pub fn lifecycle_page(view: LifecyclePageView) -> Self {
        Self::LifecyclePage(view)
    }

    /// Creates an inbound view.
    #[must_use]
    pub fn inbound(view: InboundItemView) -> Self {
        Self::Inbound(view)
    }

    /// Creates a local inbound commit view.
    #[must_use]
    pub fn inbound_commit(view: InboundCommitView) -> Self {
        Self::InboundCommit(view)
    }

    /// Creates an acknowledgement view.
    #[must_use]
    pub fn acknowledgement(view: AcknowledgementView) -> Self {
        Self::Acknowledgement(view)
    }

    /// Creates an archive view.
    #[must_use]
    pub fn archive(view: ArchiveView) -> Self {
        Self::Archive(view)
    }

    /// Creates a reply-link view.
    #[must_use]
    pub fn reply_link(view: ReplyLinkView) -> Self {
        Self::ReplyLink(view)
    }

    /// Creates a retention view.
    #[must_use]
    pub fn retention(view: RetentionStatusView) -> Self {
        Self::Retention(view)
    }

    /// Creates a purge plan view.
    #[must_use]
    pub fn purge_plan(view: PurgePlanView) -> Self {
        Self::PurgePlan(view)
    }

    /// Creates a purge execution view.
    #[must_use]
    pub fn purge_execution(view: PurgeExecutionView) -> Self {
        Self::PurgeExecution(view)
    }

    /// Creates an error view.
    #[must_use]
    pub fn error(view: LocalErrorView) -> Self {
        Self::Error(view)
    }

    /// Returns the stable view name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Config(_) => "config",
            Self::Policy(_) => "policy",
            Self::Activation(_) => "activation",
            Self::StateVerification(_) => "state-verification",
            Self::Audit(_) => "audit",
            Self::Lifecycle(_) => "lifecycle",
            Self::LifecyclePage(_) => "lifecycle-page",
            Self::Inbound(_) => "inbound",
            Self::InboundCommit(_) => "inbound-commit",
            Self::Acknowledgement(_) => "acknowledgement",
            Self::Archive(_) => "archive",
            Self::ReplyLink(_) => "reply-link",
            Self::Retention(_) => "retention",
            Self::PurgePlan(_) => "purge-plan",
            Self::PurgeExecution(_) => "purge-execution",
            Self::Error(_) => "error",
        }
    }

    /// Renders this view as complete labeled human text.
    #[must_use]
    pub fn render(&self, options: RenderOptions) -> String {
        render_view(self, options)
    }

    /// Serializes this view as one deterministic JSON value.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Serializes this view in one protocol-version-1 envelope.
    pub fn to_protocol_json(&self) -> Result<String, serde_json::Error> {
        match self {
            Self::Error(error) => render_machine_failure(error),
            _ => CommandOutcome::success(self.clone()).to_json(),
        }
    }
}

/// Reusable stateless operations renderer.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OperationsRenderer {
    options: RenderOptions,
}

impl OperationsRenderer {
    /// Creates a renderer with explicit options.
    #[must_use]
    pub const fn new(options: RenderOptions) -> Self {
        Self { options }
    }

    /// Creates the plain-text 80-column renderer.
    #[must_use]
    pub const fn plain_text() -> Self {
        Self::new(RenderOptions::plain_text())
    }

    /// Renders one view without performing I/O.
    #[must_use]
    pub fn render(&self, view: &OperationsView) -> String {
        render_view(view, self.options)
    }

    /// Serializes one view as a protocol envelope.
    pub fn render_machine(&self, view: &OperationsView) -> Result<String, serde_json::Error> {
        render_machine(view)
    }
}

/// Renders one operations view.
#[must_use]
pub fn render_view(view: &OperationsView, options: RenderOptions) -> String {
    let mut output = Output::new(options);
    match view {
        OperationsView::Config(value) => render_config_into(value, &mut output),
        OperationsView::Policy(value) => render_policy_into(value, &mut output),
        OperationsView::Activation(value) => render_activation_into(value, &mut output),
        OperationsView::StateVerification(value) => {
            render_state_verification_into(value, &mut output)
        }
        OperationsView::Audit(value) => render_audit_into(value, &mut output),
        OperationsView::Lifecycle(value) => render_lifecycle_into(value, &mut output),
        OperationsView::LifecyclePage(value) => render_lifecycle_page_into(value, &mut output),
        OperationsView::Inbound(value) => render_inbound_into(value, &mut output),
        OperationsView::InboundCommit(value) => render_inbound_commit_into(value, &mut output),
        OperationsView::Acknowledgement(value) => render_acknowledgement_into(value, &mut output),
        OperationsView::Archive(value) => render_archive_into(value, &mut output),
        OperationsView::ReplyLink(value) => render_reply_link_into(value, &mut output),
        OperationsView::Retention(value) => render_retention_into(value, &mut output),
        OperationsView::PurgePlan(value) => render_purge_plan_into(value, &mut output),
        OperationsView::PurgeExecution(value) => render_purge_execution_into(value, &mut output),
        OperationsView::Error(value) => render_error_into(value, &mut output),
    }
    output.finish()
}

/// Compatibility name for the primary operations renderer.
#[must_use]
pub fn render_operations(view: &OperationsView, options: RenderOptions) -> String {
    render_view(view, options)
}

/// Renders a successful operations view as one protocol-version-1 envelope.
pub fn render_machine(view: &OperationsView) -> Result<String, serde_json::Error> {
    view.to_protocol_json()
}

/// Renders a safe local error as one protocol-version-1 failure envelope.
pub fn render_machine_failure(error: &LocalErrorView) -> Result<String, serde_json::Error> {
    CommandOutcome::<OperationsView>::failure(RepoComError::new(
        error.category,
        format!("{}; next action: {}", error.detail, error.next_action),
    ))
    .to_json()
}

/// Renders a configuration status view directly.
#[must_use]
pub fn render_config(view: &ConfigStatusView, options: RenderOptions) -> String {
    render_view(&OperationsView::Config(view.clone()), options)
}

/// Renders a policy status view directly.
#[must_use]
pub fn render_policy(view: &PolicyStatusView, options: RenderOptions) -> String {
    render_view(&OperationsView::Policy(view.clone()), options)
}

/// Renders an exact activation preview directly.
#[must_use]
pub fn render_activation(view: &ActivationPreviewView, options: RenderOptions) -> String {
    render_view(&OperationsView::Activation(view.clone()), options)
}

/// Renders a state verification view directly.
#[must_use]
pub fn render_state_verification(view: &StateVerificationView, options: RenderOptions) -> String {
    render_view(&OperationsView::StateVerification(view.clone()), options)
}

/// Renders an audit view directly.
#[must_use]
pub fn render_audit(view: &AuditView, options: RenderOptions) -> String {
    render_view(&OperationsView::Audit(view.clone()), options)
}

/// Renders a lifecycle view directly.
#[must_use]
pub fn render_lifecycle(view: &LifecycleView, options: RenderOptions) -> String {
    render_view(&OperationsView::Lifecycle(view.clone()), options)
}

/// Renders an inbound item view directly.
#[must_use]
pub fn render_inbound(view: &InboundItemView, options: RenderOptions) -> String {
    render_view(&OperationsView::Inbound(view.clone()), options)
}

/// Renders a bounded lifecycle page directly.
#[must_use]
pub fn render_lifecycle_page(view: &LifecyclePageView, options: RenderOptions) -> String {
    render_view(&OperationsView::LifecyclePage(view.clone()), options)
}

/// Renders a local acknowledgement view directly.
#[must_use]
pub fn render_acknowledgement(view: &AcknowledgementView, options: RenderOptions) -> String {
    render_view(&OperationsView::Acknowledgement(view.clone()), options)
}

/// Renders a local archive view directly.
#[must_use]
pub fn render_archive(view: &ArchiveView, options: RenderOptions) -> String {
    render_view(&OperationsView::Archive(view.clone()), options)
}

/// Renders a local reply-link view directly.
#[must_use]
pub fn render_reply_link(view: &ReplyLinkView, options: RenderOptions) -> String {
    render_view(&OperationsView::ReplyLink(view.clone()), options)
}

/// Renders a local inbound commit view directly.
#[must_use]
pub fn render_inbound_commit(view: &InboundCommitView, options: RenderOptions) -> String {
    render_view(&OperationsView::InboundCommit(view.clone()), options)
}

/// Renders a retention view directly.
#[must_use]
pub fn render_retention(view: &RetentionStatusView, options: RenderOptions) -> String {
    render_view(&OperationsView::Retention(view.clone()), options)
}

/// Renders a purge plan view directly.
#[must_use]
pub fn render_purge_plan(view: &PurgePlanView, options: RenderOptions) -> String {
    render_view(&OperationsView::PurgePlan(view.clone()), options)
}

/// Renders a purge execution view directly.
#[must_use]
pub fn render_purge_execution(view: &PurgeExecutionView, options: RenderOptions) -> String {
    render_view(&OperationsView::PurgeExecution(view.clone()), options)
}

/// Renders a local error view directly.
#[must_use]
pub fn render_error(view: &LocalErrorView, options: RenderOptions) -> String {
    render_view(&OperationsView::Error(view.clone()), options)
}

/// Returns the longest visible line width.
#[must_use]
pub fn rendered_width(value: &str) -> usize {
    value.lines().map(display_width).max().unwrap_or(0)
}

/// Returns whether output contains no ANSI escapes.
#[must_use]
pub fn is_ansi_free(value: &str) -> bool {
    strip_ansi(value) == value
}

struct Output {
    options: RenderOptions,
    lines: Vec<String>,
}

impl Output {
    fn new(options: RenderOptions) -> Self {
        Self {
            options,
            lines: Vec::new(),
        }
    }

    fn finish(self) -> String {
        let mut result = self.lines.join("\n");
        result.push('\n');
        result
    }

    fn push_line(&mut self, line: impl Into<String>) {
        let line = line.into();
        debug_assert!(display_width(&line) <= self.options.columns());
        self.lines.push(line);
    }

    fn push_lines(&mut self, lines: impl IntoIterator<Item = String>) {
        for line in lines {
            self.push_line(line);
        }
    }

    fn heading(&mut self, text: &str) {
        let line = if self.options.ansi_enabled() {
            format!("\u{1b}[1m{text}\u{1b}[0m")
        } else {
            text.to_owned()
        };
        self.push_line(line);
    }

    fn field(&mut self, label: &str, value: impl AsRef<str>) {
        let value = value.as_ref();
        let value = if value.is_empty() { "(none)" } else { value };
        self.push_lines(field_lines(label, value, self.options.columns()));
    }

    fn optional(&mut self, label: &str, value: Option<&str>) {
        self.field(label, value.unwrap_or("(none)"));
    }

    fn block(&mut self, label: &str, value: &str) {
        self.push_lines(block_lines(label, value, self.options.columns()));
    }

    fn provenance(&mut self, provenance: Provenance) {
        self.field("Provenance", provenance.as_str());
    }

    fn details(&mut self, details: &[DetailField]) {
        for detail in details {
            self.field(&detail.label, &detail.value);
        }
    }
}

fn render_config_into(view: &ConfigStatusView, output: &mut Output) {
    output.heading("Configuration status");
    output.optional("Repository", view.repository_id.as_deref());
    output.field("Object", "repository configuration");
    output.optional("Configuration path", view.config_path.as_deref());
    output.optional(
        "Schema version",
        view.schema_version
            .map(|value| value.to_string())
            .as_deref(),
    );
    output.optional("Workspace", view.workspace_id.as_deref());
    output.optional("Configuration hash", view.config_hash.as_deref());
    output.field("Validation status", view.status.as_str());
    output.optional("Validation code", view.validation_code.as_deref());
    output.optional("Validation detail", view.detail.as_deref());
    output.field(
        "Destination aliases",
        join_or_none(&view.destination_aliases),
    );
    output.field("Inbound aliases", join_or_none(&view.inbound_aliases));
    output.field("Auto-send entries", view.auto_send_entries.to_string());
    let outcome = match view.status {
        ConfigStatus::Valid => "configuration-valid",
        ConfigStatus::Missing => "configuration-missing",
        ConfigStatus::UnsupportedSchema => "configuration-unsupported",
        ConfigStatus::Invalid => "configuration-invalid",
        ConfigStatus::SecretField => "configuration-secret-field-rejected",
    };
    output.field("Outcome", outcome);
    output.provenance(view.provenance);
    output.field("Next action", &view.next_action);
}

fn render_policy_into(view: &PolicyStatusView, output: &mut Output) {
    output.heading("Policy status");
    output.field("Repository", &view.repository_id);
    output.field("Object", "exact policy tuple");
    output.optional(
        "Policy tuple",
        view.tuple.as_ref().map(ToString::to_string).as_deref(),
    );
    output.field("Policy state", view.state.as_str());
    output.optional("Policy config hash", view.config_hash.as_deref());
    output.optional("Policy tuple hash", view.tuple_hash.as_deref());
    output.optional("Activation ID", view.activation_id.as_deref());
    output.field("Policy basis", &view.basis);
    for (index, activation) in view.activations.iter().enumerate() {
        output.heading("Policy activation");
        output.field(
            &format!("Activation {}", index + 1),
            &activation.activation_id,
        );
        output.field(
            &format!("Activation {} repository", index + 1),
            &activation.repository_id,
        );
        output.field(
            &format!("Activation {} tuple", index + 1),
            activation.tuple.to_string(),
        );
        output.field(
            &format!("Activation {} recorded config hash", index + 1),
            &activation.recorded_config_hash,
        );
        output.field(
            &format!("Activation {} recorded tuple hash", index + 1),
            &activation.recorded_tuple_hash,
        );
        output.field(
            &format!("Activation {} current config hash", index + 1),
            &activation.current_config_hash,
        );
        output.field(
            &format!("Activation {} current tuple hash", index + 1),
            &activation.current_tuple_hash,
        );
        output.field(
            &format!("Activation {} activated at", index + 1),
            &activation.activated_at,
        );
        output.optional(
            &format!("Activation {} deactivated at", index + 1),
            activation.deactivated_at.as_deref(),
        );
        output.field(
            &format!("Activation {} active", index + 1),
            activation.active.to_string(),
        );
        output.optional(
            &format!("Activation {} stale reason", index + 1),
            activation.stale_reason.as_deref(),
        );
    }
    output.field("Outcome", view.state.as_str());
    output.provenance(view.provenance);
    output.field("Next action", &view.next_action);
}

fn render_activation_into(view: &ActivationPreviewView, output: &mut Output) {
    output.heading("Policy activation preview");
    output.field("Repository", &view.repository_id);
    output.field("Object", "policy activation");
    output.field("Activation ID", &view.activation_id);
    output.field("Policy tuple", view.tuple.to_string());
    output.field("Policy state", "not-activated");
    output.field("Configuration hash", &view.config_hash);
    output.field("Policy tuple hash", &view.tuple_hash);
    output.field(
        "Policy basis",
        "exact tuple and hashes; explicit operator TTY confirmation required",
    );
    output.field("Outcome", "activation-not-recorded");
    output.provenance(view.provenance);
    output.field("Next action", &view.next_action);
}

fn render_state_verification_into(view: &StateVerificationView, output: &mut Output) {
    output.heading("Local state verification");
    output.field("Repository", &view.repository_id);
    output.field("Object", "local SQLite state");
    output.field("Read only", view.read_only.to_string());
    output.field(
        "Connection read only",
        view.connection_read_only.to_string(),
    );
    output.field("Healthy", view.healthy.to_string());
    output.field("Privacy disclosure", &view.privacy_disclosure);
    render_check(output, "Quick check", &view.quick_check);
    render_check(output, "Foreign keys", &view.foreign_keys);
    render_check(output, "Migration", &view.migration);
    render_check(output, "Repository scope", &view.repository_scope);
    render_check(
        output,
        "Filesystem permissions",
        &view.filesystem_permissions,
    );
    output.heading("Verification issues");
    if view.issues.is_empty() {
        output.field("Issue", "none");
    } else {
        output.details(&view.issues);
    }
    output.heading("Remediation");
    if view.remediation.is_empty() {
        output.field("Remediation", "none");
    } else {
        for (index, item) in view.remediation.iter().enumerate() {
            output.field(&format!("Remediation {}", index + 1), item);
        }
    }
    output.field(
        "Outcome",
        if view.healthy {
            "healthy"
        } else {
            "integrity-attention-required"
        },
    );
    output.provenance(view.provenance);
    output.field("Next action", &view.next_action);
}

fn render_check(output: &mut Output, label: &str, check: &StateCheckView) {
    output.heading(label);
    output.field(&format!("{label} status"), check_status_text(check.status));
    output.field(&format!("{label} passed"), check.passed.to_string());
    output.field(&format!("{label} code"), &check.code);
    output.details(&check.details);
}

fn render_audit_into(view: &AuditView, output: &mut Output) {
    output.heading("Bounded local audit");
    output.field("Repository", &view.repository_id);
    output.field("Object", "audit transitions");
    output.field("Page size", view.page_size.to_string());
    output.field("Event count", view.event_count.to_string());
    output.field("Has more local events", view.has_more.to_string());
    output.optional("Next cursor", view.next_cursor.as_deref());
    output.field(
        "Remote fetch performed",
        view.remote_fetch_performed.to_string(),
    );
    for (index, event) in view.events.iter().enumerate() {
        output.heading("Audit event");
        output.field(
            &format!("Event {} audit ID", index + 1),
            event.audit_id.to_string(),
        );
        output.field(&format!("Event {} ID", index + 1), &event.event_id);
        output.field(
            &format!("Event {} object", index + 1),
            format!("{} {}", event.object_type, event.object_id),
        );
        output.field(
            &format!("Event {} transition", index + 1),
            &event.transition,
        );
        output.field(
            &format!("Event {} occurred at", index + 1),
            &event.occurred_at,
        );
        output.field(&format!("Event {} actor", index + 1), &event.actor_kind);
        output.field(&format!("Event {} outcome", index + 1), &event.outcome);
        let metadata = serde_json::to_string(&event.metadata).unwrap_or_else(|_| "{}".to_owned());
        output.block(&format!("Event {} redacted metadata", index + 1), &metadata);
    }
    output.field("Outcome", "local-audit-page");
    output.provenance(view.provenance);
    output.field("Next action", &view.next_action);
}

fn render_lifecycle_into(view: &LifecycleView, output: &mut Output) {
    output.heading("Lifecycle inspection");
    output.field("Repository", &view.repository_id);
    output.field("Object", format!("{} {}", view.object_type, view.object_id));
    output.optional(
        "Revision",
        view.revision.map(|value| value.to_string()).as_deref(),
    );
    output.field("State", &view.state);
    output.field("Local state", &view.local_state);
    output.field("Untrusted inbound", view.untrusted.to_string());
    output.field("Read receipt", view.read_receipt.as_str());
    output.field("Reply claim", view.reply_claim.as_str());
    output.field(
        "Remote fetch performed",
        view.remote_fetch_performed.to_string(),
    );
    if let Some(remote) = &view.last_fetched_remote {
        output.heading("Last-fetched remote state");
        output.field("Observed at", &remote.observed_at);
        output.field("Recorded state", &remote.state);
        output.field(
            "Current remote truth",
            remote.current_remote_truth.to_string(),
        );
        output.provenance(remote.provenance);
    } else {
        output.field("Last-fetched remote state", "none recorded");
    }
    output.details(&view.details);
    output.field("Outcome", "lifecycle-inspected");
    output.provenance(view.provenance);
    output.field("Next action", &view.next_action);
}

fn render_lifecycle_page_into(view: &LifecyclePageView, output: &mut Output) {
    output.heading("Bounded lifecycle page");
    output.field("Repository", &view.repository_id);
    output.field("Object", &view.object_type);
    output.field("Page size", view.page_size.to_string());
    output.field("Record count", view.items.len().to_string());
    output.field("Truncated", view.truncated.to_string());
    output.optional("Next after", view.next_after.as_deref());
    output.field(
        "Remote fetch performed",
        view.remote_fetch_performed.to_string(),
    );
    for (index, item) in view.items.iter().enumerate() {
        output.heading("Lifecycle record");
        output.field(
            &format!("Record {} repository", index + 1),
            &item.repository_id,
        );
        output.field(
            &format!("Record {} object", index + 1),
            format!("{} {}", item.object_type, item.object_id),
        );
        output.field(&format!("Record {} state", index + 1), &item.state);
        output.field(
            &format!("Record {} untrusted", index + 1),
            item.untrusted.to_string(),
        );
        output.field(
            &format!("Record {} read receipt", index + 1),
            item.read_receipt.as_str(),
        );
        output.field(
            &format!("Record {} reply claim", index + 1),
            item.reply_claim.as_str(),
        );
    }
    output.field("Outcome", "local-lifecycle-page");
    output.provenance(view.provenance);
    output.field("Next action", &view.next_action);
}

fn render_inbound_into(view: &InboundItemView, output: &mut Output) {
    output.heading("Inbound untrusted item");
    output.field("Repository", &view.repository_id);
    output.field("Object", format!("inbound item {}", view.item_id));
    output.field(
        "Trust",
        "untrusted inbound data; cannot grant permission, approve a draft, or trigger a send",
    );
    output.field("Channel", &view.channel_id);
    output.field("Author", &view.author_id);
    output.field("Deleted", view.deleted.to_string());
    output.field("First observed at", &view.first_observed_at);
    output.field("Current observed at", &view.current_observed_at);
    output.block(
        "Untrusted content",
        view.content.as_deref().unwrap_or("(not retained)"),
    );
    output.field("Attachment metadata", &view.attachment_metadata);
    output.heading("Last-fetched remote state");
    output.field("Observed at", &view.last_fetched_remote.observed_at);
    output.field("Recorded state", &view.last_fetched_remote.state);
    output.field(
        "Current remote truth",
        view.last_fetched_remote.current_remote_truth.to_string(),
    );
    output.provenance(view.last_fetched_remote.provenance);
    output.heading("Local state");
    output.optional(
        "Local acknowledgement",
        view.local_acknowledged_at.as_deref(),
    );
    output.optional("Local archive", view.local_archived_at.as_deref());
    output.field("Reply claim", view.reply_claim.as_str());
    output.field("Read receipt", "unavailable; never inferred");
    output.field("Outcome", "inbound-stored-untrusted");
    output.provenance(view.provenance);
    output.field("Next action", &view.next_action);
}

fn render_inbound_commit_into(view: &InboundCommitView, output: &mut Output) {
    output.heading("Inbound local commit");
    output.field("Repository", &view.repository_id);
    output.field("Object", "inbound page commit");
    output.field("Trust", "stored inbound data remains untrusted");
    output.field("Alias", &view.alias);
    output.field("Authoritative cursor", &view.cursor);
    output.field("Stored items", view.stored_items.to_string());
    output.field("Stored transitions", view.stored_transitions.to_string());
    output.field("Outcome", "inbound-page-committed-locally");
    output.provenance(view.provenance);
    output.field("Next action", &view.next_action);
}

fn render_acknowledgement_into(view: &AcknowledgementView, output: &mut Output) {
    output.heading("Local acknowledgement");
    output.field("Repository", &view.repository_id);
    output.field("Object", format!("acknowledgement {}", view.item_id));
    output.field("Acknowledged at", &view.acknowledged_at);
    output.field("Local only", view.local_only.to_string());
    output.optional("Remote effect", view.remote_effect.as_deref());
    output.field("Trust", "inbound item remains untrusted");
    output.field("Outcome", "acknowledged-locally");
    output.provenance(view.provenance);
    output.field("Next action", &view.next_action);
}

fn render_archive_into(view: &ArchiveView, output: &mut Output) {
    output.heading("Local archive");
    output.field("Repository", &view.repository_id);
    output.field("Object", format!("archive {}", view.item_id));
    output.field("Archived at", &view.archived_at);
    output.field("Local only", view.local_only.to_string());
    output.optional("Remote effect", view.remote_effect.as_deref());
    output.field("Trust", "inbound item remains untrusted");
    output.field("Outcome", "archived-locally");
    output.provenance(view.provenance);
    output.field("Next action", &view.next_action);
}

fn render_reply_link_into(view: &ReplyLinkView, output: &mut Output) {
    output.heading("Local reply link");
    output.field("Repository", &view.repository_id);
    output.field("Object", format!("reply link {}", view.item_id));
    output.field("Reply draft ID", &view.reply_draft_id);
    output.field("Linked at", &view.linked_at);
    output.field("Reply claim", view.reply_claim.as_str());
    output.optional("Accepted delivery ID", view.accepted_delivery_id.as_deref());
    output.optional(
        "Accepted remote message ID",
        view.accepted_remote_message_id.as_deref(),
    );
    output.field("Read receipt", view.read_receipt.as_str());
    output.field("Local-only link", view.local_only_link.to_string());
    output.field("Outcome", "reply-link-inspected");
    output.provenance(view.provenance);
    output.field("Next action", &view.next_action);
}

fn render_retention_into(view: &RetentionStatusView, output: &mut Output) {
    output.heading("Retention status");
    output.field("Repository", &view.repository_id);
    output.field("Object", "local retention policy");
    output.field("Retention status", view.status.as_str());
    output.field("Content retention days", view.content_days.to_string());
    output.field("Metadata retention days", view.metadata_days.to_string());
    output.optional("As of", view.as_of.as_deref());
    output.optional("Content cutoff", view.content_cutoff.as_deref());
    output.optional("Metadata cutoff", view.metadata_cutoff.as_deref());
    if let Some(counts) = &view.counts {
        output.field(
            "Content rows redacted",
            counts.content_rows_redacted.to_string(),
        );
        output.field(
            "Metadata rows removed",
            counts.metadata_rows_removed.to_string(),
        );
    }
    output.optional("Audit event ID", view.audit_event_id.as_deref());
    output.optional("Error code", view.error_code.as_deref());
    output.optional("Error detail", view.error_detail.as_deref());
    output.field("Outcome", view.status.as_str());
    output.provenance(view.provenance);
    output.field("Next action", &view.next_action);
}

fn render_purge_plan_into(view: &PurgePlanView, output: &mut Output) {
    output.heading("Purge plan");
    output.field("Repository", &view.repository_id);
    output.field("Object", "local purge plan");
    output.field("Plan schema", view.schema_version.to_string());
    output.field("Scope", view.scope.as_str());
    output.field("Cutoff Unix seconds", view.cutoff_unix_seconds.to_string());
    output.field("Cutoff UTC", &view.cutoff_utc);
    output.field("Configuration hash", &view.config_hash);
    output.field("Content rows", view.counts.content_count().to_string());
    output.field("Metadata rows", view.counts.metadata_count().to_string());
    output.field("Total rows", view.counts.total_count().to_string());
    for (table, count) in &view.counts.table_rows {
        output.field(&format!("Count {table}"), count.to_string());
    }
    output.field("State fingerprint", &view.state_fingerprint);
    output.field("Plan hash", &view.plan_hash);
    output.field("Execution performed", view.execution_performed.to_string());
    output.field("Outcome", "plan-only");
    output.field("Provenance", view.provenance.as_str());
    output.field("Next action", "Review every exact scope, count, and hash; obtain a TTY confirmation before any local purge.");
}

fn render_purge_execution_into(view: &PurgeExecutionView, output: &mut Output) {
    output.heading("Purge execution result");
    output.field("Repository", &view.repository_id);
    output.field("Object", "confirmed local purge");
    output.field("Plan hash", &view.plan_hash);
    output.field("Execution state", view.state.as_str());
    output.field(
        "Execution performed",
        (view.state == PurgeExecutionState::Executed).to_string(),
    );
    if let Some(counts) = &view.counts {
        output.field("Content rows", counts.content_count().to_string());
        output.field("Metadata rows", counts.metadata_count().to_string());
        output.field("Total rows", counts.total_count().to_string());
        for (table, count) in &counts.table_rows {
            output.field(&format!("Count {table}"), count.to_string());
        }
    }
    output.optional("Audit event ID", view.audit_event_id.as_deref());
    output.optional("Executed at", view.executed_at.as_deref());
    output.optional("Error code", view.error_code.as_deref());
    output.optional("Error detail", view.error_detail.as_deref());
    output.field("Outcome", view.state.as_str());
    output.field("Provenance", view.provenance.as_str());
    output.field("Next action", &view.next_action);
}

fn render_error_into(view: &LocalErrorView, output: &mut Output) {
    output.heading("Local operations error");
    output.optional("Repository", view.repository_id.as_deref());
    output.optional("Object", view.object_type.as_deref());
    output.field("Error kind", view.kind.as_str());
    output.field("Error category", view.category.code());
    output.field("Error code", &view.code);
    output.block("Error detail", &view.detail);
    output.field("Outcome", "error");
    output.provenance(view.provenance);
    output.field("Next action", &view.next_action);
}

fn map_check_status(status: repo_com_lifecycle::CheckStatus) -> CheckState {
    match status {
        repo_com_lifecycle::CheckStatus::Passed => CheckState::Passed,
        repo_com_lifecycle::CheckStatus::Failed => CheckState::Failed,
        repo_com_lifecycle::CheckStatus::Unavailable => CheckState::Unavailable,
    }
}

fn check_status_text(status: CheckState) -> &'static str {
    status.as_str()
}

fn join_or_none(values: &[String]) -> String {
    if values.is_empty() {
        "(none)".to_owned()
    } else {
        values.join(", ")
    }
}

// Keep the upstream domain type visible to callers without duplicating it in
// the operations contract.
pub use repo_com_policy::PolicyTuple;

/// Compatibility aliases for callers that use the shorter projection names.
pub type ConfigView = ConfigStatusView;
pub type PolicyView = PolicyStatusView;
pub type StateView = StateVerificationView;
pub type LocalStateView = StateVerificationView;
pub type InboundView = InboundItemView;
pub type InboundStatusView = InboundItemView;
pub type InboundItemStatusView = InboundItemView;
pub type AuditStatusView = AuditView;
pub type LifecycleStatusView = LifecycleView;
pub type PolicyActivationView = ActivationPreviewView;
pub type PolicyActivationStatusView = PolicyStatusView;
pub type StateVerificationStatusView = StateVerificationView;
pub type AcknowledgementStatusView = AcknowledgementView;
pub type ArchiveStatusView = ArchiveView;
pub type RetentionView = RetentionStatusView;
pub type RetentionPolicyView = RetentionStatusView;
pub type PurgeView = PurgePlanView;
pub type PurgeResultView = PurgeExecutionView;
pub type ErrorView = LocalErrorView;

/// Minimum-width re-export for contract tests and downstream adapters.
pub const OPERATIONS_MIN_TERMINAL_WIDTH: usize = MIN_TERMINAL_WIDTH;
