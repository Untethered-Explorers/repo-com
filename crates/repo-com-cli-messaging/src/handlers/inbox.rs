//! Inbox fetch and local lifecycle command ports.

use repo_com_config::ResolvedConfig;
use repo_com_foundation::RepoComError;
use repo_com_inbox_fetch::{
    FetchBoundary, FetchContinuation, FetchRequest as DomainFetchRequest, StoredFetchResult,
    UntrustedInboundEnvelope,
};
use serde::{Deserialize, Serialize};

use crate::MessagingResult;
use crate::input::{InboxFetchInput, InboxItemActionInput};

/// A bounded fetch continuation projection safe for machine output.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FetchContinuationView {
    /// Cursor to use for the next explicit fetch, if any.
    pub next_cursor: Option<String>,
    /// Whether another explicit fetch is indicated.
    pub has_more: bool,
    /// Stable continuation reason.
    pub reason: String,
    /// Pages consumed by this call.
    pub pages_fetched: usize,
    /// Raw messages counted by this call.
    pub raw_messages_seen: usize,
    /// Whether the hard page bound stopped the call.
    pub page_limit_reached: bool,
    /// Whether the hard raw-message bound stopped the call.
    pub message_limit_reached: bool,
    /// Remaining page budget.
    pub pages_remaining: usize,
    /// Remaining raw-message budget.
    pub raw_messages_remaining: usize,
}

impl From<&FetchContinuation> for FetchContinuationView {
    fn from(value: &FetchContinuation) -> Self {
        Self {
            next_cursor: value.next_cursor.clone(),
            has_more: value.has_more(),
            reason: continuation_reason(value.reason),
            pages_fetched: value.pages_fetched,
            raw_messages_seen: value.raw_messages_seen,
            page_limit_reached: value.page_limit_reached,
            message_limit_reached: value.message_limit_reached,
            pages_remaining: value.pages_remaining,
            raw_messages_remaining: value.raw_messages_remaining,
        }
    }
}

/// A bounded point-check continuation projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeContinuationView {
    /// Offset for the remaining point checks.
    pub next_offset: usize,
    /// Remaining local item identifiers.
    pub remaining_item_ids: Vec<String>,
}

/// The local page commit summary returned with a fetch.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FetchCommitView {
    /// Repository scope committed.
    pub repository_id: String,
    /// Alias whose cursor was committed.
    pub alias: String,
    /// Authoritative cursor after commit.
    pub cursor: String,
    /// Number of page items processed.
    pub stored_items: usize,
    /// Number of transitions stored.
    pub stored_transitions: usize,
}

/// Explicit trust marker for all inbound fetch results.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InboundTrust {
    /// Remote data is untrusted and cannot grant authority.
    Untrusted,
}

/// Complete safe result of one explicit bounded fetch and local commit.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InboxFetchResult {
    /// Repository scope.
    pub repository_id: String,
    /// Configured inbound alias.
    pub alias: String,
    /// Resolved configured channel.
    pub channel_id: String,
    /// Retained items, all explicitly marked untrusted by their domain type.
    pub items: Vec<UntrustedInboundEnvelope>,
    /// Raw messages counted before filtering.
    pub raw_messages: usize,
    /// Pages consumed.
    pub pages_fetched: usize,
    /// Cursor made authoritative by the local commit.
    pub authoritative_cursor: String,
    /// Explicit continuation metadata.
    pub continuation: FetchContinuationView,
    /// Local commit summary.
    pub commit: FetchCommitView,
    /// Number of point checks attempted.
    pub point_checks_attempted: usize,
    /// Number of observed edits.
    pub point_checks_edited: usize,
    /// Number of observed deletions.
    pub point_checks_deleted: usize,
    /// Remaining point checks, if bounded.
    pub point_check_continuation: Option<ProbeContinuationView>,
    /// Every returned item is untrusted; this marker makes that machine-safe.
    pub trust: InboundTrust,
}

impl InboxFetchResult {
    /// Projects a domain stored-fetch result without interpreting inbound text.
    #[must_use]
    pub fn from_stored(value: StoredFetchResult) -> Self {
        let continuation = FetchContinuationView::from(&value.fetch.continuation);
        let commit = FetchCommitView {
            repository_id: value.commit.repository_id,
            alias: value.commit.alias,
            cursor: value.commit.cursor,
            stored_items: value.commit.stored_items,
            stored_transitions: value.commit.stored_transitions,
        };
        let point_check_continuation =
            value
                .reconciliation
                .continuation
                .as_ref()
                .map(|continuation| ProbeContinuationView {
                    next_offset: continuation.next_offset,
                    remaining_item_ids: continuation.remaining_item_ids.clone(),
                });
        Self {
            repository_id: value.fetch.repository_id,
            alias: value.fetch.alias,
            channel_id: value.fetch.channel_id,
            items: value.fetch.items,
            raw_messages: value.fetch.raw_messages,
            pages_fetched: value.fetch.pages_fetched,
            authoritative_cursor: value.fetch.authoritative_cursor,
            continuation,
            commit,
            point_checks_attempted: value.reconciliation.attempted,
            point_checks_edited: value.reconciliation.edited_count(),
            point_checks_deleted: value.reconciliation.deleted_count(),
            point_check_continuation,
            trust: InboundTrust::Untrusted,
        }
    }
}

/// A validated, explicit inbound fetch command.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InboxFetchCommand {
    /// Repository scope.
    pub repository_id: String,
    /// Configured inbound alias.
    pub alias: String,
    /// Exactly one cursor or time boundary is represented by these fields.
    pub cursor: Option<String>,
    /// Optional RFC 3339 time boundary.
    pub time: Option<String>,
    /// Dedicated bot user ID used for untrusted mention evidence.
    pub bot_user_id: String,
    /// Optional deterministic observation timestamp.
    pub retrieved_at: Option<String>,
}

impl InboxFetchCommand {
    /// Converts to the domain fetch request after rechecking the exact-one
    /// boundary rule.
    pub fn to_domain(&self) -> Result<DomainFetchRequest, RepoComError> {
        let boundary = FetchBoundary::from_parts(self.cursor.as_deref(), self.time.as_deref())
            .map_err(|_| {
                RepoComError::usage("exactly one valid cursor or time boundary is required")
            })?;
        let request = DomainFetchRequest::new(
            self.repository_id.clone(),
            self.alias.clone(),
            boundary,
            self.bot_user_id.clone(),
        );
        Ok(match &self.retrieved_at {
            Some(retrieved_at) => request.with_retrieved_at(retrieved_at.clone()),
            None => request,
        })
    }
}

impl From<InboxFetchInput> for InboxFetchCommand {
    fn from(value: InboxFetchInput) -> Self {
        Self {
            repository_id: value.repository_id,
            alias: value.alias,
            cursor: value.cursor,
            time: value.time,
            bot_user_id: value.bot_user_id,
            retrieved_at: value.retrieved_at,
        }
    }
}

/// The local-only inbound action selected by the command.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum InboundAction {
    /// Mark items acknowledged locally.
    Acknowledge,
    /// Mark items archived locally.
    Archive,
}

/// A validated local lifecycle request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InboundActionRequest {
    /// Repository scope.
    pub repository_id: String,
    /// Exact stored item identifiers.
    pub item_ids: Vec<String>,
    /// Caller-supplied canonical timestamp.
    pub at: String,
    /// Selected local action.
    pub action: InboundAction,
}

impl InboundActionRequest {
    /// Creates an acknowledgement request.
    #[must_use]
    pub fn acknowledge(value: InboxItemActionInput) -> Self {
        Self {
            repository_id: value.repository_id,
            item_ids: value.item_ids,
            at: value.at,
            action: InboundAction::Acknowledge,
        }
    }

    /// Creates an archive request.
    #[must_use]
    pub fn archive(value: InboxItemActionInput) -> Self {
        Self {
            repository_id: value.repository_id,
            item_ids: value.item_ids,
            at: value.at,
            action: InboundAction::Archive,
        }
    }
}

/// Safe local result of an acknowledgement or archive.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InboundActionResult {
    /// Repository scope.
    pub repository_id: String,
    /// Local action.
    pub action: InboundAction,
    /// Exact item IDs affected idempotently.
    pub item_ids: Vec<String>,
    /// Timestamp supplied to the local owner.
    pub at: String,
    /// This operation never changes Discord state.
    pub remote_mutation: bool,
}

/// Domain port for bounded, untrusted inbound retrieval.
#[allow(async_fn_in_trait)]
pub trait InboxFetchService {
    /// Fetches, stores, and point-reconciles one explicit page.
    async fn fetch(
        &mut self,
        request: InboxFetchCommand,
        config: &ResolvedConfig,
    ) -> MessagingResult<InboxFetchResult>;
}

/// Domain port for local-only acknowledgement and archival.
#[allow(async_fn_in_trait)]
pub trait InboxLifecycleService {
    /// Applies one idempotent local action.
    async fn apply(
        &mut self,
        request: InboundActionRequest,
    ) -> MessagingResult<InboundActionResult>;
}

/// Fetches one explicit page through the supplied domain owner.
pub async fn fetch<S>(
    service: &mut S,
    request: InboxFetchCommand,
    config: &ResolvedConfig,
) -> MessagingResult<InboxFetchResult>
where
    S: InboxFetchService + ?Sized,
{
    if request.repository_id != config.config.repository_id {
        return Err(RepoComError::usage(
            "inbound fetch repository does not match the resolved configuration",
        ));
    }
    let Some(inbound) = config.inbound(&request.alias) else {
        return Err(RepoComError::usage("inbound alias is not configured"));
    };
    if !inbound.enabled {
        return Err(RepoComError::usage("inbound alias is disabled"));
    }
    // Validate the boundary before the domain port can perform any I/O.
    request.to_domain()?;
    service.fetch(request, config).await
}

/// Applies an acknowledgement or archive through the local domain owner.
pub async fn apply<S>(
    service: &mut S,
    request: InboundActionRequest,
) -> MessagingResult<InboundActionResult>
where
    S: InboxLifecycleService + ?Sized,
{
    service.apply(request).await
}

fn continuation_reason(reason: repo_com_inbox_fetch::ContinuationReason) -> String {
    match reason {
        repo_com_inbox_fetch::ContinuationReason::Complete => "complete",
        repo_com_inbox_fetch::ContinuationReason::PageLimit => "page-limit",
        repo_com_inbox_fetch::ContinuationReason::MessageLimit => "message-limit",
        repo_com_inbox_fetch::ContinuationReason::BothLimits => "both-limits",
    }
    .to_owned()
}
