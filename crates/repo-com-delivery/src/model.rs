use std::fmt;

use repo_com_config::{ResolvedConfig, ResolvedDestination};
use repo_com_draft_content::RenderedMessage;
use repo_com_policy::PolicyTuple;
use repo_com_send_eligibility::{EligibilityAuthority, EligibilityDecision};
use serde::{Deserialize, Serialize};

/// The durable delivery states exposed by the coordinator.
///
/// `retry_wait` is a recorded transport outcome. It is deliberately distinct
/// from a definitive failure: only the later retry owner may explicitly claim
/// the next attempt after observing this state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    /// No local delivery claim exists for the exact revision.
    Unclaimed,
    /// A claim and its matching audit event have committed.
    Claimed,
    /// Discord returned a validated message identifier.
    Accepted,
    /// Discord definitively rejected the request.
    Failed,
    /// A proven pre-dispatch failure or rate limit requires an explicit retry.
    RetryWait,
    /// Dispatch may have reached Discord and the outcome is not proven.
    Unknown,
    /// Reconciliation found one exact matching remote message.
    ReconciledAccepted,
    /// Reconciliation conservatively proved absence after its observation gate.
    ReconciledAbsent,
    /// Reconciliation found conflicting or insufficient evidence.
    Unresolved,
}

impl DeliveryState {
    /// Returns the stable state spelling used by local persistence and audit.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unclaimed => "unclaimed",
            Self::Claimed => "claimed",
            Self::Accepted => "accepted",
            Self::Failed => "failed",
            Self::RetryWait => "retry_wait",
            Self::Unknown => "unknown",
            Self::ReconciledAccepted => "reconciled_accepted",
            Self::ReconciledAbsent => "reconciled_absent",
            Self::Unresolved => "unresolved",
        }
    }

    /// Parses a state persisted by the state or audit layer.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "unclaimed" => Some(Self::Unclaimed),
            "claimed" => Some(Self::Claimed),
            "accepted" => Some(Self::Accepted),
            "failed" => Some(Self::Failed),
            "retry_wait" => Some(Self::RetryWait),
            "unknown" => Some(Self::Unknown),
            "reconciled_accepted" => Some(Self::ReconciledAccepted),
            "reconciled_absent" => Some(Self::ReconciledAbsent),
            "unresolved" => Some(Self::Unresolved),
            _ => None,
        }
    }

    /// Returns whether this state is terminal for ordinary delivery.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Accepted
                | Self::Failed
                | Self::ReconciledAccepted
                | Self::ReconciledAbsent
                | Self::Unresolved
        )
    }

    /// Returns whether this state must block an automatic resend.
    #[must_use]
    pub const fn blocks_automatic_retry(self) -> bool {
        matches!(
            self,
            Self::Claimed | Self::Unknown | Self::ReconciledAbsent | Self::Unresolved
        )
    }

    /// Returns whether a fresh transport claim is allowed after this state.
    #[must_use]
    pub const fn permits_next_attempt(self) -> bool {
        matches!(self, Self::RetryWait)
    }

    /// Returns the only legal forward transition from this state.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (Self::Unclaimed, Self::Claimed)
                | (Self::Claimed, Self::Accepted)
                | (Self::Claimed, Self::Failed)
                | (Self::Claimed, Self::RetryWait)
                | (Self::Claimed, Self::Unknown)
                | (Self::RetryWait, Self::Claimed)
                | (Self::Unknown, Self::ReconciledAccepted)
                | (Self::Unknown, Self::ReconciledAbsent)
                | (Self::Unknown, Self::Unresolved)
        )
    }
}

impl fmt::Display for DeliveryState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Borrowed inputs for one atomic claim.
///
/// The decision and rendered message are intentionally borrowed. The
/// coordinator does not create or refresh authority; it revalidates these
/// exact projections against the repository-scoped transaction before it
/// records anything.
#[derive(Clone)]
pub struct ClaimRequest<'a> {
    /// A prior pure eligibility decision for the same exact revision.
    pub decision: &'a EligibilityDecision,
    /// The freshly resolved current configuration.
    pub config: &'a ResolvedConfig,
    /// The exact rendered message that will be sent by the transport owner.
    pub rendered: &'a RenderedMessage,
    /// Exact event/destination/severity tuple used for policy revalidation.
    pub policy_tuple: PolicyTuple,
    /// Injected current Unix time used for all expiry checks.
    pub now_unix_seconds: u64,
    /// Canonical UTC timestamp for the local claim audit event.
    pub started_at: String,
    /// Stable caller-selected attempt ID; an empty value requests a derived ID.
    pub attempt_id: String,
    /// Deterministic per-attempt request nonce. The first attempt uses the
    /// rendered content nonce; retry attempts use a deterministic suffix while
    /// the rendered content nonce remains unchanged.
    pub request_nonce: String,
    /// Stable non-secret actor kind for the audit envelope.
    pub actor_kind: String,
}

impl<'a> ClaimRequest<'a> {
    /// Creates a request using the rendered deterministic nonce and a derived
    /// attempt ID. Callers may replace the IDs with stable application IDs.
    #[must_use]
    pub fn new(
        decision: &'a EligibilityDecision,
        config: &'a ResolvedConfig,
        rendered: &'a RenderedMessage,
        policy_tuple: PolicyTuple,
        now_unix_seconds: u64,
        started_at: impl Into<String>,
    ) -> Self {
        let facts = decision.revalidation();
        let attempt_id = format!(
            "delivery-{}-r{}",
            stable_id_component(&facts.draft_id),
            facts.revision
        );
        Self {
            decision,
            config,
            rendered,
            policy_tuple,
            now_unix_seconds,
            started_at: started_at.into(),
            attempt_id,
            request_nonce: rendered.nonce().to_owned(),
            actor_kind: "system".to_owned(),
        }
    }

    /// Replaces the stable attempt ID.
    #[must_use]
    pub fn with_attempt_id(mut self, value: impl Into<String>) -> Self {
        self.attempt_id = value.into();
        self
    }

    /// Replaces the per-attempt request nonce. The value is checked against
    /// the rendered revision at the claim boundary.
    #[must_use]
    pub fn with_request_nonce(mut self, value: impl Into<String>) -> Self {
        self.request_nonce = value.into();
        self
    }

    /// Replaces the audit actor kind.
    #[must_use]
    pub fn with_actor_kind(mut self, value: impl Into<String>) -> Self {
        self.actor_kind = value.into();
        self
    }
}

impl fmt::Debug for ClaimRequest<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClaimRequest")
            .field("decision", &self.decision)
            .field("config_hash", &self.config.canonical_hash())
            .field("rendered_content", &"[REDACTED]")
            .field("content_nonce", &self.rendered.nonce())
            .field("policy_tuple", &self.policy_tuple)
            .field("now_unix_seconds", &self.now_unix_seconds)
            .field("started_at", &self.started_at)
            .field("attempt_id", &self.attempt_id)
            .field("request_nonce", &self.request_nonce)
            .field("actor_kind", &self.actor_kind)
            .finish()
    }
}

/// Owned convenience form of [`ClaimRequest`].
#[derive(Clone)]
pub struct ClaimInput {
    /// Current pure eligibility decision.
    pub decision: EligibilityDecision,
    /// Current resolved configuration.
    pub config: ResolvedConfig,
    /// Exact rendered message.
    pub rendered: RenderedMessage,
    /// Exact policy tuple.
    pub policy_tuple: PolicyTuple,
    /// Injected current time.
    pub now_unix_seconds: u64,
    /// Claim timestamp.
    pub started_at: String,
    /// Attempt ID.
    pub attempt_id: String,
    /// Request nonce.
    pub request_nonce: String,
    /// Audit actor kind.
    pub actor_kind: String,
}

impl fmt::Debug for ClaimInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClaimInput")
            .field("decision", &self.decision)
            .field("config_hash", &self.config.canonical_hash())
            .field("rendered_content", &"[REDACTED]")
            .field("content_nonce", &self.rendered.nonce())
            .field("policy_tuple", &self.policy_tuple)
            .field("now_unix_seconds", &self.now_unix_seconds)
            .field("started_at", &self.started_at)
            .field("attempt_id", &self.attempt_id)
            .field("request_nonce", &self.request_nonce)
            .field("actor_kind", &self.actor_kind)
            .finish()
    }
}

impl ClaimInput {
    /// Creates an owned request from the normal borrowed inputs.
    #[must_use]
    pub fn new(
        decision: &EligibilityDecision,
        config: &ResolvedConfig,
        rendered: &RenderedMessage,
        policy_tuple: PolicyTuple,
        now_unix_seconds: u64,
        started_at: impl Into<String>,
    ) -> Self {
        let request = ClaimRequest::new(
            decision,
            config,
            rendered,
            policy_tuple,
            now_unix_seconds,
            started_at,
        );
        Self {
            decision: decision.clone(),
            config: config.clone(),
            rendered: rendered.clone(),
            policy_tuple: request.policy_tuple,
            now_unix_seconds: request.now_unix_seconds,
            started_at: request.started_at,
            attempt_id: request.attempt_id,
            request_nonce: request.request_nonce,
            actor_kind: request.actor_kind,
        }
    }

    /// Borrows this input for the coordinator API.
    #[must_use]
    pub fn as_request(&self) -> ClaimRequest<'_> {
        ClaimRequest {
            decision: &self.decision,
            config: &self.config,
            rendered: &self.rendered,
            policy_tuple: self.policy_tuple.clone(),
            now_unix_seconds: self.now_unix_seconds,
            started_at: self.started_at.clone(),
            attempt_id: self.attempt_id.clone(),
            request_nonce: self.request_nonce.clone(),
            actor_kind: self.actor_kind.clone(),
        }
    }
}

/// One durable delivery attempt and its safe public projections.
#[derive(Clone, Eq, PartialEq)]
pub struct DeliveryAttempt {
    /// Repository scope.
    pub repository_id: String,
    /// Draft identity.
    pub draft_id: String,
    /// Immutable revision number.
    pub revision: u64,
    /// Stable attempt ID.
    pub attempt_id: String,
    /// Monotonic attempt number for the revision.
    pub attempt_number: u64,
    /// Deterministic per-attempt request nonce.
    pub request_nonce: String,
    /// Stable content nonce rendered into the exact Discord message.
    pub content_nonce: String,
    /// Current logical delivery state.
    pub state: DeliveryState,
    /// Canonical claim/start timestamp.
    pub started_at: String,
    /// Canonical completion timestamp, when a transport result was recorded.
    pub completed_at: Option<String>,
    /// Stable redacted error code, when present.
    pub error_code: Option<String>,
    /// Discord message ID, only after acceptance.
    pub remote_message_id: Option<String>,
    /// Exact immutable rendered content retained by the draft revision.
    pub exact_content: String,
    /// Destination alias bound to the revision.
    pub destination_alias: String,
    /// Resolved destination bound to the revision.
    pub resolved_destination: ResolvedDestination,
    /// Immutable revision hash.
    pub revision_hash: String,
    /// Current configuration hash used by the claim.
    pub config_hash: String,
    /// Hash of the exact destination snapshot.
    pub destination_hash: String,
    /// Hash of the redacted secret-scan result.
    pub scan_hash: String,
    /// Exact authority used by the claim.
    pub authority: EligibilityAuthority,
}

impl fmt::Debug for DeliveryAttempt {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeliveryAttempt")
            .field("repository_id", &self.repository_id)
            .field("draft_id", &self.draft_id)
            .field("revision", &self.revision)
            .field("attempt_id", &self.attempt_id)
            .field("attempt_number", &self.attempt_number)
            .field("request_nonce", &self.request_nonce)
            .field("content_nonce", &self.content_nonce)
            .field("state", &self.state)
            .field("started_at", &self.started_at)
            .field("completed_at", &self.completed_at)
            .field("error_code", &self.error_code)
            .field("remote_message_id", &self.remote_message_id)
            .field("exact_content", &"[REDACTED]")
            .field("destination_alias", &self.destination_alias)
            .field("resolved_destination", &self.resolved_destination)
            .field("revision_hash", &self.revision_hash)
            .field("config_hash", &self.config_hash)
            .field("destination_hash", &self.destination_hash)
            .field("scan_hash", &self.scan_hash)
            .field("authority", &self.authority)
            .finish()
    }
}

impl DeliveryAttempt {
    /// Returns the current logical state.
    #[must_use]
    pub const fn state(&self) -> DeliveryState {
        self.state
    }

    /// Returns the exact immutable content without including it in diagnostics.
    #[must_use]
    pub fn exact_content(&self) -> &str {
        &self.exact_content
    }

    /// Returns the deterministic request nonce.
    #[must_use]
    pub fn request_nonce(&self) -> &str {
        &self.request_nonce
    }

    /// Returns the nonce embedded in the exact rendered content.
    #[must_use]
    pub fn content_nonce(&self) -> &str {
        &self.content_nonce
    }

    /// Returns whether this recorded state permits an explicit next attempt.
    #[must_use]
    pub const fn permits_next_attempt(&self) -> bool {
        self.state.permits_next_attempt()
    }

    /// Returns whether this outcome blocks an automatic resend.
    #[must_use]
    pub const fn blocks_automatic_retry(&self) -> bool {
        self.state.blocks_automatic_retry()
    }

    /// Returns the accepted Discord message identifier, if known.
    #[must_use]
    pub fn message_id(&self) -> Option<&str> {
        self.remote_message_id.as_deref()
    }

    /// Returns the redacted failure code, if one was recorded.
    #[must_use]
    pub fn failure_code(&self) -> Option<&str> {
        self.error_code.as_deref()
    }

    /// Returns the claim/start timestamp.
    #[must_use]
    pub fn started_at(&self) -> &str {
        &self.started_at
    }

    /// Returns the completion timestamp, if one was recorded.
    #[must_use]
    pub fn completed_at(&self) -> Option<&str> {
        self.completed_at.as_deref()
    }
}

/// The disposition of a claim call.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClaimDisposition {
    /// This call committed the only new claim and may perform one network
    /// attempt after the transaction has returned.
    NewlyClaimed,
    /// A prior claim or recorded outcome was returned without a network permit.
    Existing,
}

/// A single-use authorization to make one transport attempt.
///
/// The coordinator never performs network I/O. A permit can only be obtained
/// from the successful newly-claimed branch, and it is intentionally not
/// cloneable so a duplicate caller cannot manufacture a second authorization.
pub struct ClaimPermit {
    attempt: DeliveryAttempt,
    rendered: RenderedMessage,
}

impl fmt::Debug for ClaimPermit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClaimPermit")
            .field("attempt", &self.attempt)
            .field("exact_content", &"[REDACTED]")
            .field("nonce", &self.rendered.nonce())
            .finish()
    }
}

impl ClaimPermit {
    pub(crate) fn new(attempt: DeliveryAttempt, rendered: RenderedMessage) -> Self {
        Self { attempt, rendered }
    }

    /// Returns the claimed attempt.
    #[must_use]
    pub const fn attempt(&self) -> &DeliveryAttempt {
        &self.attempt
    }

    /// Returns the exact rendered message for the transport adapter.
    #[must_use]
    pub const fn rendered(&self) -> &RenderedMessage {
        &self.rendered
    }

    /// Returns the deterministic content nonce used by reconciliation.
    #[must_use]
    pub fn request_nonce(&self) -> &str {
        self.rendered.nonce()
    }

    /// Explicit alias for the content nonce.
    #[must_use]
    pub fn content_nonce(&self) -> &str {
        self.rendered.nonce()
    }

    /// Returns the durable per-attempt request nonce.
    #[must_use]
    pub fn attempt_request_nonce(&self) -> &str {
        &self.attempt.request_nonce
    }

    /// Consumes the permit and returns its durable attempt projection.
    #[must_use]
    pub fn into_attempt(self) -> DeliveryAttempt {
        self.attempt
    }
}

/// Result of a claim call.
pub struct ClaimResult {
    /// Whether this call created the claim.
    pub disposition: ClaimDisposition,
    /// The recorded attempt returned to every caller.
    pub attempt: DeliveryAttempt,
    /// A permit exists only for the newly committed claim.
    pub permit: Option<ClaimPermit>,
}

impl fmt::Debug for ClaimResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClaimResult")
            .field("disposition", &self.disposition)
            .field("attempt", &self.attempt)
            .field("network_authorized", &self.permit.is_some())
            .finish()
    }
}

impl ClaimResult {
    /// Returns whether this result authorizes exactly one new transport call.
    #[must_use]
    pub const fn network_authorized(&self) -> bool {
        self.permit.is_some()
    }

    /// Returns whether this result was an existing recorded outcome.
    #[must_use]
    pub const fn is_existing(&self) -> bool {
        matches!(self.disposition, ClaimDisposition::Existing)
    }

    /// Returns the recorded attempt projection shared by duplicate callers.
    #[must_use]
    pub const fn recorded_attempt(&self) -> &DeliveryAttempt {
        &self.attempt
    }

    /// Takes the single-use permit, if this was the winning claim.
    #[must_use]
    pub fn into_permit(mut self) -> Option<ClaimPermit> {
        self.permit.take()
    }
}

/// Input for one local delivery transition.
#[derive(Clone, Eq, PartialEq)]
pub struct TransitionRequest {
    /// Repository scope.
    pub repository_id: String,
    /// Draft identity.
    pub draft_id: String,
    /// Immutable revision number.
    pub revision: u64,
    /// Attempt identity.
    pub attempt_id: String,
    /// Requested terminal or retry-wait state.
    pub state: DeliveryState,
    /// Canonical completion timestamp.
    pub completed_at: String,
    /// Stable redacted error code.
    pub error_code: Option<String>,
    /// Discord message ID for acceptance.
    pub remote_message_id: Option<String>,
    /// Stable non-secret audit actor kind.
    pub actor_kind: String,
}

impl fmt::Debug for TransitionRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TransitionRequest")
            .field("repository_id", &self.repository_id)
            .field("draft_id", &self.draft_id)
            .field("revision", &self.revision)
            .field("attempt_id", &self.attempt_id)
            .field("state", &self.state)
            .field("completed_at", &self.completed_at)
            .field(
                "error_code",
                &self.error_code.as_ref().map(|_| "[REDACTED]"),
            )
            .field("remote_message_id", &self.remote_message_id)
            .field("actor_kind", &self.actor_kind)
            .finish()
    }
}

impl TransitionRequest {
    /// Creates a transition input.
    #[must_use]
    pub fn new(
        repository_id: impl Into<String>,
        draft_id: impl Into<String>,
        revision: u64,
        attempt_id: impl Into<String>,
        state: DeliveryState,
        completed_at: impl Into<String>,
        actor_kind: impl Into<String>,
    ) -> Self {
        Self {
            repository_id: repository_id.into(),
            draft_id: draft_id.into(),
            revision,
            attempt_id: attempt_id.into(),
            state,
            completed_at: completed_at.into(),
            error_code: None,
            remote_message_id: None,
            actor_kind: actor_kind.into(),
        }
    }

    /// Adds a redacted error code.
    #[must_use]
    pub fn with_error_code(mut self, value: impl Into<String>) -> Self {
        self.error_code = Some(value.into());
        self
    }

    /// Adds a Discord message ID.
    #[must_use]
    pub fn with_remote_message_id(mut self, value: impl Into<String>) -> Self {
        self.remote_message_id = Some(value.into());
        self
    }
}

/// Metadata persisted with a claim or transition audit event.
///
/// Field names intentionally use the audit crate's safe-key vocabulary. No
/// exact message body, credential, authorization value, or response body is
/// placed in this structure.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct PersistedAttemptMetadata {
    #[serde(default)]
    pub attempt_number: Option<u64>,
    #[serde(default, rename = "nonce_hash")]
    pub request_nonce: Option<String>,
    #[serde(default, rename = "content_nonce_hash")]
    pub content_nonce: Option<String>,
    #[serde(default, rename = "updated_at")]
    pub completed_at: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub remote_message_id: Option<String>,
    #[serde(default)]
    pub revision_hash: Option<String>,
    #[serde(default)]
    pub config_hash: Option<String>,
    #[serde(default)]
    pub destination_hash: Option<String>,
    #[serde(default)]
    pub destination_alias: Option<String>,
    #[serde(default)]
    pub resolved_destination_hash: Option<String>,
    #[serde(default)]
    pub metadata_hash: Option<String>,
    #[serde(default)]
    pub exact_text_hash: Option<String>,
    #[serde(default)]
    pub scan_hash: Option<String>,
    #[serde(default)]
    pub policy_basis_hash: Option<String>,
    #[serde(default)]
    pub approval_preview_hash: Option<String>,
    #[serde(default, rename = "kind", alias = "authority_kind")]
    pub authority_kind: Option<String>,
    #[serde(default)]
    pub authority_id: Option<String>,
    #[serde(default)]
    pub authority_hash: Option<String>,
    #[serde(default)]
    pub authority_expiry: Option<u64>,
    #[serde(default)]
    pub authority_config_hash: Option<String>,
    #[serde(default)]
    pub authority_tuple_hash: Option<String>,
}

impl PersistedAttemptMetadata {
    pub(crate) fn to_value(&self) -> Result<serde_json::Value, serde_json::Error> {
        serde_json::to_value(self)
    }
}

fn stable_id_component(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
            output.push(character);
        } else {
            output.push('-');
        }
    }
    if output.is_empty() {
        "draft".to_owned()
    } else {
        output
    }
}
