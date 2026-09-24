//! Bounded, repository-scoped lifecycle projections.
//!
//! Every query in this module is a `SELECT` (or a local read-only pragma used
//! by the verifier). Projections retain identifiers, hashes, timestamps, and
//! state labels. Message text is carried only in [`RetainedContent`], which is
//! populated when the caller explicitly opts in and is redacted in `Debug`
//! output even then.

use std::fmt;

use repo_com_audit::{REDACTED, redact_metadata, redact_text};
use repo_com_audit_query::{
    AuditCursor, AuditFilter, AuditPage, AuditQuery, AuditQueryError,
    MAX_PAGE_SIZE as AUDIT_MAX_PAGE_SIZE,
};
use repo_com_state::StateStore;
use rusqlite::{Connection, OptionalExtension, Row, named_params, params};
use serde::Serialize;
use serde_json::Value;

use crate::{LifecycleError, LifecycleResult};

/// Default number of rows returned by one non-audit lifecycle page.
pub const DEFAULT_PAGE_SIZE: usize = 50;
/// Maximum number of rows returned by one non-audit lifecycle page.
pub const MAX_PAGE_SIZE: usize = 100;
/// Stable object-family label for repository identity rows.
pub const REPOSITORY_OBJECT: &str = "repository";
/// Stable object-family label for draft parent rows.
pub const DRAFT_OBJECT: &str = "draft";
/// Stable object-family label for immutable draft revisions.
pub const DRAFT_REVISION_OBJECT: &str = "draft_revision";
/// Stable object-family label for delivery attempts.
pub const DELIVERY_ATTEMPT_OBJECT: &str = "delivery_attempt";
/// Stable object-family label for inbound items.
pub const INBOUND_ITEM_OBJECT: &str = "inbound_item";
/// Stable object-family label for local acknowledgements.
pub const ACKNOWLEDGEMENT_OBJECT: &str = "acknowledgement";
/// Stable object-family label for local archive markers.
pub const ARCHIVE_OBJECT: &str = "archive";
/// Stable object-family label for local reply links.
pub const REPLY_LINK_OBJECT: &str = "reply_link";
/// Stable object-family label for local audit transitions.
pub const AUDIT_OBJECT: &str = "audit_transition";

const CURSOR_SEPARATOR: char = '|';
const REVISION_SEPARATOR: char = '~';
const MAX_CURSOR_LENGTH: usize = 1_024;
const MAX_SAFE_TEXT_LENGTH: usize = 16 * 1_024;

/// Content that is returned only after an explicit caller opt-in.
///
/// `Debug` never prints the underlying value. `Serialize` intentionally does
/// expose it so an explicitly authorized caller can render the retained item;
/// ordinary projections leave the field absent.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct RetainedContent(String);

impl RetainedContent {
    /// Creates an explicitly requested retained-content value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the exact retained value to an authorized caller.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes the wrapper and returns the exact retained value.
    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl fmt::Debug for RetainedContent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("RetainedContent")
            .field(&REDACTED)
            .finish()
    }
}

/// Provenance of a projected value.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceSource {
    /// Evidence read from the local SQLite state database.
    LocalState,
    /// The latest remote value recorded by a prior local fetch or point check.
    LastRecordedRemoteFetch,
}

/// A bounded page request. `after` is an opaque continuation produced by the
/// previous page; it is never interpreted as SQL.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct PageRequest {
    /// Requested number of rows.
    pub limit: usize,
    /// Opaque continuation from a prior page.
    pub after: Option<String>,
}

impl Default for PageRequest {
    fn default() -> Self {
        Self {
            limit: DEFAULT_PAGE_SIZE,
            after: None,
        }
    }
}

impl PageRequest {
    /// Creates a request with an explicit row bound.
    #[must_use]
    pub const fn new(limit: usize) -> Self {
        Self { limit, after: None }
    }

    /// Changes the requested row bound.
    #[must_use]
    pub const fn with_limit(mut self, limit: usize) -> Self {
        self.limit = limit;
        self
    }

    /// Compatibility alias for [`Self::with_limit`].
    #[must_use]
    pub const fn with_page_size(self, page_size: usize) -> Self {
        self.with_limit(page_size)
    }

    /// Sets an opaque continuation returned by a prior page.
    #[must_use]
    pub fn with_after(mut self, after: impl Into<String>) -> Self {
        self.after = Some(after.into());
        self
    }

    /// Compatibility alias for [`Self::with_after`].
    #[must_use]
    pub fn with_cursor(self, cursor: impl Into<String>) -> Self {
        self.with_after(cursor)
    }

    /// Validates the bound and continuation without querying state.
    pub fn validate(&self) -> LifecycleResult<()> {
        if self.limit == 0 || self.limit > MAX_PAGE_SIZE {
            return Err(LifecycleError::InvalidPage { field: "limit" });
        }
        if let Some(after) = &self.after
            && (after.is_empty()
                || after.len() > MAX_CURSOR_LENGTH
                || after.chars().any(char::is_control))
        {
            return Err(LifecycleError::InvalidPage { field: "after" });
        }
        Ok(())
    }

    fn fetch_limit(&self) -> LifecycleResult<i64> {
        self.validate()?;
        self.limit
            .checked_add(1)
            .and_then(|value| i64::try_from(value).ok())
            .ok_or(LifecycleError::InvalidPage { field: "limit" })
    }
}

impl fmt::Debug for PageRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PageRequest")
            .field("limit", &self.limit)
            .field("has_cursor", &self.after.is_some())
            .finish()
    }
}

/// One bounded page of typed lifecycle projections.
#[derive(Clone, Eq, PartialEq, Serialize)]
pub struct LifecyclePage<T> {
    /// Repository scope echoed explicitly.
    pub repository_id: String,
    /// Stable object family represented by the page.
    pub object_type: String,
    /// Returned rows in deterministic order.
    pub items: Vec<T>,
    /// Effective page bound.
    pub page_size: usize,
    /// Whether more matching rows exist after this page.
    pub truncated: bool,
    /// Opaque continuation for the next page, if needed.
    pub next_after: Option<String>,
}

impl<T> LifecyclePage<T> {
    /// Returns whether the page is incomplete.
    #[must_use]
    pub const fn has_more(&self) -> bool {
        self.truncated
    }

    /// Returns the continuation token, if any.
    #[must_use]
    pub fn continuation(&self) -> Option<&str> {
        self.next_after.as_deref()
    }

    /// Returns the rows in this page.
    #[must_use]
    pub fn records(&self) -> &[T] {
        &self.items
    }

    /// Returns whether the page has no rows.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl<T> fmt::Debug for LifecyclePage<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LifecyclePage")
            .field("repository_bound", &true)
            .field("object_type", &self.object_type)
            .field("row_count", &self.items.len())
            .field("page_size", &self.page_size)
            .field("truncated", &self.truncated)
            .field("has_cursor", &self.next_after.is_some())
            .finish()
    }
}

/// Compatibility name for a bounded lifecycle page.
pub type Page<T> = LifecyclePage<T>;
/// Compatibility name emphasizing that the page has an explicit bound.
pub type BoundedPage<T> = LifecyclePage<T>;

/// Remote snapshot states are explicitly observations, not current truth.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteSnapshotState {
    /// A remote content snapshot was recorded and was not marked deleted.
    Present,
    /// A remote deletion marker was recorded.
    Deleted,
    /// No current snapshot was recorded.
    Unknown,
}

/// Read-receipt evidence is intentionally unavailable in this v1 projection.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadReceiptStatus {
    /// The product does not collect or infer read receipts.
    Unavailable,
}

/// A local reply claim is conservative and requires durable accepted evidence.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplyClaimStatus {
    /// No accepted reply evidence was found.
    NotClaimed,
    /// A matching local link and accepted delivery evidence were found.
    AcceptedEvidence,
    /// Evidence exists but could not be verified safely.
    Unverified,
}

/// Provenance and state of the latest remote value recorded locally.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RemoteSnapshotProjection {
    /// Timestamp of the recorded observation.
    pub observed_at: String,
    /// Recorded state at that observation.
    pub state: RemoteSnapshotState,
    /// Always identifies this as last-fetched evidence.
    pub provenance: EvidenceSource,
    /// Explicitly false: this crate never claims current remote truth.
    pub current_remote_truth: bool,
}

/// A first or current inbound snapshot. Text is opt-in and untrusted.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InboundSnapshotProjection {
    /// Observation timestamp.
    pub observed_at: String,
    /// Remote content when explicitly requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<RetainedContent>,
    /// Whether the recorded snapshot is a deletion marker.
    pub deleted: bool,
    /// Redacted attachment-indicator metadata.
    pub attachment_metadata: String,
    /// Provenance of the snapshot.
    pub provenance: EvidenceSource,
}

/// Local inbound markers. These are local facts, not remote effects.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LocalInboundMarkers {
    /// Local acknowledgement timestamp, if present.
    pub acknowledged_at: Option<String>,
    /// Local archive timestamp, if present.
    pub archived_at: Option<String>,
    /// Local reply link, if present.
    pub reply_link: Option<ReplyLinkProjection>,
}

/// An inbound item projection with local and last-fetched evidence separated.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InboundItemProjection {
    /// Repository scope.
    pub repository_id: String,
    /// Remote item identifier.
    pub item_id: String,
    /// Channel identifier observed locally.
    pub channel_id: String,
    /// Human author identifier observed locally.
    pub author_id: String,
    /// Immutable first snapshot.
    pub first_snapshot: InboundSnapshotProjection,
    /// Latest current snapshot recorded by a prior fetch or point check.
    pub current_snapshot: InboundSnapshotProjection,
    /// Latest remote value recorded by a prior fetch or point check.
    pub last_recorded_remote: RemoteSnapshotProjection,
    /// Local acknowledgement/archive/link state.
    pub local_state: LocalInboundMarkers,
    /// Always true: inbound content is untrusted data.
    pub untrusted: bool,
}

impl InboundItemProjection {
    /// Returns whether the projection contains an explicitly requested value.
    #[must_use]
    pub fn retained_content_requested(&self) -> bool {
        self.first_snapshot.content.is_some() || self.current_snapshot.content.is_some()
    }

    /// Returns whether a local reply has durable accepted evidence.
    #[must_use]
    pub fn replied(&self) -> bool {
        self.local_state
            .reply_link
            .as_ref()
            .is_some_and(|link| link.replied)
    }

    /// Returns whether this projection claims current remote truth.
    #[must_use]
    pub const fn claims_current_remote_truth(&self) -> bool {
        false
    }
}

/// A repository identity projection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct RepositoryProjection {
    /// Repository scope.
    pub repository_id: String,
    /// Configured workspace identifier.
    pub workspace_id: String,
    /// Canonical configuration hash.
    pub config_hash: String,
    /// Creation timestamp.
    pub created_at: String,
    /// Last local update timestamp.
    pub updated_at: String,
    /// Local-state provenance.
    pub provenance: EvidenceSource,
}

/// A draft parent projection. It contains no revision text.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DraftProjection {
    /// Repository scope.
    pub repository_id: String,
    /// Draft identifier.
    pub draft_id: String,
    /// Event type.
    pub event_type: String,
    /// Destination alias.
    pub destination_alias: String,
    /// Current local lifecycle state.
    pub status: String,
    /// Current immutable revision number.
    pub current_revision: i64,
    /// Optional expiry.
    pub expiry_at: Option<String>,
    /// Optional inbound reply target.
    pub reply_to_inbound_item_id: Option<String>,
    /// Redacted local metadata.
    pub metadata_json: String,
    /// Creation timestamp.
    pub created_at: String,
    /// Last local update timestamp.
    pub updated_at: String,
    /// Local-state provenance.
    pub provenance: EvidenceSource,
}

/// An immutable draft revision projection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DraftRevisionProjection {
    /// Repository scope.
    pub repository_id: String,
    /// Draft identifier.
    pub draft_id: String,
    /// Immutable revision number.
    pub revision: i64,
    /// Stored content hash.
    pub content_hash: String,
    /// Exact body only after explicit retained-content opt-in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<RetainedContent>,
    /// Redacted revision metadata.
    pub metadata_json: String,
    /// Destination alias snapshot.
    pub destination_alias: String,
    /// Resolved destination snapshot, redacted for diagnostics.
    pub resolved_destination: String,
    /// Optional expiry.
    pub expiry_at: Option<String>,
    /// Local lifecycle state at the revision.
    pub lifecycle_state: String,
    /// Optional inbound reply target.
    pub reply_to_inbound_item_id: Option<String>,
    /// Revision creation timestamp.
    pub created_at: String,
    /// Local-state provenance.
    pub provenance: EvidenceSource,
    /// Remote state is not inspected by this projection.
    pub remote_fetch_performed: bool,
}

impl DraftRevisionProjection {
    /// Returns whether the exact retained body was explicitly requested.
    #[must_use]
    pub const fn retained_content_requested(&self) -> bool {
        self.body.is_some()
    }
}

/// Last-recorded remote delivery evidence, not a read receipt.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct LastRecordedDeliveryEvidence {
    /// Remote message identifier returned by the last recorded attempt.
    pub message_id: String,
    /// Completion timestamp, if the attempt completed.
    pub observed_at: Option<String>,
    /// Provenance is always a prior local record.
    pub provenance: EvidenceSource,
}

/// A local delivery-attempt projection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DeliveryAttemptProjection {
    /// Repository scope.
    pub repository_id: String,
    /// Attempt identifier.
    pub attempt_id: String,
    /// Draft identifier.
    pub draft_id: String,
    /// Exact draft revision.
    pub revision: i64,
    /// Attempt number.
    pub attempt_number: i64,
    /// Claim nonce is local non-content metadata.
    pub claim_nonce: String,
    /// Durable local delivery state.
    pub state: String,
    /// Claim timestamp.
    pub claimed_at: String,
    /// Completion timestamp.
    pub completed_at: Option<String>,
    /// Last-recorded remote message ID, if the attempt recorded one.
    pub remote_message_id: Option<String>,
    /// Last-recorded remote evidence, explicitly separate from local state.
    pub last_recorded_remote: Option<LastRecordedDeliveryEvidence>,
    /// Local safe failure code, if any.
    pub failure_code: Option<String>,
    /// The product never infers a read receipt.
    pub read_receipt: ReadReceiptStatus,
    /// An accepted outbound attempt alone never establishes a reply.
    pub reply_claim: ReplyClaimStatus,
    /// Local-state provenance.
    pub provenance: EvidenceSource,
}

impl DeliveryAttemptProjection {
    /// Returns whether the local attempt is an accepted state.
    #[must_use]
    pub fn is_accepted(&self) -> bool {
        matches!(self.state.as_str(), "accepted" | "reconciled_accepted")
    }

    /// Explicitly returns false; accepted delivery is not a read receipt.
    #[must_use]
    pub const fn read_receipt_claimed(&self) -> bool {
        false
    }

    /// Explicitly returns false; accepted delivery is not a conversation reply.
    #[must_use]
    pub const fn replied_claimed(&self) -> bool {
        false
    }
}

/// A local acknowledgement projection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AcknowledgementProjection {
    /// Repository scope.
    pub repository_id: String,
    /// Inbound item identifier.
    pub item_id: String,
    /// Local acknowledgement timestamp.
    pub acknowledged_at: String,
    /// Acknowledgement is local-only.
    pub local_only: bool,
    /// No remote effect is claimed.
    pub remote_effect: Option<String>,
    /// Local-state provenance.
    pub provenance: EvidenceSource,
}

/// A local archive projection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ArchiveProjection {
    /// Repository scope.
    pub repository_id: String,
    /// Inbound item identifier.
    pub item_id: String,
    /// Local archive timestamp.
    pub archived_at: String,
    /// Archival is local-only.
    pub local_only: bool,
    /// No remote effect is claimed.
    pub remote_effect: Option<String>,
    /// Local-state provenance.
    pub provenance: EvidenceSource,
}

/// A local reply-link projection.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ReplyLinkProjection {
    /// Repository scope.
    pub repository_id: String,
    /// Inbound item identifier.
    pub item_id: String,
    /// Linked local reply draft identifier.
    pub reply_draft_id: String,
    /// Link timestamp.
    pub linked_at: String,
    /// Whether durable accepted-delivery evidence was verified.
    pub replied: bool,
    /// Accepted delivery ID, only when verified.
    pub accepted_delivery_id: Option<String>,
    /// Remote message ID recorded by the accepted delivery, only when verified.
    pub accepted_remote_message_id: Option<String>,
    /// Local audit event ID, only when verified.
    pub audit_event_id: Option<String>,
    /// Conservative evidence status.
    pub evidence_status: ReplyClaimStatus,
    /// Linking is local-only until a separate accepted delivery is recorded.
    pub local_only_link: bool,
    /// No read receipt is inferred from this link.
    pub read_receipt: ReadReceiptStatus,
    /// Local-state provenance.
    pub provenance: EvidenceSource,
}

impl ReplyLinkProjection {
    fn linked_only(record: &repo_com_state::ReplyLinkRecord) -> Self {
        Self {
            repository_id: safe_text(record.repository_id.clone()),
            item_id: safe_text(record.item_id.clone()),
            reply_draft_id: safe_text(record.reply_draft_id.clone()),
            linked_at: safe_text(record.linked_at.clone()),
            replied: false,
            accepted_delivery_id: None,
            accepted_remote_message_id: None,
            audit_event_id: None,
            evidence_status: ReplyClaimStatus::NotClaimed,
            local_only_link: true,
            read_receipt: ReadReceiptStatus::Unavailable,
            provenance: EvidenceSource::LocalState,
        }
    }
}

/// A bounded page of local audit transitions.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct AuditTransitionsProjection {
    /// The repository-scoped page returned by the audit query owner.
    pub page: AuditPage,
    /// Audit evidence is local, never a remote fetch.
    pub provenance: EvidenceSource,
    /// Explicitly false for this read-only projection.
    pub remote_fetch_performed: bool,
}

impl std::ops::Deref for AuditTransitionsProjection {
    type Target = AuditPage;

    fn deref(&self) -> &Self::Target {
        &self.page
    }
}

/// Object selector for the unified inspection boundary.
#[derive(Clone, Eq, PartialEq)]
pub enum LifecycleObject {
    /// Repository identity.
    Repository,
    /// Draft parent row.
    Draft {
        /// Draft identifier.
        draft_id: String,
    },
    /// One immutable draft revision.
    DraftRevision {
        /// Draft identifier.
        draft_id: String,
        /// Revision number.
        revision: i64,
    },
    /// One delivery attempt.
    DeliveryAttempt {
        /// Attempt identifier.
        attempt_id: String,
    },
    /// One inbound item.
    InboundItem {
        /// Inbound item identifier.
        item_id: String,
    },
    /// One local acknowledgement marker.
    Acknowledgement {
        /// Inbound item identifier.
        item_id: String,
    },
    /// One local archive marker.
    Archive {
        /// Inbound item identifier.
        item_id: String,
    },
    /// One local reply link.
    ReplyLink {
        /// Inbound item identifier.
        item_id: String,
    },
    /// Bounded local audit transitions.
    AuditTransitions {
        /// Caller-supplied repository-scoped audit filter.
        filter: AuditFilter,
    },
}

impl LifecycleObject {
    /// Returns the stable object-family label.
    #[must_use]
    pub const fn object_type(&self) -> &'static str {
        match self {
            Self::Repository => REPOSITORY_OBJECT,
            Self::Draft { .. } => DRAFT_OBJECT,
            Self::DraftRevision { .. } => DRAFT_REVISION_OBJECT,
            Self::DeliveryAttempt { .. } => DELIVERY_ATTEMPT_OBJECT,
            Self::InboundItem { .. } => INBOUND_ITEM_OBJECT,
            Self::Acknowledgement { .. } => ACKNOWLEDGEMENT_OBJECT,
            Self::Archive { .. } => ARCHIVE_OBJECT,
            Self::ReplyLink { .. } => REPLY_LINK_OBJECT,
            Self::AuditTransitions { .. } => AUDIT_OBJECT,
        }
    }
}

impl fmt::Debug for LifecycleObject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Repository => formatter.write_str("Repository"),
            Self::Draft { draft_id } => formatter
                .debug_struct("Draft")
                .field("draft_id", &redact_text(draft_id))
                .finish(),
            Self::DraftRevision { draft_id, revision } => formatter
                .debug_struct("DraftRevision")
                .field("draft_id", &redact_text(draft_id))
                .field("revision", revision)
                .finish(),
            Self::DeliveryAttempt { attempt_id } => formatter
                .debug_struct("DeliveryAttempt")
                .field("attempt_id", &redact_text(attempt_id))
                .finish(),
            Self::InboundItem { item_id } => formatter
                .debug_struct("InboundItem")
                .field("item_id", &redact_text(item_id))
                .finish(),
            Self::Acknowledgement { item_id } => formatter
                .debug_struct("Acknowledgement")
                .field("item_id", &redact_text(item_id))
                .finish(),
            Self::Archive { item_id } => formatter
                .debug_struct("Archive")
                .field("item_id", &redact_text(item_id))
                .finish(),
            Self::ReplyLink { item_id } => formatter
                .debug_struct("ReplyLink")
                .field("item_id", &redact_text(item_id))
                .finish(),
            Self::AuditTransitions { .. } => formatter.write_str("AuditTransitions"),
        }
    }
}

/// A unified, read-only inspection request.
#[derive(Clone, Eq, PartialEq)]
pub struct InspectionRequest {
    /// Repository scope.
    pub repository_id: String,
    /// Object selector.
    pub object: LifecycleObject,
    /// Bounded page request for list-like operations.
    pub page: PageRequest,
    /// Explicit opt-in for retained message or draft text.
    pub include_retained_content: bool,
}

impl InspectionRequest {
    /// Creates a request with the default bounded page and no content opt-in.
    #[must_use]
    pub fn new(repository_id: impl Into<String>, object: LifecycleObject) -> Self {
        Self {
            repository_id: repository_id.into(),
            object,
            page: PageRequest::default(),
            include_retained_content: false,
        }
    }

    /// Creates a repository request.
    #[must_use]
    pub fn repository(repository_id: impl Into<String>) -> Self {
        Self::new(repository_id, LifecycleObject::Repository)
    }

    /// Creates a draft request.
    #[must_use]
    pub fn draft(repository_id: impl Into<String>, draft_id: impl Into<String>) -> Self {
        Self::new(
            repository_id,
            LifecycleObject::Draft {
                draft_id: draft_id.into(),
            },
        )
    }

    /// Creates an immutable revision request.
    #[must_use]
    pub fn draft_revision(
        repository_id: impl Into<String>,
        draft_id: impl Into<String>,
        revision: i64,
    ) -> Self {
        Self::new(
            repository_id,
            LifecycleObject::DraftRevision {
                draft_id: draft_id.into(),
                revision,
            },
        )
    }

    /// Creates a delivery-attempt request.
    #[must_use]
    pub fn delivery_attempt(
        repository_id: impl Into<String>,
        attempt_id: impl Into<String>,
    ) -> Self {
        Self::new(
            repository_id,
            LifecycleObject::DeliveryAttempt {
                attempt_id: attempt_id.into(),
            },
        )
    }

    /// Creates an inbound-item request.
    #[must_use]
    pub fn inbound_item(repository_id: impl Into<String>, item_id: impl Into<String>) -> Self {
        Self::new(
            repository_id,
            LifecycleObject::InboundItem {
                item_id: item_id.into(),
            },
        )
    }

    /// Creates an acknowledgement request.
    #[must_use]
    pub fn acknowledgement(repository_id: impl Into<String>, item_id: impl Into<String>) -> Self {
        Self::new(
            repository_id,
            LifecycleObject::Acknowledgement {
                item_id: item_id.into(),
            },
        )
    }

    /// Creates an archive request.
    #[must_use]
    pub fn archive(repository_id: impl Into<String>, item_id: impl Into<String>) -> Self {
        Self::new(
            repository_id,
            LifecycleObject::Archive {
                item_id: item_id.into(),
            },
        )
    }

    /// Creates a reply-link request.
    #[must_use]
    pub fn reply_link(repository_id: impl Into<String>, item_id: impl Into<String>) -> Self {
        Self::new(
            repository_id,
            LifecycleObject::ReplyLink {
                item_id: item_id.into(),
            },
        )
    }

    /// Creates an audit-transition request with a repository-scoped filter.
    #[must_use]
    pub fn audit_transitions(filter: AuditFilter) -> Self {
        let repository_id = filter.repository_id.clone();
        Self::new(repository_id, LifecycleObject::AuditTransitions { filter })
    }

    /// Sets the bounded page request.
    #[must_use]
    pub fn with_page(mut self, page: PageRequest) -> Self {
        if let LifecycleObject::AuditTransitions { filter } = &mut self.object {
            filter.page_size = page.limit;
        }
        self.page = page;
        self
    }

    /// Compatibility alias for [`Self::with_page`].
    #[must_use]
    pub fn with_page_size(self, page_size: usize) -> Self {
        self.with_page(PageRequest::new(page_size))
    }

    /// Explicitly opts in to retained content for this request.
    #[must_use]
    pub const fn with_retained_content(mut self, include: bool) -> Self {
        self.include_retained_content = include;
        self
    }

    /// Compatibility alias for [`Self::with_retained_content`].
    #[must_use]
    pub const fn include_content(self, include: bool) -> Self {
        self.with_retained_content(include)
    }

    /// Sets a typed audit continuation after constructing an audit request.
    #[must_use]
    pub fn with_audit_cursor(mut self, cursor: AuditCursor) -> Self {
        if let LifecycleObject::AuditTransitions { filter } = &mut self.object {
            filter.cursor = Some(cursor);
        }
        self
    }
}

impl fmt::Debug for InspectionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InspectionRequest")
            .field("repository_bound", &true)
            .field("object", &self.object)
            .field("page", &self.page)
            .field("include_retained_content", &self.include_retained_content)
            .finish()
    }
}

/// A typed lifecycle projection.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum LifecycleProjection {
    /// Repository identity.
    Repository(RepositoryProjection),
    /// Draft parent.
    Draft(DraftProjection),
    /// Immutable draft revision.
    DraftRevision(DraftRevisionProjection),
    /// Delivery attempt.
    DeliveryAttempt(DeliveryAttemptProjection),
    /// Inbound item.
    InboundItem(InboundItemProjection),
    /// Local acknowledgement.
    Acknowledgement(AcknowledgementProjection),
    /// Local archive.
    Archive(ArchiveProjection),
    /// Local reply link.
    ReplyLink(ReplyLinkProjection),
    /// Local audit transitions.
    AuditTransitions(AuditTransitionsProjection),
}

impl LifecycleProjection {
    /// Returns the stable object-family label.
    #[must_use]
    pub const fn object_type(&self) -> &'static str {
        match self {
            Self::Repository(_) => REPOSITORY_OBJECT,
            Self::Draft(_) => DRAFT_OBJECT,
            Self::DraftRevision(_) => DRAFT_REVISION_OBJECT,
            Self::DeliveryAttempt(_) => DELIVERY_ATTEMPT_OBJECT,
            Self::InboundItem(_) => INBOUND_ITEM_OBJECT,
            Self::Acknowledgement(_) => ACKNOWLEDGEMENT_OBJECT,
            Self::Archive(_) => ARCHIVE_OBJECT,
            Self::ReplyLink(_) => REPLY_LINK_OBJECT,
            Self::AuditTransitions(_) => AUDIT_OBJECT,
        }
    }

    /// Returns whether the projection carries explicitly requested retained text.
    #[must_use]
    pub const fn retained_content_requested(&self) -> bool {
        match self {
            Self::DraftRevision(value) => value.body.is_some(),
            Self::InboundItem(value) => {
                value.first_snapshot.content.is_some() || value.current_snapshot.content.is_some()
            }
            _ => false,
        }
    }
}

/// Read-only lifecycle inspection over an already opened state store.
pub struct LifecycleInspector<'store> {
    state: &'store StateStore,
}

impl<'store> LifecycleInspector<'store> {
    /// Creates an inspector borrowing local state immutably.
    #[must_use]
    pub const fn new(state: &'store StateStore) -> Self {
        Self { state }
    }

    /// Compatibility alias for [`Self::new`].
    #[must_use]
    pub const fn from_state(state: &'store StateStore) -> Self {
        Self::new(state)
    }

    /// Executes one bounded, read-only object inspection.
    pub fn inspect(&self, request: &InspectionRequest) -> LifecycleResult<LifecycleProjection> {
        request.page.validate()?;
        let repository_id = validate_repository(&request.repository_id)?;
        ensure_repository(self.state.connection(), &repository_id)?;
        match &request.object {
            LifecycleObject::Repository => self
                .inspect_repository(&repository_id)
                .map(LifecycleProjection::Repository),
            LifecycleObject::Draft { draft_id } => self
                .inspect_draft(&repository_id, draft_id)
                .map(LifecycleProjection::Draft),
            LifecycleObject::DraftRevision { draft_id, revision } => self
                .inspect_draft_revision(
                    &repository_id,
                    draft_id,
                    *revision,
                    request.include_retained_content,
                )
                .map(LifecycleProjection::DraftRevision),
            LifecycleObject::DeliveryAttempt { attempt_id } => self
                .inspect_delivery_attempt(&repository_id, attempt_id)
                .map(LifecycleProjection::DeliveryAttempt),
            LifecycleObject::InboundItem { item_id } => self
                .inspect_inbound_item(&repository_id, item_id, request.include_retained_content)
                .map(LifecycleProjection::InboundItem),
            LifecycleObject::Acknowledgement { item_id } => self
                .inspect_acknowledgement(&repository_id, item_id)
                .map(LifecycleProjection::Acknowledgement),
            LifecycleObject::Archive { item_id } => self
                .inspect_archive(&repository_id, item_id)
                .map(LifecycleProjection::Archive),
            LifecycleObject::ReplyLink { item_id } => self
                .inspect_reply_link(&repository_id, item_id)
                .map(LifecycleProjection::ReplyLink),
            LifecycleObject::AuditTransitions { filter } => {
                if filter.repository_id != repository_id {
                    return Err(LifecycleError::CrossRepositoryDenied {
                        repository_id: repository_id.clone(),
                    });
                }
                if filter.page_size > AUDIT_MAX_PAGE_SIZE || filter.page_size == 0 {
                    return Err(LifecycleError::InvalidPage { field: "limit" });
                }
                self.inspect_audit_transitions(filter)
                    .map(LifecycleProjection::AuditTransitions)
            }
        }
    }

    /// Reads one repository identity.
    pub fn inspect_repository(
        &self,
        repository_id: impl AsRef<str>,
    ) -> LifecycleResult<RepositoryProjection> {
        let repository_id = validate_repository(repository_id.as_ref())?;
        ensure_repository(self.state.connection(), &repository_id)?;
        self.state
            .connection()
            .query_row(
                "SELECT repository_id, workspace_id, config_hash, created_at, updated_at
                 FROM repositories WHERE repository_id = ?1",
                [&repository_id],
                |row| {
                    Ok(RepositoryProjection {
                        repository_id: safe_text(row.get(0)?),
                        workspace_id: safe_text(row.get(1)?),
                        config_hash: safe_text(row.get(2)?),
                        created_at: safe_text(row.get(3)?),
                        updated_at: safe_text(row.get(4)?),
                        provenance: EvidenceSource::LocalState,
                    })
                },
            )
            .optional()?
            .ok_or(LifecycleError::RepositoryNotFound {
                repository_id: repository_id.clone(),
            })
    }

    /// Compatibility alias for [`Self::inspect_repository`].
    pub fn repository(
        &self,
        repository_id: impl AsRef<str>,
    ) -> LifecycleResult<RepositoryProjection> {
        self.inspect_repository(repository_id)
    }

    /// Reads one draft parent without its content.
    pub fn inspect_draft(
        &self,
        repository_id: impl AsRef<str>,
        draft_id: impl AsRef<str>,
    ) -> LifecycleResult<DraftProjection> {
        let repository_id = validate_repository(repository_id.as_ref())?;
        let draft_id = validate_object_id(DRAFT_OBJECT, draft_id.as_ref())?;
        ensure_repository(self.state.connection(), &repository_id)?;
        self.state
            .connection()
            .query_row(
                "SELECT repository_id, draft_id, event_type, destination_alias, status,
                        current_revision, expiry_at, reply_to_inbound_item_id, metadata_json,
                        created_at, updated_at
                 FROM drafts WHERE repository_id = ?1 AND draft_id = ?2",
                params![repository_id, draft_id],
                row_to_draft,
            )
            .optional()?
            .ok_or(LifecycleError::ObjectNotFound {
                object_type: DRAFT_OBJECT,
                repository_id,
                object_id: draft_id,
            })
    }

    /// Compatibility alias for [`Self::inspect_draft`].
    pub fn draft(
        &self,
        repository_id: impl AsRef<str>,
        draft_id: impl AsRef<str>,
    ) -> LifecycleResult<DraftProjection> {
        self.inspect_draft(repository_id, draft_id)
    }

    /// Reads one immutable revision, optionally returning its retained body.
    pub fn inspect_draft_revision(
        &self,
        repository_id: impl AsRef<str>,
        draft_id: impl AsRef<str>,
        revision: i64,
        include_retained_content: bool,
    ) -> LifecycleResult<DraftRevisionProjection> {
        let repository_id = validate_repository(repository_id.as_ref())?;
        let draft_id = validate_object_id(DRAFT_REVISION_OBJECT, draft_id.as_ref())?;
        if revision <= 0 {
            return Err(LifecycleError::InvalidObject {
                object_type: DRAFT_REVISION_OBJECT,
            });
        }
        ensure_repository(self.state.connection(), &repository_id)?;
        self.state
            .connection()
            .query_row(
                "SELECT repository_id, draft_id, revision, content_hash,
                        CASE WHEN ?3 THEN body ELSE NULL END,
                        metadata_json, destination_alias, resolved_destination, expiry_at,
                        lifecycle_state, reply_to_inbound_item_id, created_at
                 FROM draft_revisions
                 WHERE repository_id = ?1 AND draft_id = ?2 AND revision = ?4",
                params![repository_id, draft_id, include_retained_content, revision],
                row_to_draft_revision,
            )
            .optional()?
            .ok_or(LifecycleError::ObjectNotFound {
                object_type: DRAFT_REVISION_OBJECT,
                repository_id,
                object_id: format!("{draft_id}@{revision}"),
            })
    }

    /// Compatibility alias for [`Self::inspect_draft_revision`].
    pub fn draft_revision(
        &self,
        repository_id: impl AsRef<str>,
        draft_id: impl AsRef<str>,
        revision: i64,
        include_retained_content: bool,
    ) -> LifecycleResult<DraftRevisionProjection> {
        self.inspect_draft_revision(repository_id, draft_id, revision, include_retained_content)
    }

    /// Reads one delivery attempt without inferring a read receipt or reply.
    pub fn inspect_delivery_attempt(
        &self,
        repository_id: impl AsRef<str>,
        attempt_id: impl AsRef<str>,
    ) -> LifecycleResult<DeliveryAttemptProjection> {
        let repository_id = validate_repository(repository_id.as_ref())?;
        let attempt_id = validate_object_id(DELIVERY_ATTEMPT_OBJECT, attempt_id.as_ref())?;
        ensure_repository(self.state.connection(), &repository_id)?;
        self.state
            .connection()
            .query_row(
                "SELECT repository_id, attempt_id, draft_id, revision, attempt_number,
                        claim_nonce, state, claimed_at, completed_at, remote_message_id,
                        failure_code
                 FROM delivery_attempts WHERE repository_id = ?1 AND attempt_id = ?2",
                params![repository_id, attempt_id],
                row_to_delivery,
            )
            .optional()?
            .ok_or(LifecycleError::ObjectNotFound {
                object_type: DELIVERY_ATTEMPT_OBJECT,
                repository_id,
                object_id: attempt_id,
            })
    }

    /// Compatibility alias for [`Self::inspect_delivery_attempt`].
    pub fn delivery_attempt(
        &self,
        repository_id: impl AsRef<str>,
        attempt_id: impl AsRef<str>,
    ) -> LifecycleResult<DeliveryAttemptProjection> {
        self.inspect_delivery_attempt(repository_id, attempt_id)
    }

    /// Reads one inbound item with separate local and last-fetched evidence.
    pub fn inspect_inbound_item(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
        include_retained_content: bool,
    ) -> LifecycleResult<InboundItemProjection> {
        let repository_id = validate_repository(repository_id.as_ref())?;
        let item_id = validate_object_id(INBOUND_ITEM_OBJECT, item_id.as_ref())?;
        ensure_repository(self.state.connection(), &repository_id)?;
        let mut projection = self
            .state
            .connection()
            .query_row(
                "SELECT i.repository_id, i.item_id, i.channel_id, i.author_id,
                        CASE WHEN ?3 THEN i.first_content ELSE NULL END,
                        i.first_attachments_json, i.first_observed_at,
                        CASE WHEN ?3 THEN c.current_content ELSE NULL END,
                        c.current_attachments_json, c.deleted, c.observed_at,
                        a.acknowledged_at, ar.archived_at, l.reply_draft_id, l.linked_at,
                        c.current_content IS NOT NULL AS current_content_present,
                        c.item_id IS NOT NULL AS current_snapshot_present
                 FROM inbound_items AS i
                 LEFT JOIN inbound_current_snapshots AS c
                   ON c.repository_id = i.repository_id AND c.item_id = i.item_id
                 LEFT JOIN inbound_acknowledgements AS a
                   ON a.repository_id = i.repository_id AND a.item_id = i.item_id
                 LEFT JOIN inbound_archives AS ar
                   ON ar.repository_id = i.repository_id AND ar.item_id = i.item_id
                 LEFT JOIN inbound_reply_links AS l
                   ON l.repository_id = i.repository_id AND l.item_id = i.item_id
                 WHERE i.repository_id = ?1 AND i.item_id = ?2",
                params![repository_id, item_id, include_retained_content],
                |row| row_to_inbound(row, include_retained_content),
            )
            .optional()?
            .ok_or(LifecycleError::ObjectNotFound {
                object_type: INBOUND_ITEM_OBJECT,
                repository_id: repository_id.clone(),
                object_id: item_id.clone(),
            })?;
        self.attach_reply_evidence(&mut projection);
        Ok(projection)
    }

    /// Compatibility alias for [`Self::inspect_inbound_item`].
    pub fn inbound_item(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
        include_retained_content: bool,
    ) -> LifecycleResult<InboundItemProjection> {
        self.inspect_inbound_item(repository_id, item_id, include_retained_content)
    }

    /// Reads one local acknowledgement marker.
    pub fn inspect_acknowledgement(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> LifecycleResult<AcknowledgementProjection> {
        let repository_id = validate_repository(repository_id.as_ref())?;
        let item_id = validate_object_id(ACKNOWLEDGEMENT_OBJECT, item_id.as_ref())?;
        ensure_repository(self.state.connection(), &repository_id)?;
        self.state
            .connection()
            .query_row(
                "SELECT repository_id, item_id, acknowledged_at
                 FROM inbound_acknowledgements WHERE repository_id = ?1 AND item_id = ?2",
                params![repository_id, item_id],
                row_to_acknowledgement,
            )
            .optional()?
            .ok_or(LifecycleError::ObjectNotFound {
                object_type: ACKNOWLEDGEMENT_OBJECT,
                repository_id,
                object_id: item_id,
            })
    }

    /// Compatibility alias for [`Self::inspect_acknowledgement`].
    pub fn acknowledgement(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> LifecycleResult<AcknowledgementProjection> {
        self.inspect_acknowledgement(repository_id, item_id)
    }

    /// Reads one local archive marker.
    pub fn inspect_archive(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> LifecycleResult<ArchiveProjection> {
        let repository_id = validate_repository(repository_id.as_ref())?;
        let item_id = validate_object_id(ARCHIVE_OBJECT, item_id.as_ref())?;
        ensure_repository(self.state.connection(), &repository_id)?;
        self.state
            .connection()
            .query_row(
                "SELECT repository_id, item_id, archived_at
                 FROM inbound_archives WHERE repository_id = ?1 AND item_id = ?2",
                params![repository_id, item_id],
                row_to_archive,
            )
            .optional()?
            .ok_or(LifecycleError::ObjectNotFound {
                object_type: ARCHIVE_OBJECT,
                repository_id,
                object_id: item_id,
            })
    }

    /// Compatibility alias for [`Self::inspect_archive`].
    pub fn archive(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> LifecycleResult<ArchiveProjection> {
        self.inspect_archive(repository_id, item_id)
    }

    /// Reads one local reply link and verifies accepted evidence conservatively.
    pub fn inspect_reply_link(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> LifecycleResult<ReplyLinkProjection> {
        let repository_id = validate_repository(repository_id.as_ref())?;
        let item_id = validate_object_id(REPLY_LINK_OBJECT, item_id.as_ref())?;
        ensure_repository(self.state.connection(), &repository_id)?;
        let record = self
            .state
            .connection()
            .query_row(
                "SELECT repository_id, item_id, reply_draft_id, linked_at
                 FROM inbound_reply_links WHERE repository_id = ?1 AND item_id = ?2",
                params![repository_id, item_id],
                |row| {
                    Ok(repo_com_state::ReplyLinkRecord {
                        repository_id: row.get(0)?,
                        item_id: row.get(1)?,
                        reply_draft_id: row.get(2)?,
                        linked_at: row.get(3)?,
                    })
                },
            )
            .optional()?
            .ok_or(LifecycleError::ObjectNotFound {
                object_type: REPLY_LINK_OBJECT,
                repository_id,
                object_id: item_id,
            })?;
        Ok(self.reply_projection(&record))
    }

    /// Compatibility alias for [`Self::inspect_reply_link`].
    pub fn reply_link(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> LifecycleResult<ReplyLinkProjection> {
        self.inspect_reply_link(repository_id, item_id)
    }

    /// Executes the bounded local audit query owned by REPO-AUDIT-2.
    pub fn inspect_audit_transitions(
        &self,
        filter: &AuditFilter,
    ) -> LifecycleResult<AuditTransitionsProjection> {
        if filter.page_size == 0 || filter.page_size > AUDIT_MAX_PAGE_SIZE {
            return Err(LifecycleError::InvalidPage { field: "limit" });
        }
        let repository_id = validate_repository(&filter.repository_id)?;
        if filter
            .cursor
            .as_ref()
            .is_some_and(|cursor| cursor.repository_id != repository_id)
        {
            return Err(LifecycleError::CrossRepositoryDenied {
                repository_id: repository_id.clone(),
            });
        }
        ensure_repository(self.state.connection(), &repository_id)?;
        let page = AuditQuery::new(self.state)
            .query(filter)
            .map_err(map_audit_error)?;
        Ok(AuditTransitionsProjection {
            page,
            provenance: EvidenceSource::LocalState,
            remote_fetch_performed: false,
        })
    }

    /// Compatibility alias for [`Self::inspect_audit_transitions`].
    pub fn audit_transitions(
        &self,
        filter: &AuditFilter,
    ) -> LifecycleResult<AuditTransitionsProjection> {
        self.inspect_audit_transitions(filter)
    }

    /// Lists bounded local audit transitions using the audit owner's filter.
    pub fn list_audit_transitions(
        &self,
        filter: &AuditFilter,
    ) -> LifecycleResult<AuditTransitionsProjection> {
        self.inspect_audit_transitions(filter)
    }

    /// Lists repository-scoped draft parents in stable ID order.
    pub fn list_drafts(
        &self,
        repository_id: impl AsRef<str>,
        page: &PageRequest,
    ) -> LifecycleResult<LifecyclePage<DraftProjection>> {
        let repository_id = validate_repository(repository_id.as_ref())?;
        ensure_repository(self.state.connection(), &repository_id)?;
        let fetch_limit = page.fetch_limit()?;
        let after = decode_after(&repository_id, page.after.as_deref())?;
        let mut statement = self.state.connection().prepare(
            "SELECT repository_id, draft_id, event_type, destination_alias, status,
                    current_revision, expiry_at, reply_to_inbound_item_id, metadata_json,
                    created_at, updated_at
             FROM drafts
             WHERE repository_id = :repository_id
               AND (:after IS NULL OR draft_id > :after)
             ORDER BY draft_id ASC
             LIMIT :fetch_limit",
        )?;
        let rows = statement.query_map(
            named_params! {
                ":repository_id": repository_id.as_str(),
                ":after": after.as_deref(),
                ":fetch_limit": fetch_limit,
            },
            row_to_draft,
        )?;
        let mut projected = Vec::new();
        for row in rows {
            let value = row?;
            projected.push((value.draft_id.clone(), value));
        }
        Ok(finish_page(
            repository_id,
            DRAFT_OBJECT,
            page.limit,
            projected,
        ))
    }

    /// Lists immutable revisions in stable draft/revision order.
    pub fn list_draft_revisions(
        &self,
        repository_id: impl AsRef<str>,
        draft_id: Option<&str>,
        page: &PageRequest,
        include_retained_content: bool,
    ) -> LifecycleResult<LifecyclePage<DraftRevisionProjection>> {
        let repository_id = validate_repository(repository_id.as_ref())?;
        let draft_id = draft_id
            .map(|value| validate_object_id(DRAFT_REVISION_OBJECT, value))
            .transpose()?;
        ensure_repository(self.state.connection(), &repository_id)?;
        let fetch_limit = page.fetch_limit()?;
        let after = decode_after(&repository_id, page.after.as_deref())?;
        let (after_draft, after_revision) = match after {
            Some(value) => split_revision_cursor(&value)?,
            None => (None, 0),
        };
        let mut statement = self.state.connection().prepare(
            "SELECT repository_id, draft_id, revision, content_hash,
                    CASE WHEN :include_content THEN body ELSE NULL END,
                    metadata_json, destination_alias, resolved_destination, expiry_at,
                    lifecycle_state, reply_to_inbound_item_id, created_at
             FROM draft_revisions
             WHERE repository_id = :repository_id
               AND (:draft_id IS NULL OR draft_id = :draft_id)
               AND (
                    :after_draft IS NULL
                    OR draft_id > :after_draft
                    OR (draft_id = :after_draft AND revision > :after_revision)
               )
             ORDER BY draft_id ASC, revision ASC
             LIMIT :fetch_limit",
        )?;
        let rows = statement.query_map(
            named_params! {
                ":repository_id": repository_id.as_str(),
                ":draft_id": draft_id.as_deref(),
                ":include_content": include_retained_content,
                ":after_draft": after_draft.as_deref(),
                ":after_revision": after_revision,
                ":fetch_limit": fetch_limit,
            },
            row_to_draft_revision,
        )?;
        let mut projected = Vec::new();
        for row in rows {
            let value = row?;
            projected.push((revision_key(&value.draft_id, value.revision), value));
        }
        Ok(finish_page(
            repository_id,
            DRAFT_REVISION_OBJECT,
            page.limit,
            projected,
        ))
    }

    /// Lists delivery attempts in stable attempt-ID order.
    pub fn list_delivery_attempts(
        &self,
        repository_id: impl AsRef<str>,
        page: &PageRequest,
    ) -> LifecycleResult<LifecyclePage<DeliveryAttemptProjection>> {
        let repository_id = validate_repository(repository_id.as_ref())?;
        ensure_repository(self.state.connection(), &repository_id)?;
        let fetch_limit = page.fetch_limit()?;
        let after = decode_after(&repository_id, page.after.as_deref())?;
        let mut statement = self.state.connection().prepare(
            "SELECT repository_id, attempt_id, draft_id, revision, attempt_number,
                    claim_nonce, state, claimed_at, completed_at, remote_message_id, failure_code
             FROM delivery_attempts
             WHERE repository_id = :repository_id
               AND (:after IS NULL OR attempt_id > :after)
             ORDER BY attempt_id ASC
             LIMIT :fetch_limit",
        )?;
        let rows = statement.query_map(
            named_params! {
                ":repository_id": repository_id.as_str(),
                ":after": after.as_deref(),
                ":fetch_limit": fetch_limit,
            },
            row_to_delivery,
        )?;
        let mut projected = Vec::new();
        for row in rows {
            let value = row?;
            projected.push((value.attempt_id.clone(), value));
        }
        Ok(finish_page(
            repository_id,
            DELIVERY_ATTEMPT_OBJECT,
            page.limit,
            projected,
        ))
    }

    /// Lists inbound items in stable remote-item-ID order.
    pub fn list_inbound_items(
        &self,
        repository_id: impl AsRef<str>,
        page: &PageRequest,
        include_retained_content: bool,
    ) -> LifecycleResult<LifecyclePage<InboundItemProjection>> {
        let repository_id = validate_repository(repository_id.as_ref())?;
        ensure_repository(self.state.connection(), &repository_id)?;
        let fetch_limit = page.fetch_limit()?;
        let after = decode_after(&repository_id, page.after.as_deref())?;
        let mut statement = self.state.connection().prepare(
            "SELECT i.repository_id, i.item_id, i.channel_id, i.author_id,
                    CASE WHEN :include_content THEN i.first_content ELSE NULL END,
                    i.first_attachments_json, i.first_observed_at,
                    CASE WHEN :include_content THEN c.current_content ELSE NULL END,
                    c.current_attachments_json, c.deleted, c.observed_at,
                    a.acknowledged_at, ar.archived_at, l.reply_draft_id, l.linked_at,
                    c.current_content IS NOT NULL AS current_content_present,
                    c.item_id IS NOT NULL AS current_snapshot_present
             FROM inbound_items AS i
             LEFT JOIN inbound_current_snapshots AS c
               ON c.repository_id = i.repository_id AND c.item_id = i.item_id
             LEFT JOIN inbound_acknowledgements AS a
               ON a.repository_id = i.repository_id AND a.item_id = i.item_id
             LEFT JOIN inbound_archives AS ar
               ON ar.repository_id = i.repository_id AND ar.item_id = i.item_id
             LEFT JOIN inbound_reply_links AS l
               ON l.repository_id = i.repository_id AND l.item_id = i.item_id
             WHERE i.repository_id = :repository_id
               AND (:after IS NULL OR i.item_id > :after)
             ORDER BY i.item_id ASC
             LIMIT :fetch_limit",
        )?;
        let rows = statement.query_map(
            named_params! {
                ":repository_id": repository_id.as_str(),
                ":include_content": include_retained_content,
                ":after": after.as_deref(),
                ":fetch_limit": fetch_limit,
            },
            |row| row_to_inbound(row, include_retained_content),
        )?;
        let mut projected = Vec::new();
        for row in rows {
            let mut value = row?;
            self.attach_reply_evidence(&mut value);
            projected.push((value.item_id.clone(), value));
        }
        Ok(finish_page(
            repository_id,
            INBOUND_ITEM_OBJECT,
            page.limit,
            projected,
        ))
    }

    /// Lists local acknowledgement markers in stable item-ID order.
    pub fn list_acknowledgements(
        &self,
        repository_id: impl AsRef<str>,
        page: &PageRequest,
    ) -> LifecycleResult<LifecyclePage<AcknowledgementProjection>> {
        let repository_id = validate_repository(repository_id.as_ref())?;
        ensure_repository(self.state.connection(), &repository_id)?;
        let fetch_limit = page.fetch_limit()?;
        let after = decode_after(&repository_id, page.after.as_deref())?;
        let mut statement = self.state.connection().prepare(
            "SELECT repository_id, item_id, acknowledged_at
             FROM inbound_acknowledgements
             WHERE repository_id = :repository_id
               AND (:after IS NULL OR item_id > :after)
             ORDER BY item_id ASC
             LIMIT :fetch_limit",
        )?;
        let rows = statement.query_map(
            named_params! {
                ":repository_id": repository_id.as_str(),
                ":after": after.as_deref(),
                ":fetch_limit": fetch_limit,
            },
            row_to_acknowledgement,
        )?;
        let mut projected = Vec::new();
        for row in rows {
            let value = row?;
            projected.push((value.item_id.clone(), value));
        }
        Ok(finish_page(
            repository_id,
            ACKNOWLEDGEMENT_OBJECT,
            page.limit,
            projected,
        ))
    }

    /// Lists local archive markers in stable item-ID order.
    pub fn list_archives(
        &self,
        repository_id: impl AsRef<str>,
        page: &PageRequest,
    ) -> LifecycleResult<LifecyclePage<ArchiveProjection>> {
        let repository_id = validate_repository(repository_id.as_ref())?;
        ensure_repository(self.state.connection(), &repository_id)?;
        let fetch_limit = page.fetch_limit()?;
        let after = decode_after(&repository_id, page.after.as_deref())?;
        let mut statement = self.state.connection().prepare(
            "SELECT repository_id, item_id, archived_at
             FROM inbound_archives
             WHERE repository_id = :repository_id
               AND (:after IS NULL OR item_id > :after)
             ORDER BY item_id ASC
             LIMIT :fetch_limit",
        )?;
        let rows = statement.query_map(
            named_params! {
                ":repository_id": repository_id.as_str(),
                ":after": after.as_deref(),
                ":fetch_limit": fetch_limit,
            },
            row_to_archive,
        )?;
        let mut projected = Vec::new();
        for row in rows {
            let value = row?;
            projected.push((value.item_id.clone(), value));
        }
        Ok(finish_page(
            repository_id,
            ARCHIVE_OBJECT,
            page.limit,
            projected,
        ))
    }

    /// Lists local reply links in stable item-ID order.
    pub fn list_reply_links(
        &self,
        repository_id: impl AsRef<str>,
        page: &PageRequest,
    ) -> LifecycleResult<LifecyclePage<ReplyLinkProjection>> {
        let repository_id = validate_repository(repository_id.as_ref())?;
        ensure_repository(self.state.connection(), &repository_id)?;
        let fetch_limit = page.fetch_limit()?;
        let after = decode_after(&repository_id, page.after.as_deref())?;
        let mut statement = self.state.connection().prepare(
            "SELECT repository_id, item_id, reply_draft_id, linked_at
             FROM inbound_reply_links
             WHERE repository_id = :repository_id
               AND (:after IS NULL OR item_id > :after)
             ORDER BY item_id ASC
             LIMIT :fetch_limit",
        )?;
        let rows = statement.query_map(
            named_params! {
                ":repository_id": repository_id.as_str(),
                ":after": after.as_deref(),
                ":fetch_limit": fetch_limit,
            },
            |row| {
                row.get::<_, String>(0).and_then(|repository_id| {
                    row.get::<_, String>(1).and_then(|item_id| {
                        row.get::<_, String>(2).and_then(|reply_draft_id| {
                            row.get::<_, String>(3).map(|linked_at| {
                                repo_com_state::ReplyLinkRecord {
                                    repository_id,
                                    item_id,
                                    reply_draft_id,
                                    linked_at,
                                }
                            })
                        })
                    })
                })
            },
        )?;
        let mut projected = Vec::new();
        for row in rows {
            let record = row?;
            let value = self.reply_projection(&record);
            projected.push((value.item_id.clone(), value));
        }
        Ok(finish_page(
            repository_id,
            REPLY_LINK_OBJECT,
            page.limit,
            projected,
        ))
    }

    fn reply_projection(&self, record: &repo_com_state::ReplyLinkRecord) -> ReplyLinkProjection {
        let mut projection = ReplyLinkProjection::linked_only(record);
        let evidence = self.reply_evidence(record);
        match evidence {
            Some(ReplyEvidence {
                event_id,
                accepted_delivery_id,
                remote_message_id,
            }) => {
                projection.replied = true;
                projection.local_only_link = false;
                projection.evidence_status = ReplyClaimStatus::AcceptedEvidence;
                projection.audit_event_id = Some(safe_text(event_id));
                projection.accepted_delivery_id = Some(safe_text(accepted_delivery_id));
                projection.accepted_remote_message_id = Some(safe_text(remote_message_id));
            }
            None => projection.evidence_status = ReplyClaimStatus::NotClaimed,
        }
        projection
    }

    fn attach_reply_evidence(&self, projection: &mut InboundItemProjection) {
        if let Some(link) = projection.local_state.reply_link.clone() {
            let record = repo_com_state::ReplyLinkRecord {
                repository_id: link.repository_id.clone(),
                item_id: link.item_id.clone(),
                reply_draft_id: link.reply_draft_id.clone(),
                linked_at: link.linked_at.clone(),
            };
            projection.local_state.reply_link = Some(self.reply_projection(&record));
        }
    }

    fn reply_evidence(&self, link: &repo_com_state::ReplyLinkRecord) -> Option<ReplyEvidence> {
        let event = self
            .state
            .connection()
            .query_row(
                "SELECT event_id, metadata_json
                 FROM audit_events
                 WHERE repository_id = ?1 AND object_type = 'inbound_item'
                   AND object_id = ?2 AND transition = 'replied' AND outcome = 'accepted'
                 ORDER BY audit_id DESC LIMIT 1",
                params![link.repository_id, link.item_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .ok()
            .flatten()?;
        let (event_id, metadata) = event;
        let value: Value = serde_json::from_str(&metadata).ok()?;
        let accepted_delivery_id = value.get("accepted_delivery_id")?.as_str()?.to_owned();
        let remote_message_id = value.get("reply_remote_message_id")?.as_str()?.to_owned();
        let draft_id = value.get("draft_id")?.as_str()?;
        let revision_hash = value.get("revision_hash")?.as_str()?;
        if value.get("schema_version")?.as_u64()? != 1
            || value.get("repository_id")?.as_str()? != link.repository_id
            || value.get("inbound_item_id")?.as_str()? != link.item_id
            || value.get("target_message_id")?.as_str()? != link.item_id
            || draft_id != link.reply_draft_id
            || value.get("human_message_delivery_claimed")?.as_bool()?
            || !valid_component(&accepted_delivery_id)
            || !valid_component(&remote_message_id)
        {
            return None;
        }
        let stored_revision_hash = self
            .state
            .connection()
            .query_row(
                "SELECT content_hash FROM draft_revisions
                 WHERE repository_id = ?1 AND draft_id = ?2 AND revision = ?3",
                params![
                    link.repository_id,
                    link.reply_draft_id,
                    value.get("revision")?.as_i64()?
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten();
        if stored_revision_hash.as_deref() != Some(revision_hash) {
            return None;
        }
        let attempt = self
            .state
            .connection()
            .query_row(
                "SELECT draft_id, revision, state, remote_message_id
                 FROM delivery_attempts WHERE repository_id = ?1 AND attempt_id = ?2",
                params![link.repository_id, accepted_delivery_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                },
            )
            .optional()
            .ok()
            .flatten()?;
        let expected_revision = value.get("revision")?.as_u64();
        if attempt.0 != link.reply_draft_id
            || !matches!(attempt.2.as_str(), "accepted" | "reconciled_accepted")
            || attempt.3.as_deref() != Some(remote_message_id.as_str())
            || expected_revision.and_then(|value| i64::try_from(value).ok()) != Some(attempt.1)
        {
            return None;
        }
        Some(ReplyEvidence {
            event_id,
            accepted_delivery_id,
            remote_message_id,
        })
    }
}

struct ReplyEvidence {
    event_id: String,
    accepted_delivery_id: String,
    remote_message_id: String,
}

fn ensure_repository(connection: &Connection, repository_id: &str) -> LifecycleResult<()> {
    let exists = connection
        .query_row(
            "SELECT 1 FROM repositories WHERE repository_id = ?1 LIMIT 1",
            [repository_id],
            |_| Ok(()),
        )
        .optional()?;
    if exists.is_some() {
        Ok(())
    } else {
        Err(LifecycleError::RepositoryNotFound {
            repository_id: repository_id.to_owned(),
        })
    }
}

fn validate_repository(value: &str) -> LifecycleResult<String> {
    if valid_component(value) {
        Ok(value.to_owned())
    } else {
        Err(LifecycleError::InvalidRepository)
    }
}

fn validate_object_id(object_type: &'static str, value: &str) -> LifecycleResult<String> {
    if valid_component(value) {
        Ok(value.to_owned())
    } else {
        Err(LifecycleError::InvalidObject { object_type })
    }
}

fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(
                    character,
                    '-' | '_' | '.' | '/' | ':' | '@' | '+' | '#' | '='
                )
        })
        && redact_text(value) == value
}

fn safe_text(value: String) -> String {
    if value.len() > MAX_SAFE_TEXT_LENGTH {
        REDACTED.to_owned()
    } else {
        redact_text(&value)
    }
}

fn safe_json(value: &str) -> String {
    if value.len() > MAX_SAFE_TEXT_LENGTH {
        REDACTED.to_owned()
    } else {
        serde_json::from_str::<Value>(value)
            .ok()
            .map(|value| redact_metadata(&value))
            .and_then(|value| serde_json::to_string(&value).ok())
            .unwrap_or_else(|| REDACTED.to_owned())
    }
}

fn row_to_draft(row: &Row<'_>) -> rusqlite::Result<DraftProjection> {
    Ok(DraftProjection {
        repository_id: safe_text(row.get(0)?),
        draft_id: safe_text(row.get(1)?),
        event_type: safe_text(row.get(2)?),
        destination_alias: safe_text(row.get(3)?),
        status: safe_text(row.get(4)?),
        current_revision: row.get(5)?,
        expiry_at: row.get::<_, Option<String>>(6)?.map(safe_text),
        reply_to_inbound_item_id: row.get::<_, Option<String>>(7)?.map(safe_text),
        metadata_json: safe_json(&row.get::<_, String>(8)?),
        created_at: safe_text(row.get(9)?),
        updated_at: safe_text(row.get(10)?),
        provenance: EvidenceSource::LocalState,
    })
}

fn row_to_draft_revision(row: &Row<'_>) -> rusqlite::Result<DraftRevisionProjection> {
    let body = row.get::<_, Option<String>>(4)?;
    Ok(DraftRevisionProjection {
        repository_id: safe_text(row.get(0)?),
        draft_id: safe_text(row.get(1)?),
        revision: row.get(2)?,
        content_hash: safe_text(row.get(3)?),
        body: body.map(RetainedContent::new),
        metadata_json: safe_json(&row.get::<_, String>(5)?),
        destination_alias: safe_text(row.get(6)?),
        resolved_destination: safe_text(row.get(7)?),
        expiry_at: row.get::<_, Option<String>>(8)?.map(safe_text),
        lifecycle_state: safe_text(row.get(9)?),
        reply_to_inbound_item_id: row.get::<_, Option<String>>(10)?.map(safe_text),
        created_at: safe_text(row.get(11)?),
        provenance: EvidenceSource::LocalState,
        remote_fetch_performed: false,
    })
}

fn row_to_delivery(row: &Row<'_>) -> rusqlite::Result<DeliveryAttemptProjection> {
    let remote_message_id = row.get::<_, Option<String>>(9)?.map(safe_text);
    let completed_at = row.get::<_, Option<String>>(7)?.map(safe_text);
    let last_recorded_remote =
        remote_message_id
            .as_ref()
            .map(|message_id| LastRecordedDeliveryEvidence {
                message_id: message_id.clone(),
                observed_at: completed_at.clone(),
                provenance: EvidenceSource::LastRecordedRemoteFetch,
            });
    Ok(DeliveryAttemptProjection {
        repository_id: safe_text(row.get(0)?),
        attempt_id: safe_text(row.get(1)?),
        draft_id: safe_text(row.get(2)?),
        revision: row.get(3)?,
        attempt_number: row.get(4)?,
        claim_nonce: safe_text(row.get(5)?),
        state: safe_text(row.get(6)?),
        claimed_at: safe_text(row.get(7)?),
        completed_at,
        remote_message_id,
        last_recorded_remote,
        failure_code: row.get::<_, Option<String>>(10)?.map(safe_text),
        read_receipt: ReadReceiptStatus::Unavailable,
        reply_claim: ReplyClaimStatus::NotClaimed,
        provenance: EvidenceSource::LocalState,
    })
}

fn row_to_inbound(
    row: &Row<'_>,
    include_retained_content: bool,
) -> rusqlite::Result<InboundItemProjection> {
    let first_content = row.get::<_, Option<String>>(4)?;
    let current_content = row.get::<_, Option<String>>(7)?;
    let current_content_present = row.get::<_, Option<i64>>(15)?.unwrap_or(0) == 1;
    let current_snapshot_present = row.get::<_, Option<i64>>(16)?.unwrap_or(0) == 1;
    let current_deleted = row
        .get::<_, Option<i64>>(9)?
        .is_some_and(|value| value == 1);
    let current_observed_at = match row.get::<_, Option<String>>(10)? {
        Some(value) => value,
        None => row.get::<_, String>(6)?,
    };
    let current_state = if !current_snapshot_present {
        RemoteSnapshotState::Unknown
    } else if current_content_present && !current_deleted {
        RemoteSnapshotState::Present
    } else if current_deleted {
        RemoteSnapshotState::Deleted
    } else {
        RemoteSnapshotState::Unknown
    };
    let first_snapshot = InboundSnapshotProjection {
        observed_at: safe_text(row.get::<_, String>(6)?),
        content: include_retained_content
            .then(|| first_content.map(RetainedContent::new))
            .flatten(),
        deleted: false,
        attachment_metadata: safe_json(&row.get::<_, String>(5)?),
        provenance: EvidenceSource::LastRecordedRemoteFetch,
    };
    let current_snapshot = InboundSnapshotProjection {
        observed_at: safe_text(current_observed_at.clone()),
        content: include_retained_content
            .then(|| current_content.map(RetainedContent::new))
            .flatten(),
        deleted: current_deleted,
        attachment_metadata: safe_json(&row.get::<_, Option<String>>(8)?.unwrap_or_default()),
        provenance: EvidenceSource::LastRecordedRemoteFetch,
    };
    let reply_link = match (
        row.get::<_, Option<String>>(13)?,
        row.get::<_, Option<String>>(14)?,
    ) {
        (Some(reply_draft_id), Some(linked_at)) => Some(ReplyLinkProjection::linked_only(
            &repo_com_state::ReplyLinkRecord {
                repository_id: row.get::<_, String>(0)?,
                item_id: row.get::<_, String>(1)?,
                reply_draft_id,
                linked_at,
            },
        )),
        _ => None,
    };
    Ok(InboundItemProjection {
        repository_id: safe_text(row.get(0)?),
        item_id: safe_text(row.get(1)?),
        channel_id: safe_text(row.get(2)?),
        author_id: safe_text(row.get(3)?),
        first_snapshot,
        current_snapshot,
        last_recorded_remote: RemoteSnapshotProjection {
            observed_at: safe_text(current_observed_at),
            state: current_state,
            provenance: EvidenceSource::LastRecordedRemoteFetch,
            current_remote_truth: false,
        },
        local_state: LocalInboundMarkers {
            acknowledged_at: row.get::<_, Option<String>>(11)?.map(safe_text),
            archived_at: row.get::<_, Option<String>>(12)?.map(safe_text),
            reply_link,
        },
        untrusted: true,
    })
}

fn row_to_acknowledgement(row: &Row<'_>) -> rusqlite::Result<AcknowledgementProjection> {
    Ok(AcknowledgementProjection {
        repository_id: safe_text(row.get(0)?),
        item_id: safe_text(row.get(1)?),
        acknowledged_at: safe_text(row.get(2)?),
        local_only: true,
        remote_effect: None,
        provenance: EvidenceSource::LocalState,
    })
}

fn row_to_archive(row: &Row<'_>) -> rusqlite::Result<ArchiveProjection> {
    Ok(ArchiveProjection {
        repository_id: safe_text(row.get(0)?),
        item_id: safe_text(row.get(1)?),
        archived_at: safe_text(row.get(2)?),
        local_only: true,
        remote_effect: None,
        provenance: EvidenceSource::LocalState,
    })
}

fn finish_page<T>(
    repository_id: String,
    object_type: &str,
    page_size: usize,
    mut rows: Vec<(String, T)>,
) -> LifecyclePage<T> {
    let truncated = rows.len() > page_size;
    let next_after = if truncated {
        rows.get(page_size.saturating_sub(1))
            .map(|(key, _)| encode_cursor(&repository_id, key))
    } else {
        None
    };
    rows.truncate(page_size);
    LifecyclePage {
        repository_id,
        object_type: object_type.to_owned(),
        items: rows.into_iter().map(|(_, value)| value).collect(),
        page_size,
        truncated,
        next_after,
    }
}

fn encode_cursor(repository_id: &str, key: &str) -> String {
    format!("{repository_id}{CURSOR_SEPARATOR}{key}")
}

fn decode_after(repository_id: &str, value: Option<&str>) -> LifecycleResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if let Some((cursor_repository, key)) = value.split_once(CURSOR_SEPARATOR) {
        if cursor_repository != repository_id {
            return Err(LifecycleError::CrossRepositoryDenied {
                repository_id: repository_id.to_owned(),
            });
        }
        if key.is_empty() {
            return Err(LifecycleError::InvalidPage { field: "after" });
        }
        return Ok(Some(key.to_owned()));
    }
    if value.is_empty() || value.len() > MAX_CURSOR_LENGTH {
        return Err(LifecycleError::InvalidPage { field: "after" });
    }
    Ok(Some(value.to_owned()))
}

fn revision_key(draft_id: &str, revision: i64) -> String {
    format!("{draft_id}{REVISION_SEPARATOR}{revision:020}")
}

fn split_revision_cursor(value: &str) -> LifecycleResult<(Option<String>, i64)> {
    let Some((draft_id, revision)) = value.rsplit_once(REVISION_SEPARATOR) else {
        return Err(LifecycleError::InvalidPage { field: "after" });
    };
    let revision = revision
        .parse::<i64>()
        .map_err(|_| LifecycleError::InvalidPage { field: "after" })?;
    if draft_id.is_empty() || revision <= 0 {
        return Err(LifecycleError::InvalidPage { field: "after" });
    }
    Ok((Some(draft_id.to_owned()), revision))
}

fn map_audit_error(error: AuditQueryError) -> LifecycleError {
    match error {
        AuditQueryError::InvalidFilter { .. } | AuditQueryError::InvalidCursor => {
            LifecycleError::InvalidPage { field: "audit" }
        }
        AuditQueryError::UnsafeStoredEvidence => LifecycleError::UnsafeStoredEvidence,
        AuditQueryError::Storage => LifecycleError::Storage,
    }
}
