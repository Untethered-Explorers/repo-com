use std::{error::Error, fmt, thread};

use repo_com_approval::{
    APPROVAL_LIFETIME_SECONDS, ApprovalRecord, OverrideReasonCode, SecretOverrideRecord,
};
use repo_com_audit::{AuditError, AuditEvent};
use repo_com_config::{ResolvedConfig, ResolvedDestination};
use repo_com_draft_model::DraftMetadata;
use repo_com_draft_safety::SecretScanner;
use repo_com_policy::{
    PolicyDecision, PolicyHashes, PolicyTuple, classify_activation_records, decision_from_status,
    exact_policy_match, is_sha256_hex,
};
use repo_com_send_eligibility::{EligibilityAuthority, EligibilityBlocker, EligibilityDecision};
use repo_com_state::{
    AuditEventInput, DeliveryAttemptInput, DeliveryAttemptRecord, PolicyActivationRecord,
    StateError, StateStore, StateTransaction,
};
use rusqlite::{OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::{
    ClaimDisposition, ClaimInput, ClaimPermit, ClaimRequest, ClaimResult, DeliveryAttempt,
    DeliveryState, PersistedAttemptMetadata,
};

const MAX_TRANSACTION_CONTENTION_RETRIES: usize = 32;

/// A safe, typed delivery-coordinator failure.
///
/// Error values contain stable categories and state-layer diagnostics only;
/// draft text, Discord response bodies, credentials, and authorization values
/// are never copied into an error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeliveryError {
    /// A required request field was empty, malformed, or internally
    /// inconsistent.
    InvalidInput { field: &'static str },
    /// A pre-claim current-state fact no longer matches the exact request.
    StaleInput { field: &'static str },
    /// The supplied pure decision is not eligible.
    NotEligible { blocker: EligibilityBlocker },
    /// The requested transition is not legal from the recorded state.
    InvalidTransition {
        from: DeliveryState,
        to: DeliveryState,
    },
    /// The exact attempt could not be found in the requested repository scope.
    AttemptNotFound,
    /// A message ID is already bound to another accepted attempt.
    RemoteMessageConflict,
    /// A deterministic audit event ID already exists with different evidence.
    AuditConflict,
    /// The claim or transition evidence is not internally coherent.
    CorruptAttempt,
    /// The repository-scoped state operation failed.
    State(StateError),
    /// The redacted audit boundary failed.
    Audit(AuditError),
    /// A safe internal value could not be serialized.
    Serialization,
}

impl DeliveryError {
    /// Returns a stable, non-sensitive error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidInput { .. } | Self::CorruptAttempt => "delivery-usage",
            Self::StaleInput { .. } | Self::NotEligible { .. } => "delivery-blocked",
            Self::InvalidTransition { .. } | Self::RemoteMessageConflict | Self::AuditConflict => {
                "delivery-transition"
            }
            Self::AttemptNotFound => "delivery-not-found",
            Self::State(_) | Self::Audit(_) => "storage-integrity",
            Self::Serialization => "delivery-serialization",
        }
    }

    fn is_lock_timeout(&self) -> bool {
        matches!(self, Self::State(StateError::LockTimeout { .. }))
    }
}

impl fmt::Display for DeliveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidInput { field } => write!(formatter, "invalid delivery input: {field}"),
            Self::StaleInput { field } => write!(formatter, "delivery input is stale: {field}"),
            Self::NotEligible { blocker } => {
                write!(formatter, "delivery is not eligible: {}", blocker.code())
            }
            Self::InvalidTransition { from, to } => {
                write!(formatter, "illegal delivery transition: {from} -> {to}")
            }
            Self::AttemptNotFound => formatter.write_str("delivery attempt was not found"),
            Self::RemoteMessageConflict => {
                formatter.write_str("Discord message ID is already accepted by another attempt")
            }
            Self::AuditConflict => {
                formatter.write_str("delivery audit event conflicts with evidence")
            }
            Self::CorruptAttempt => formatter.write_str("delivery attempt evidence is incoherent"),
            Self::State(error) => write!(formatter, "delivery state operation failed: {error}"),
            Self::Audit(error) => write!(formatter, "delivery audit operation failed: {error}"),
            Self::Serialization => formatter.write_str("delivery evidence could not be serialized"),
        }
    }
}

impl Error for DeliveryError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::State(error) => Some(error),
            Self::Audit(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StateError> for DeliveryError {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}

impl From<AuditError> for DeliveryError {
    fn from(error: AuditError) -> Self {
        Self::Audit(error)
    }
}

impl From<serde_json::Error> for DeliveryError {
    fn from(_: serde_json::Error) -> Self {
        Self::Serialization
    }
}

impl From<rusqlite::Error> for DeliveryError {
    fn from(error: rusqlite::Error) -> Self {
        Self::State(StateError::Sqlite {
            path: None,
            operation: "delivery state query",
            message: error.to_string(),
        })
    }
}

/// Durable, duplicate-safe coordinator around one repository-scoped state
/// store. The coordinator performs no Discord or other network I/O.
pub struct DeliveryCoordinator {
    state: StateStore,
}

impl fmt::Debug for DeliveryCoordinator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DeliveryCoordinator")
            .field("state", &self.state)
            .field("network_io", &false)
            .finish()
    }
}

impl DeliveryCoordinator {
    /// Creates a coordinator over an owned state store.
    #[must_use]
    pub const fn new(state: StateStore) -> Self {
        Self { state }
    }

    /// Compatibility alias for [`Self::new`].
    #[must_use]
    pub fn from_state(state: StateStore) -> Self {
        Self::new(state)
    }

    /// Returns read-only access to the underlying state store.
    #[must_use]
    pub const fn state(&self) -> &StateStore {
        &self.state
    }

    /// Returns mutable access for explicitly composed state workflows.
    pub const fn state_mut(&mut self) -> &mut StateStore {
        &mut self.state
    }

    /// Consumes the coordinator and returns its state store.
    #[must_use]
    pub fn into_state(self) -> StateStore {
        self.state
    }

    /// Claims one exact revision if no prior logical outcome exists.
    ///
    /// The claim, current-state checks, and matching audit event are committed
    /// before this method returns. Only the winning result carries a
    /// single-use [`ClaimPermit`]. Repeated and concurrent callers receive the
    /// recorded attempt and no permit.
    pub fn claim(&mut self, request: &ClaimRequest<'_>) -> Result<ClaimResult, DeliveryError> {
        self.claim_inner(request, false)
    }

    /// Claims from an owned convenience input.
    pub fn claim_input(&mut self, input: &ClaimInput) -> Result<ClaimResult, DeliveryError> {
        self.claim_inner(&input.as_request(), false)
    }

    /// Claims the next attempt only after the latest recorded state is
    /// `retry_wait`.
    ///
    /// This is an explicit state-machine boundary for the later retry owner;
    /// it contains no delay, jitter, transport loop, or automatic resend.
    pub fn claim_retry(
        &mut self,
        request: &ClaimRequest<'_>,
    ) -> Result<ClaimResult, DeliveryError> {
        self.claim_inner(request, true)
    }

    /// Claims a retry from an owned convenience input.
    pub fn claim_retry_input(&mut self, input: &ClaimInput) -> Result<ClaimResult, DeliveryError> {
        self.claim_inner(&input.as_request(), true)
    }

    /// Compatibility alias for [`Self::claim_retry`].
    pub fn claim_retry_attempt(
        &mut self,
        request: &ClaimRequest<'_>,
    ) -> Result<ClaimResult, DeliveryError> {
        self.claim_retry(request)
    }

    fn claim_inner(
        &mut self,
        request: &ClaimRequest<'_>,
        retry: bool,
    ) -> Result<ClaimResult, DeliveryError> {
        let mut contention = 0usize;
        loop {
            let transaction = match self.state.begin_transaction() {
                Ok(transaction) => transaction,
                Err(_error @ StateError::LockTimeout { .. })
                    if contention < MAX_TRANSACTION_CONTENTION_RETRIES =>
                {
                    contention += 1;
                    thread::yield_now();
                    continue;
                }
                Err(error) => return Err(error.into()),
            };

            let (attempt, disposition, permit_rendered) =
                match claim_in_transaction(&transaction, request, retry) {
                    Ok(value) => value,
                    Err(error)
                        if error.is_lock_timeout()
                            && contention < MAX_TRANSACTION_CONTENTION_RETRIES =>
                    {
                        contention += 1;
                        thread::yield_now();
                        continue;
                    }
                    Err(error) => return Err(error),
                };

            transaction.commit()?;
            let permit =
                permit_rendered.map(|rendered| ClaimPermit::new(attempt.clone(), rendered));
            return Ok(ClaimResult {
                disposition,
                attempt,
                permit,
            });
        }
    }
}

fn claim_in_transaction(
    transaction: &StateTransaction<'_>,
    request: &ClaimRequest<'_>,
    retry: bool,
) -> Result<
    (
        DeliveryAttempt,
        ClaimDisposition,
        Option<repo_com_draft_content::RenderedMessage>,
    ),
    DeliveryError,
> {
    let facts = match request.decision {
        EligibilityDecision::Eligible { revalidation, .. } => revalidation,
        EligibilityDecision::Blocked { blocker, .. } => {
            return Err(DeliveryError::NotEligible { blocker: *blocker });
        }
    };
    let existing = latest_attempt_in_transaction(
        transaction,
        &facts.repository_id,
        &facts.draft_id,
        facts.revision,
    )?;

    if let Some(existing) = existing {
        let recorded = load_attempt_in_transaction(transaction, &existing)?;
        if recorded.repository_id != facts.repository_id
            || recorded.draft_id != facts.draft_id
            || recorded.revision != facts.revision
            || recorded.content_nonce != request.rendered.nonce()
        {
            return Err(DeliveryError::StaleInput {
                field: "claim identity",
            });
        }
        if !retry || recorded.state != DeliveryState::RetryWait {
            return Ok((recorded, ClaimDisposition::Existing, None));
        }
        let validated = revalidate_request(transaction, request)?;
        ensure_same_claim(&recorded, &validated, request)?;
        let next_number =
            recorded
                .attempt_number
                .checked_add(1)
                .ok_or(DeliveryError::InvalidInput {
                    field: "attempt number",
                })?;
        let attempt_id = retry_attempt_id(&request.attempt_id, &recorded.attempt_id, next_number);
        let request_nonce = retry_request_nonce(request, next_number);
        return insert_claim(
            transaction,
            request,
            &validated,
            next_number,
            attempt_id,
            &request_nonce,
        );
    }

    let validated = revalidate_request(transaction, request)?;
    if retry {
        return Err(DeliveryError::StaleInput {
            field: "retry state",
        });
    }
    insert_claim(
        transaction,
        request,
        &validated,
        1,
        request.attempt_id.clone(),
        &request.request_nonce,
    )
}

fn insert_claim(
    transaction: &StateTransaction<'_>,
    request: &ClaimRequest<'_>,
    validated: &ValidatedClaim,
    attempt_number: u64,
    attempt_id: String,
    request_nonce: &str,
) -> Result<
    (
        DeliveryAttempt,
        ClaimDisposition,
        Option<repo_com_draft_content::RenderedMessage>,
    ),
    DeliveryError,
> {
    let attempt_id = if attempt_id.is_empty() {
        format!("delivery-{}-r{}", validated.draft_id, validated.revision)
    } else {
        attempt_id
    };
    validate_identifier("attempt id", &attempt_id)?;
    let expected_request_nonce = if attempt_number == 1 {
        request.rendered.nonce().to_owned()
    } else {
        retry_request_nonce(request, attempt_number)
    };
    if request_nonce != expected_request_nonce {
        return Err(DeliveryError::InvalidInput {
            field: "request nonce",
        });
    }
    let revision = i64::try_from(validated.revision)
        .map_err(|_| DeliveryError::InvalidInput { field: "revision" })?;
    let attempt_input = DeliveryAttemptInput {
        repository_id: validated.repository_id.clone(),
        attempt_id: attempt_id.clone(),
        draft_id: validated.draft_id.clone(),
        revision,
        attempt_number: i64::try_from(attempt_number).map_err(|_| DeliveryError::InvalidInput {
            field: "attempt number",
        })?,
        claim_nonce: request_nonce.to_owned(),
        state: DeliveryState::Claimed.as_str().to_owned(),
        claimed_at: request.started_at.clone(),
        completed_at: None,
        remote_message_id: None,
        failure_code: None,
    };
    let repositories = transaction.repositories();
    let delivery_attempts = repositories.delivery_attempts();
    if attempt_number == 1 {
        delivery_attempts.claim(&attempt_input)?;
    } else {
        delivery_attempts.record(&attempt_input)?;
    }

    let metadata = claim_metadata(validated, request, request_nonce, attempt_number)?;
    append_claim_audit(transaction, validated, &attempt_id, request, metadata)?;

    let attempt = DeliveryAttempt {
        repository_id: validated.repository_id.clone(),
        draft_id: validated.draft_id.clone(),
        revision: validated.revision,
        attempt_id,
        attempt_number,
        request_nonce: request_nonce.to_owned(),
        content_nonce: request.rendered.nonce().to_owned(),
        state: DeliveryState::Claimed,
        started_at: request.started_at.clone(),
        completed_at: None,
        error_code: None,
        remote_message_id: None,
        exact_content: validated.rendered.exact_text().to_owned(),
        destination_alias: validated.destination.alias.clone(),
        resolved_destination: validated.destination.clone(),
        revision_hash: validated.revision_hash.clone(),
        config_hash: validated.config_hash.clone(),
        destination_hash: validated.destination_hash.clone(),
        scan_hash: validated.scan_hash.clone(),
        authority: validated.authority.clone(),
    };
    Ok((
        attempt,
        ClaimDisposition::NewlyClaimed,
        Some(validated.rendered.clone()),
    ))
}

fn ensure_same_claim(
    recorded: &DeliveryAttempt,
    validated: &ValidatedClaim,
    request: &ClaimRequest<'_>,
) -> Result<(), DeliveryError> {
    if recorded.content_nonce != request.rendered.nonce()
        || recorded.revision_hash != validated.revision_hash
        || recorded.config_hash != validated.config_hash
        || recorded.destination_hash != validated.destination_hash
        || recorded.scan_hash != validated.scan_hash
        || recorded.authority != validated.authority
    {
        return Err(DeliveryError::StaleInput {
            field: "claim identity",
        });
    }
    Ok(())
}

struct ValidatedClaim {
    repository_id: String,
    draft_id: String,
    revision: u64,
    rendered: repo_com_draft_content::RenderedMessage,
    destination: ResolvedDestination,
    revision_hash: String,
    config_hash: String,
    destination_hash: String,
    metadata_hash: String,
    exact_text_hash: String,
    scan_hash: String,
    policy_basis_hash: String,
    authority: EligibilityAuthority,
}

fn revalidate_request(
    transaction: &StateTransaction<'_>,
    request: &ClaimRequest<'_>,
) -> Result<ValidatedClaim, DeliveryError> {
    let EligibilityDecision::Eligible {
        revalidation: facts,
        authority,
        ..
    } = request.decision
    else {
        let blocker = request
            .decision
            .blocker()
            .unwrap_or(EligibilityBlocker::CurrentStateInvalid);
        return Err(DeliveryError::NotEligible { blocker });
    };

    if facts.evaluated_at_unix_seconds != request.now_unix_seconds {
        return Err(DeliveryError::StaleInput {
            field: "evaluation time",
        });
    }
    if request.config.config.repository_id != facts.repository_id
        || request.rendered.repository_id() != facts.repository_id
        || facts.workspace_id != request.config.config.discord.workspace_id
    {
        return Err(DeliveryError::StaleInput {
            field: "repository scope",
        });
    }
    if facts.destination_alias != request.rendered.destination_alias()
        || facts.destination_alias != request.policy_tuple.destination_alias
    {
        return Err(DeliveryError::StaleInput {
            field: "destination alias",
        });
    }
    if request.rendered.revision() != facts.revision
        || request.rendered.content_hash() != facts.revision_hash
    {
        return Err(DeliveryError::StaleInput { field: "revision" });
    }
    for (field, value) in [
        ("repository hash", &facts.repository_hash),
        ("revision hash", &facts.revision_hash),
        ("destination hash", &facts.destination_hash),
        ("exact text hash", &facts.exact_text_hash),
        ("metadata hash", &facts.metadata_hash),
        ("config hash", &facts.config_hash),
        ("policy basis hash", &facts.policy_basis_hash),
        ("scan hash", &facts.scan_hash),
        ("approval preview hash", &facts.approval_preview_hash),
    ] {
        if !is_sha256_hex(value) {
            return Err(DeliveryError::StaleInput { field });
        }
    }
    if !valid_request_nonce(&request.request_nonce, request.rendered.nonce()) {
        return Err(DeliveryError::InvalidInput {
            field: "request nonce",
        });
    }
    validate_identifier("actor kind", &request.actor_kind)?;
    validate_audit_timestamp(&request.started_at)?;

    let revision = i64::try_from(facts.revision)
        .map_err(|_| DeliveryError::InvalidInput { field: "revision" })?;
    let repositories = transaction.repositories();
    let repository = repositories
        .repositories()
        .get(&facts.repository_id)?
        .ok_or(DeliveryError::StaleInput {
            field: "repository",
        })?;
    let draft_row = repositories
        .drafts()
        .get(&facts.repository_id, &facts.draft_id)?
        .ok_or(DeliveryError::StaleInput { field: "draft" })?;
    let revision_row = repositories
        .drafts()
        .revision(&facts.repository_id, &facts.draft_id, revision)?
        .ok_or(DeliveryError::StaleInput { field: "revision" })?;

    if draft_row.current_revision != revision
        || draft_row.destination_alias != facts.destination_alias
        || draft_row.status == "sent"
        || draft_row.status == "expired"
        || draft_row.status == "blocked"
        || revision_row.lifecycle_state == "sent"
        || revision_row.lifecycle_state == "expired"
        || revision_row.lifecycle_state == "blocked"
    {
        return Err(DeliveryError::StaleInput {
            field: "current revision",
        });
    }
    if repository.repository_id != facts.repository_id
        || repository.workspace_id != request.config.config.discord.workspace_id
        || repository.config_hash != request.config.canonical_hash()
        || facts.config_hash != request.config.canonical_hash()
        || facts.repository_hash
            != sha256_json(&RepositoryBinding {
                repository_id: &facts.repository_id,
                workspace_id: &request.config.config.discord.workspace_id,
            })?
    {
        return Err(DeliveryError::StaleInput {
            field: "repository or config",
        });
    }

    let current_destination =
        request
            .config
            .destination(&facts.destination_alias)
            .ok_or(DeliveryError::StaleInput {
                field: "destination",
            })?;
    if current_destination != request.rendered.resolved_destination()
        || current_destination.alias != facts.destination_alias
    {
        return Err(DeliveryError::StaleInput {
            field: "destination",
        });
    }
    let stored_destination: ResolvedDestination =
        serde_json::from_str(&revision_row.resolved_destination).map_err(|_| {
            DeliveryError::StaleInput {
                field: "stored destination",
            }
        })?;
    if stored_destination != *current_destination {
        return Err(DeliveryError::StaleInput {
            field: "stored destination",
        });
    }
    if revision_row.destination_alias != facts.destination_alias
        || revision_row.content_hash != facts.revision_hash
        || revision_row.body != request.rendered.exact_text()
    {
        return Err(DeliveryError::StaleInput {
            field: "revision content",
        });
    }
    let stored_metadata: DraftMetadata = serde_json::from_str(&revision_row.metadata_json)
        .map_err(|_| DeliveryError::StaleInput {
            field: "revision metadata",
        })?;
    if &stored_metadata != request.rendered.metadata()
        || request.rendered.metadata() != &stored_metadata
        || draft_row.reply_to_inbound_item_id.as_deref()
            != request
                .rendered
                .message_reference()
                .map(|reference| reference.inbound_item_id())
        || revision_row.reply_to_inbound_item_id.as_deref()
            != request
                .rendered
                .message_reference()
                .map(|reference| reference.inbound_item_id())
    {
        return Err(DeliveryError::StaleInput {
            field: "revision metadata",
        });
    }

    let expiry = parse_expiry(revision_row.expiry_at.as_deref())
        .ok_or(DeliveryError::StaleInput { field: "expiry" })?;
    if let Some(draft_expiry) = draft_row.expiry_at.as_deref()
        && parse_expiry(Some(draft_expiry)) != Some(expiry)
        && Some(draft_expiry) != revision_row.expiry_at.as_deref()
    {
        return Err(DeliveryError::StaleInput { field: "expiry" });
    }
    if facts.draft_created_at_unix_seconds != request.rendered.expiry().created_at_unix_seconds()
        || expiry != facts.draft_expires_at_unix_seconds
        || expiry != request.rendered.expiry().expires_at_unix_seconds()
        || request.now_unix_seconds < facts.draft_created_at_unix_seconds
        || request.now_unix_seconds >= expiry
    {
        return Err(DeliveryError::StaleInput { field: "expiry" });
    }

    let exact_text_hash = sha256_bytes(request.rendered.exact_text().as_bytes());
    let metadata_hash = sha256_json(request.rendered.metadata())?;
    let destination_hash = sha256_json(&DestinationBinding {
        current: Some(current_destination),
        revision: current_destination,
    })?;
    let scan = SecretScanner::new().scan_rendered(request.rendered);
    let scan_hash = sha256_json(&scan)?;
    if facts.exact_text_hash != exact_text_hash
        || facts.metadata_hash != metadata_hash
        || facts.destination_hash != destination_hash
        || facts.scan_hash != scan_hash
    {
        return Err(DeliveryError::StaleInput {
            field: "content or scan",
        });
    }

    request
        .policy_tuple
        .validate()
        .map_err(|_| DeliveryError::InvalidInput {
            field: "policy tuple",
        })?;
    if draft_row.event_type != request.policy_tuple.event_type {
        return Err(DeliveryError::StaleInput {
            field: "event type",
        });
    }
    let policy_decision =
        current_policy_decision(transaction, request.config, &request.policy_tuple)?;
    let policy_basis_hash = sha256_json(&policy_decision)?;
    if facts.policy_basis_hash != policy_basis_hash {
        return Err(DeliveryError::StaleInput {
            field: "policy basis",
        });
    }

    if scan.is_blocked() && matches!(authority, EligibilityAuthority::ActivatedPolicy { .. }) {
        return Err(DeliveryError::NotEligible {
            blocker: EligibilityBlocker::UnresolvedSecretFinding,
        });
    }

    match authority {
        EligibilityAuthority::HumanApproval {
            approval_id,
            approval_hash,
            expires_at_unix_seconds,
        } => validate_human_approval(
            transaction,
            request.config,
            request.rendered,
            facts,
            &scan,
            &policy_decision,
            approval_id.as_str(),
            approval_hash.as_str(),
            *expires_at_unix_seconds,
        )?,
        EligibilityAuthority::ActivatedPolicy {
            activation_id,
            activation_hash,
            config_hash,
            tuple_hash,
            ..
        } => validate_activated_policy(
            &policy_decision,
            request.config,
            &request.policy_tuple,
            activation_id,
            activation_hash,
            config_hash,
            tuple_hash,
        )?,
    }

    Ok(ValidatedClaim {
        repository_id: facts.repository_id.clone(),
        draft_id: facts.draft_id.clone(),
        revision: facts.revision,
        rendered: request.rendered.clone(),
        destination: current_destination.clone(),
        revision_hash: facts.revision_hash.clone(),
        config_hash: facts.config_hash.clone(),
        destination_hash,
        metadata_hash,
        exact_text_hash,
        scan_hash,
        policy_basis_hash,
        authority: authority.clone(),
    })
}

fn current_policy_decision(
    transaction: &StateTransaction<'_>,
    config: &ResolvedConfig,
    tuple: &PolicyTuple,
) -> Result<PolicyDecision, DeliveryError> {
    let configured = exact_policy_match(&config.config, tuple)
        .map_err(|_| DeliveryError::StaleInput {
            field: "policy configuration",
        })?
        .is_some();
    let current = PolicyHashes {
        config_hash: config.canonical_hash(),
        tuple_hash: tuple.canonical_hash(),
    };
    let records = policy_records(transaction, &config.config.repository_id)?;
    let status = classify_activation_records(
        &records,
        &config.config.repository_id,
        tuple,
        &current,
        configured,
    );
    let decision = decision_from_status(status);
    if !configured && matches!(decision, PolicyDecision::NotConfigured) {
        Ok(PolicyDecision::NoExactMatch)
    } else {
        Ok(decision)
    }
}

fn policy_records(
    transaction: &StateTransaction<'_>,
    repository_id: &str,
) -> Result<Vec<PolicyActivationRecord>, DeliveryError> {
    let mut statement = transaction.prepare(
        "SELECT repository_id, activation_id, config_hash, policy_tuple_hash,
                event_type, destination_alias, severity, activated_at, deactivated_at, active
         FROM policy_activations WHERE repository_id = ?1
         ORDER BY activation_id ASC",
    )?;
    let rows = statement.query_map([repository_id], row_to_policy_record)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
}

fn row_to_policy_record(row: &Row<'_>) -> rusqlite::Result<PolicyActivationRecord> {
    Ok(PolicyActivationRecord {
        repository_id: row.get(0)?,
        activation_id: row.get(1)?,
        config_hash: row.get(2)?,
        policy_tuple_hash: row.get(3)?,
        event_type: row.get(4)?,
        destination_alias: row.get(5)?,
        severity: row.get(6)?,
        activated_at: row.get(7)?,
        deactivated_at: row.get(8)?,
        active: row.get::<_, i64>(9)? == 1,
    })
}

#[allow(clippy::too_many_arguments)]
fn validate_human_approval(
    transaction: &StateTransaction<'_>,
    config: &ResolvedConfig,
    rendered: &repo_com_draft_content::RenderedMessage,
    facts: &repo_com_send_eligibility::RevalidationFacts,
    scan: &repo_com_draft_safety::SecretScanResult,
    policy_decision: &PolicyDecision,
    approval_id: &str,
    approval_hash: &str,
    expires_at_unix_seconds: u64,
) -> Result<(), DeliveryError> {
    let repositories = transaction.repositories();
    let state_approval = repositories
        .approvals()
        .get(
            &facts.repository_id,
            &facts.draft_id,
            i64::try_from(facts.revision)
                .map_err(|_| DeliveryError::InvalidInput { field: "revision" })?,
        )?
        .ok_or(DeliveryError::StaleInput { field: "approval" })?;
    if state_approval.approval_state != "approved"
        || state_approval.approval_id != approval_id
        || state_approval.draft_id != facts.draft_id
        || state_approval.revision != i64::try_from(facts.revision).unwrap_or(-1)
        || state_approval.actor_kind != "operator"
        || state_approval.revoked_at.is_some()
    {
        return Err(DeliveryError::StaleInput { field: "approval" });
    }
    let event_id = format!(
        "approval-recorded-{}",
        approval_id.strip_prefix("approval-").unwrap_or(approval_id)
    );
    let event = repositories
        .audit()
        .get(&facts.repository_id, &event_id)?
        .ok_or(DeliveryError::StaleInput {
            field: "approval evidence",
        })?;
    let persisted = decode_approval_event(&event)?;
    if event.transition != "approved"
        || event.outcome != "approved"
        || event.object_type != "draft"
        || event.object_id != facts.draft_id
        || event.actor_kind != "operator"
        || state_approval.approved_at != event.occurred_at
        || persisted.approval_id != approval_id
        || persisted.approval_id != format!("approval-{}", persisted.preview_hash)
        || persisted.repository_id != facts.repository_id
        || persisted.draft_id != facts.draft_id
        || persisted.revision != facts.revision
        || persisted.revision_hash != facts.revision_hash
        || persisted.exact_text_hash != sha256_bytes(rendered.exact_text().as_bytes())
        || persisted.metadata_hash != sha256_json(rendered.metadata())?
        || persisted.config_hash != config.canonical_hash()
        || persisted.destination_hash
            != sha256_json(&DestinationBinding {
                current: Some(config.destination(&facts.destination_alias).ok_or(
                    DeliveryError::StaleInput {
                        field: "destination",
                    },
                )?),
                revision: config.destination(&facts.destination_alias).ok_or(
                    DeliveryError::StaleInput {
                        field: "destination",
                    },
                )?,
            })?
        || persisted.policy_basis_hash != sha256_json(policy_decision)?
        || persisted.scan_hash != sha256_json(scan)?
        || persisted.preview_hash != facts.approval_preview_hash
        || persisted.draft_expires_at_unix_seconds != facts.draft_expires_at_unix_seconds
        || persisted.expires_at_unix_seconds != expires_at_unix_seconds
        || persisted.expires_at_unix_seconds
            != persisted
                .approved_at_unix_seconds
                .saturating_add(APPROVAL_LIFETIME_SECONDS)
                .min(persisted.draft_expires_at_unix_seconds)
        || persisted.expires_at_unix_seconds <= facts.evaluated_at_unix_seconds
        || persisted
            .hash()
            .map_err(|_| DeliveryError::CorruptAttempt)?
            != approval_hash
        || persisted.approved_at_unix_seconds >= persisted.expires_at_unix_seconds
        || persisted.override_hash.is_some() != scan.is_blocked()
    {
        return Err(DeliveryError::StaleInput {
            field: "approval binding",
        });
    }
    if scan.is_blocked() {
        let override_hash =
            persisted
                .override_hash
                .as_deref()
                .ok_or(DeliveryError::StaleInput {
                    field: "secret override",
                })?;
        validate_override(
            transaction,
            &facts.repository_id,
            &facts.draft_id,
            facts.revision,
            &facts.revision_hash,
            &facts.scan_hash,
            &facts.approval_preview_hash,
            override_hash,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn validate_override(
    transaction: &StateTransaction<'_>,
    repository_id: &str,
    draft_id: &str,
    revision: u64,
    revision_hash: &str,
    scan_hash: &str,
    preview_hash: &str,
    expected_hash: &str,
) -> Result<(), DeliveryError> {
    let event_id = format!("secret-override-{preview_hash}");
    let event = transaction
        .repositories()
        .audit()
        .get(repository_id, &event_id)?
        .ok_or(DeliveryError::StaleInput {
            field: "secret override",
        })?;
    if event.transition != "secret_finding_overridden"
        || event.outcome != "overridden"
        || event.object_type != "draft"
        || event.object_id != draft_id
    {
        return Err(DeliveryError::StaleInput {
            field: "secret override",
        });
    }
    let value: serde_json::Value = serde_json::from_str(&event.metadata_json)?;
    let record_value = value
        .get("override_record")
        .ok_or(DeliveryError::CorruptAttempt)?;
    let record: PersistedOverride = serde_json::from_value(record_value.clone())?;
    if record.schema_version != 1
        || record.event_id != event_id
        || record.repository_id != repository_id
        || record.draft_id != draft_id
        || record.revision != revision
        || record.revision_hash != revision_hash
        || record.scan_hash != scan_hash
        || record.preview_hash != preview_hash
        || !is_sha256_hex(expected_hash)
    {
        return Err(DeliveryError::StaleInput {
            field: "secret override",
        });
    }
    let reconstructed = SecretOverrideRecord {
        schema_version: record.schema_version,
        event_id: record.event_id,
        repository_id: record.repository_id,
        draft_id: record.draft_id,
        revision: record.revision,
        revision_hash: record.revision_hash,
        scan_hash: record.scan_hash,
        preview_hash: record.preview_hash,
        reason_code: record.reason,
        created_at_unix_seconds: record.created_at_unix_seconds,
        created_at_utc: event.occurred_at.clone(),
    };
    if reconstructed
        .hash()
        .map_err(|_| DeliveryError::CorruptAttempt)?
        != expected_hash
    {
        return Err(DeliveryError::StaleInput {
            field: "secret override",
        });
    }
    Ok(())
}

fn validate_activated_policy(
    policy_decision: &PolicyDecision,
    config: &ResolvedConfig,
    tuple: &PolicyTuple,
    activation_id: &str,
    activation_hash: &str,
    config_hash: &str,
    tuple_hash: &str,
) -> Result<(), DeliveryError> {
    let PolicyDecision::Eligible(snapshot) = policy_decision else {
        return Err(DeliveryError::StaleInput {
            field: "policy activation",
        });
    };
    if snapshot.activation_id != activation_id
        || snapshot.repository_id != config.config.repository_id
        || snapshot.current_config_hash != config.canonical_hash()
        || snapshot.recorded_config_hash != config.canonical_hash()
        || snapshot.current_tuple_hash != tuple.canonical_hash()
        || snapshot.recorded_tuple_hash != tuple.canonical_hash()
        || snapshot.tuple != *tuple
        || !snapshot.is_current()
        || config_hash != snapshot.current_config_hash
        || tuple_hash != snapshot.current_tuple_hash
        || sha256_json(snapshot)? != activation_hash
    {
        return Err(DeliveryError::StaleInput {
            field: "policy activation",
        });
    }
    Ok(())
}

fn claim_metadata(
    validated: &ValidatedClaim,
    request: &ClaimRequest<'_>,
    request_nonce: &str,
    attempt_number: u64,
) -> Result<PersistedAttemptMetadata, DeliveryError> {
    let (
        authority_kind,
        authority_id,
        authority_hash,
        authority_expiry,
        authority_config_hash,
        authority_tuple_hash,
    ) = authority_metadata(&validated.authority);
    Ok(PersistedAttemptMetadata {
        attempt_number: Some(attempt_number),
        request_nonce: Some(request_nonce.to_owned()),
        content_nonce: Some(request.rendered.nonce().to_owned()),
        completed_at: None,
        code: None,
        remote_message_id: None,
        revision_hash: Some(validated.revision_hash.clone()),
        config_hash: Some(validated.config_hash.clone()),
        destination_hash: Some(validated.destination_hash.clone()),
        destination_alias: Some(validated.destination.alias.clone()),
        resolved_destination_hash: Some(validated.destination_hash.clone()),
        metadata_hash: Some(validated.metadata_hash.clone()),
        exact_text_hash: Some(validated.exact_text_hash.clone()),
        scan_hash: Some(validated.scan_hash.clone()),
        policy_basis_hash: Some(validated.policy_basis_hash.clone()),
        approval_preview_hash: Some(
            request
                .decision
                .revalidation()
                .approval_preview_hash
                .clone(),
        ),
        authority_kind: Some(authority_kind.to_owned()),
        authority_id: Some(authority_id),
        authority_hash: Some(authority_hash),
        authority_expiry,
        authority_config_hash: Some(authority_config_hash),
        authority_tuple_hash: Some(authority_tuple_hash),
    })
}

fn authority_metadata(
    authority: &EligibilityAuthority,
) -> (&'static str, String, String, Option<u64>, String, String) {
    match authority {
        EligibilityAuthority::HumanApproval {
            approval_id,
            approval_hash,
            expires_at_unix_seconds,
        } => (
            "human-approval",
            approval_id.clone(),
            approval_hash.clone(),
            Some(*expires_at_unix_seconds),
            String::new(),
            String::new(),
        ),
        EligibilityAuthority::ActivatedPolicy {
            activation_id,
            activation_hash,
            config_hash,
            tuple_hash,
            ..
        } => (
            "activated-policy",
            activation_id.clone(),
            activation_hash.clone(),
            None,
            config_hash.clone(),
            tuple_hash.clone(),
        ),
    }
}

fn append_claim_audit(
    transaction: &StateTransaction<'_>,
    validated: &ValidatedClaim,
    attempt_id: &str,
    request: &ClaimRequest<'_>,
    metadata: PersistedAttemptMetadata,
) -> Result<(), DeliveryError> {
    let event_id = claim_audit_event_id(
        &validated.repository_id,
        &validated.draft_id,
        validated.revision,
        metadata
            .attempt_number
            .ok_or(DeliveryError::CorruptAttempt)?,
    );
    if transaction
        .repositories()
        .audit()
        .get(&validated.repository_id, &event_id)?
        .is_some()
    {
        return Err(DeliveryError::AuditConflict);
    }
    let event = AuditEvent::with_metadata(
        &validated.repository_id,
        event_id,
        "delivery_attempt",
        attempt_id,
        "claimed",
        &request.started_at,
        &request.actor_kind,
        "claimed",
        metadata.to_value()?,
    );
    let input: AuditEventInput = event.to_state_input()?;
    transaction.append_audit_event(&input)?;
    Ok(())
}

fn latest_attempt_in_transaction(
    transaction: &StateTransaction<'_>,
    repository_id: &str,
    draft_id: &str,
    revision: u64,
) -> Result<Option<DeliveryAttemptRecord>, DeliveryError> {
    let revision =
        i64::try_from(revision).map_err(|_| DeliveryError::InvalidInput { field: "revision" })?;
    transaction
        .query_row(
            "SELECT repository_id, attempt_id, draft_id, revision, attempt_number, claim_nonce,
                    state, claimed_at, completed_at, remote_message_id, failure_code
             FROM delivery_attempts
             WHERE repository_id = ?1 AND draft_id = ?2 AND revision = ?3
             ORDER BY attempt_number DESC LIMIT 1",
            params![repository_id, draft_id, revision],
            row_to_attempt,
        )
        .optional()
        .map_err(Into::into)
}

fn row_to_attempt(row: &Row<'_>) -> rusqlite::Result<DeliveryAttemptRecord> {
    Ok(DeliveryAttemptRecord {
        repository_id: row.get(0)?,
        attempt_id: row.get(1)?,
        draft_id: row.get(2)?,
        revision: row.get(3)?,
        attempt_number: row.get(4)?,
        claim_nonce: row.get(5)?,
        state: row.get(6)?,
        claimed_at: row.get(7)?,
        completed_at: row.get(8)?,
        remote_message_id: row.get(9)?,
        failure_code: row.get(10)?,
    })
}

pub(crate) fn load_attempt_in_transaction(
    transaction: &StateTransaction<'_>,
    state_attempt: &DeliveryAttemptRecord,
) -> Result<DeliveryAttempt, DeliveryError> {
    let repositories = transaction.repositories();
    let revision = repositories
        .drafts()
        .revision(
            &state_attempt.repository_id,
            &state_attempt.draft_id,
            state_attempt.revision,
        )?
        .ok_or(DeliveryError::CorruptAttempt)?;
    let destination: ResolvedDestination = serde_json::from_str(&revision.resolved_destination)
        .map_err(|_| DeliveryError::CorruptAttempt)?;
    let claim_event = latest_event_of_type(
        transaction,
        &state_attempt.repository_id,
        &state_attempt.attempt_id,
        "claimed",
    )?
    .ok_or(DeliveryError::CorruptAttempt)?;
    let latest_event = latest_event_for_attempt(
        transaction,
        &state_attempt.repository_id,
        &state_attempt.attempt_id,
    )?
    .ok_or(DeliveryError::CorruptAttempt)?;
    let claim_metadata: PersistedAttemptMetadata =
        serde_json::from_str(&claim_event.metadata_json)?;
    let latest_metadata: PersistedAttemptMetadata =
        serde_json::from_str(&latest_event.metadata_json)?;
    let state = DeliveryState::parse(&latest_event.transition)
        .or_else(|| DeliveryState::parse(&state_attempt.state))
        .ok_or(DeliveryError::CorruptAttempt)?;
    let base_state =
        DeliveryState::parse(&state_attempt.state).ok_or(DeliveryError::CorruptAttempt)?;
    let consistent_base = state == base_state
        || (base_state == DeliveryState::Claimed && state == DeliveryState::RetryWait)
        || (base_state == DeliveryState::Unknown
            && matches!(
                state,
                DeliveryState::ReconciledAccepted
                    | DeliveryState::ReconciledAbsent
                    | DeliveryState::Unresolved
            ));
    if !consistent_base {
        return Err(DeliveryError::CorruptAttempt);
    }
    let attempt_number =
        u64::try_from(state_attempt.attempt_number).map_err(|_| DeliveryError::CorruptAttempt)?;
    if latest_metadata
        .attempt_number
        .is_some_and(|number| number != attempt_number)
        || claim_metadata.attempt_number != Some(attempt_number)
    {
        return Err(DeliveryError::CorruptAttempt);
    }
    let revision_number =
        u64::try_from(state_attempt.revision).map_err(|_| DeliveryError::CorruptAttempt)?;
    let authority =
        authority_from_metadata(transaction, &state_attempt.repository_id, &claim_metadata)?;
    Ok(DeliveryAttempt {
        repository_id: state_attempt.repository_id.clone(),
        draft_id: state_attempt.draft_id.clone(),
        revision: revision_number,
        attempt_id: state_attempt.attempt_id.clone(),
        attempt_number,
        request_nonce: state_attempt.claim_nonce.clone(),
        content_nonce: claim_metadata
            .content_nonce
            .clone()
            .unwrap_or_else(|| state_attempt.claim_nonce.clone()),
        state,
        started_at: state_attempt.claimed_at.clone(),
        completed_at: latest_metadata
            .completed_at
            .clone()
            .or_else(|| state_attempt.completed_at.clone()),
        error_code: latest_metadata
            .code
            .clone()
            .or_else(|| state_attempt.failure_code.clone()),
        remote_message_id: latest_metadata
            .remote_message_id
            .clone()
            .or_else(|| state_attempt.remote_message_id.clone()),
        exact_content: revision.body,
        destination_alias: revision.destination_alias,
        resolved_destination: destination,
        revision_hash: claim_metadata
            .revision_hash
            .ok_or(DeliveryError::CorruptAttempt)?,
        config_hash: claim_metadata
            .config_hash
            .ok_or(DeliveryError::CorruptAttempt)?,
        destination_hash: claim_metadata
            .destination_hash
            .ok_or(DeliveryError::CorruptAttempt)?,
        scan_hash: claim_metadata
            .scan_hash
            .ok_or(DeliveryError::CorruptAttempt)?,
        authority,
    })
}

fn latest_event_for_attempt(
    transaction: &StateTransaction<'_>,
    repository_id: &str,
    attempt_id: &str,
) -> Result<Option<repo_com_state::AuditEventRecord>, DeliveryError> {
    transaction
        .query_row(
            "SELECT audit_id, repository_id, event_id, object_type, object_id, transition,
                    occurred_at, actor_kind, outcome, metadata_json
             FROM audit_events
             WHERE repository_id = ?1 AND object_type = 'delivery_attempt' AND object_id = ?2
             ORDER BY audit_id DESC LIMIT 1",
            params![repository_id, attempt_id],
            |row| {
                Ok(repo_com_state::AuditEventRecord {
                    audit_id: row.get(0)?,
                    repository_id: row.get(1)?,
                    event_id: row.get(2)?,
                    object_type: row.get(3)?,
                    object_id: row.get(4)?,
                    transition: row.get(5)?,
                    occurred_at: row.get(6)?,
                    actor_kind: row.get(7)?,
                    outcome: row.get(8)?,
                    metadata_json: row.get(9)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

pub(crate) fn latest_event_of_type(
    transaction: &StateTransaction<'_>,
    repository_id: &str,
    attempt_id: &str,
    transition: &str,
) -> Result<Option<repo_com_state::AuditEventRecord>, DeliveryError> {
    transaction
        .query_row(
            "SELECT audit_id, repository_id, event_id, object_type, object_id, transition,
                    occurred_at, actor_kind, outcome, metadata_json
             FROM audit_events
             WHERE repository_id = ?1 AND object_type = 'delivery_attempt' AND object_id = ?2
               AND transition = ?3
             ORDER BY audit_id DESC LIMIT 1",
            params![repository_id, attempt_id, transition],
            |row| {
                Ok(repo_com_state::AuditEventRecord {
                    audit_id: row.get(0)?,
                    repository_id: row.get(1)?,
                    event_id: row.get(2)?,
                    object_type: row.get(3)?,
                    object_id: row.get(4)?,
                    transition: row.get(5)?,
                    occurred_at: row.get(6)?,
                    actor_kind: row.get(7)?,
                    outcome: row.get(8)?,
                    metadata_json: row.get(9)?,
                })
            },
        )
        .optional()
        .map_err(Into::into)
}

fn authority_from_metadata(
    transaction: &StateTransaction<'_>,
    repository_id: &str,
    metadata: &PersistedAttemptMetadata,
) -> Result<EligibilityAuthority, DeliveryError> {
    let kind = metadata
        .authority_kind
        .as_deref()
        .ok_or(DeliveryError::CorruptAttempt)?;
    let id = metadata
        .authority_id
        .clone()
        .ok_or(DeliveryError::CorruptAttempt)?;
    let hash = metadata
        .authority_hash
        .clone()
        .ok_or(DeliveryError::CorruptAttempt)?;
    match kind {
        "human-approval" => Ok(EligibilityAuthority::HumanApproval {
            approval_id: id,
            approval_hash: hash,
            expires_at_unix_seconds: metadata
                .authority_expiry
                .ok_or(DeliveryError::CorruptAttempt)?,
        }),
        "activated-policy" => {
            let record = policy_record_by_id(transaction, repository_id, &id)?
                .ok_or(DeliveryError::CorruptAttempt)?;
            Ok(EligibilityAuthority::ActivatedPolicy {
                activation_id: id,
                activation_hash: hash,
                config_hash: metadata
                    .authority_config_hash
                    .clone()
                    .ok_or(DeliveryError::CorruptAttempt)?,
                tuple_hash: metadata
                    .authority_tuple_hash
                    .clone()
                    .ok_or(DeliveryError::CorruptAttempt)?,
                activated_at: record.activated_at,
            })
        }
        _ => Err(DeliveryError::CorruptAttempt),
    }
}

fn policy_record_by_id(
    transaction: &StateTransaction<'_>,
    repository_id: &str,
    activation_id: &str,
) -> Result<Option<PolicyActivationRecord>, DeliveryError> {
    transaction
        .query_row(
            "SELECT repository_id, activation_id, config_hash, policy_tuple_hash,
                    event_type, destination_alias, severity, activated_at, deactivated_at, active
             FROM policy_activations WHERE repository_id = ?1 AND activation_id = ?2 LIMIT 1",
            params![repository_id, activation_id],
            row_to_policy_record,
        )
        .optional()
        .map_err(Into::into)
}

pub(crate) fn claim_audit_event_id(
    repository_id: &str,
    draft_id: &str,
    revision: u64,
    attempt_number: u64,
) -> String {
    let material = format!("claimed|{repository_id}|{draft_id}|{revision}|{attempt_number}");
    format!("delivery-claim-{}", sha256_bytes(material.as_bytes()))
}

pub(crate) fn audit_event_id(
    transition: &str,
    repository_id: &str,
    draft_id: &str,
    attempt_id: &str,
) -> String {
    let material = format!("{transition}|{repository_id}|{draft_id}|{attempt_id}");
    format!(
        "delivery-{}-{}",
        transition,
        sha256_bytes(material.as_bytes())
    )
}

fn valid_request_nonce(value: &str, content_nonce: &str) -> bool {
    if value == content_nonce {
        return true;
    }
    let Some(suffix) = value.strip_prefix(&format!("{content_nonce}-attempt-")) else {
        return false;
    };
    !suffix.is_empty()
        && suffix.bytes().all(|byte| byte.is_ascii_digit())
        && suffix.parse::<u64>().is_ok_and(|number| number > 0)
}

fn retry_request_nonce(request: &ClaimRequest<'_>, number: u64) -> String {
    format!("{}-attempt-{number}", request.rendered.nonce())
}

fn retry_attempt_id(requested: &str, previous: &str, number: u64) -> String {
    if requested.is_empty() || requested == previous {
        format!("{previous}-retry-{number}")
    } else {
        requested.to_owned()
    }
}

pub(crate) fn validate_identifier(kind: &'static str, value: &str) -> Result<(), DeliveryError> {
    if value.is_empty()
        || value.len() > 512
        || value.contains('\0')
        || value.chars().any(char::is_control)
    {
        return Err(DeliveryError::InvalidInput { field: kind });
    }
    Ok(())
}

pub(crate) fn validate_audit_timestamp(value: &str) -> Result<(), DeliveryError> {
    let event = AuditEvent::new(
        "delivery-validation",
        "delivery-validation",
        "delivery",
        "delivery",
        "validated",
        value,
        "system",
        "valid",
    );
    event.validate().map_err(Into::into)
}

fn parse_expiry(value: Option<&str>) -> Option<u64> {
    value?.parse::<u64>().ok()
}

fn sha256_json<T: Serialize>(value: &T) -> Result<String, DeliveryError> {
    let bytes = serde_json::to_vec(value)?;
    Ok(sha256_bytes(&bytes))
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Serialize)]
struct RepositoryBinding<'a> {
    repository_id: &'a str,
    workspace_id: &'a str,
}

#[derive(Serialize)]
struct DestinationBinding<'a> {
    current: Option<&'a ResolvedDestination>,
    revision: &'a ResolvedDestination,
}

#[derive(Deserialize)]
struct ApprovalAuditEnvelope {
    schema_version: u8,
    approval: PersistedApproval,
}

#[derive(Deserialize)]
struct PersistedOverride {
    schema_version: u8,
    event_id: String,
    repository_id: String,
    draft_id: String,
    revision: u64,
    revision_hash: String,
    scan_hash: String,
    preview_hash: String,
    reason: OverrideReasonCode,
    created_at_unix_seconds: u64,
}

#[derive(Deserialize)]
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
}

fn decode_approval_event(
    event: &repo_com_state::AuditEventRecord,
) -> Result<ApprovalRecord, DeliveryError> {
    let envelope: ApprovalAuditEnvelope = serde_json::from_str(&event.metadata_json)?;
    if envelope.schema_version != 1
        || envelope.approval.schema_version != 1
        || envelope.approval.audit_event_id != event.event_id
    {
        return Err(DeliveryError::CorruptAttempt);
    }
    Ok(ApprovalRecord {
        schema_version: envelope.approval.schema_version,
        approval_id: envelope.approval.approval_id,
        audit_event_id: envelope.approval.audit_event_id,
        repository_id: envelope.approval.repository_id,
        draft_id: envelope.approval.draft_id,
        revision: envelope.approval.revision,
        revision_hash: envelope.approval.revision_hash,
        exact_text_hash: envelope.approval.exact_text_hash,
        metadata_hash: envelope.approval.metadata_hash,
        config_hash: envelope.approval.config_hash,
        destination_hash: envelope.approval.destination_hash,
        policy_basis_hash: envelope.approval.policy_basis_hash,
        scan_hash: envelope.approval.scan_hash,
        preview_hash: envelope.approval.preview_hash,
        draft_expires_at_unix_seconds: envelope.approval.draft_expires_at_unix_seconds,
        approved_at_unix_seconds: envelope.approval.approved_at_unix_seconds,
        approved_at_utc: event.occurred_at.clone(),
        expires_at_unix_seconds: envelope.approval.expires_at_unix_seconds,
        override_hash: envelope.approval.override_hash,
        actor_kind: "operator".to_owned(),
    })
}
