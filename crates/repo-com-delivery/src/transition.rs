use std::thread;

use repo_com_audit::AuditEvent;
use repo_com_state::{AuditEventInput, DeliveryAttemptRecord, StateError, StateTransaction};
use rusqlite::{OptionalExtension, params};

use crate::claim::{
    DeliveryCoordinator, DeliveryError, audit_event_id, latest_event_of_type,
    load_attempt_in_transaction, validate_audit_timestamp, validate_identifier,
};
use crate::model::{DeliveryAttempt, DeliveryState, PersistedAttemptMetadata, TransitionRequest};

const MAX_TRANSITION_CONTENTION_RETRIES: usize = 32;

impl DeliveryCoordinator {
    /// Records one legal post-claim local transition and its matching audit
    /// event in the same SQLite transaction.
    pub fn record_transition(
        &mut self,
        request: &TransitionRequest,
    ) -> Result<DeliveryAttempt, DeliveryError> {
        validate_transition_request(request)?;
        let mut contention = 0usize;
        loop {
            let transaction = match self.state_mut().begin_transaction() {
                Ok(transaction) => transaction,
                Err(_error @ StateError::LockTimeout { .. })
                    if contention < MAX_TRANSITION_CONTENTION_RETRIES =>
                {
                    contention += 1;
                    thread::yield_now();
                    continue;
                }
                Err(error) => return Err(error.into()),
            };

            let result = record_transition_in_transaction(&transaction, request);
            let attempt = match result {
                Ok(attempt) => attempt,
                Err(error)
                    if matches!(error, DeliveryError::State(StateError::LockTimeout { .. }))
                        && contention < MAX_TRANSITION_CONTENTION_RETRIES =>
                {
                    contention += 1;
                    thread::yield_now();
                    continue;
                }
                Err(error) => return Err(error),
            };
            transaction.commit()?;
            return Ok(attempt);
        }
    }

    /// Compatibility alias for [`Self::record_transition`].
    pub fn transition(
        &mut self,
        request: &TransitionRequest,
    ) -> Result<DeliveryAttempt, DeliveryError> {
        self.record_transition(request)
    }

    /// Records an accepted Discord response.
    pub fn record_accepted(
        &mut self,
        mut request: TransitionRequest,
    ) -> Result<DeliveryAttempt, DeliveryError> {
        request.state = DeliveryState::Accepted;
        self.record_transition(&request)
    }

    /// Records a definitive Discord rejection.
    pub fn record_definitive_failed(
        &mut self,
        mut request: TransitionRequest,
    ) -> Result<DeliveryAttempt, DeliveryError> {
        request.state = DeliveryState::Failed;
        self.record_transition(&request)
    }

    /// Records a retry-wait outcome without implementing retry timing.
    pub fn record_retry_wait(
        &mut self,
        mut request: TransitionRequest,
    ) -> Result<DeliveryAttempt, DeliveryError> {
        request.state = DeliveryState::RetryWait;
        self.record_transition(&request)
    }

    /// Records a post-dispatch ambiguous outcome. This method never labels an
    /// unknown result as failed, absent, or safe to retry.
    pub fn record_unknown(
        &mut self,
        mut request: TransitionRequest,
    ) -> Result<DeliveryAttempt, DeliveryError> {
        request.state = DeliveryState::Unknown;
        self.record_transition(&request)
    }

    /// Compatibility aliases used by transport adapters.
    pub fn mark_accepted(
        &mut self,
        request: TransitionRequest,
    ) -> Result<DeliveryAttempt, DeliveryError> {
        self.record_accepted(request)
    }

    /// Compatibility alias for definitive failure.
    pub fn mark_failed(
        &mut self,
        request: TransitionRequest,
    ) -> Result<DeliveryAttempt, DeliveryError> {
        self.record_definitive_failed(request)
    }

    /// Compatibility alias for retry-wait.
    pub fn mark_retry_wait(
        &mut self,
        request: TransitionRequest,
    ) -> Result<DeliveryAttempt, DeliveryError> {
        self.record_retry_wait(request)
    }

    /// Compatibility alias for unknown.
    pub fn mark_unknown(
        &mut self,
        request: TransitionRequest,
    ) -> Result<DeliveryAttempt, DeliveryError> {
        self.record_unknown(request)
    }

    /// Reads one exact attempt through a short read transaction.
    pub fn read_attempt(
        &mut self,
        repository_id: &str,
        attempt_id: &str,
    ) -> Result<DeliveryAttempt, DeliveryError> {
        let transaction = self.state_mut().begin_transaction()?;
        let state_attempt = transaction
            .repositories()
            .delivery_attempts()
            .get(repository_id, attempt_id)?
            .ok_or(DeliveryError::AttemptNotFound)?;
        let result = load_attempt_in_transaction(&transaction, &state_attempt);
        transaction.commit()?;
        result
    }

    /// Reads the latest attempt for one exact revision.
    pub fn read_revision_outcome(
        &mut self,
        repository_id: &str,
        draft_id: &str,
        revision: u64,
    ) -> Result<Option<DeliveryAttempt>, DeliveryError> {
        let transaction = self.state_mut().begin_transaction()?;
        let revision = i64::try_from(revision)
            .map_err(|_| DeliveryError::InvalidInput { field: "revision" })?;
        let state_attempt = transaction
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
            .map_err(|error| {
                DeliveryError::State(StateError::Sqlite {
                    path: None,
                    operation: "read delivery outcome",
                    message: error.to_string(),
                })
            })?;
        let result = state_attempt
            .as_ref()
            .map(|attempt| load_attempt_in_transaction(&transaction, attempt))
            .transpose();
        transaction.commit()?;
        result
    }
}

fn record_transition_in_transaction(
    transaction: &StateTransaction<'_>,
    request: &TransitionRequest,
) -> Result<DeliveryAttempt, DeliveryError> {
    let state_attempt = transaction
        .repositories()
        .delivery_attempts()
        .get(&request.repository_id, &request.attempt_id)?
        .ok_or(DeliveryError::AttemptNotFound)?;
    if state_attempt.draft_id != request.draft_id
        || u64::try_from(state_attempt.revision).ok() != Some(request.revision)
    {
        return Err(DeliveryError::StaleInput {
            field: "attempt identity",
        });
    }
    let current = load_attempt_in_transaction(transaction, &state_attempt)?;
    if request.completed_at < current.started_at {
        return Err(DeliveryError::StaleInput {
            field: "completion time",
        });
    }
    if current.state == request.state {
        if transition_matches(&current, request) {
            return Ok(current);
        }
        return Err(DeliveryError::StaleInput {
            field: "transition metadata",
        });
    }
    if !current.state.can_transition_to(request.state) {
        return Err(DeliveryError::InvalidTransition {
            from: current.state,
            to: request.state,
        });
    }
    if request.state == DeliveryState::Accepted {
        let message_id =
            request
                .remote_message_id
                .as_deref()
                .ok_or(DeliveryError::InvalidInput {
                    field: "remote message id",
                })?;
        validate_identifier("remote message id", message_id)?;
        let duplicate = transaction
            .query_row(
                "SELECT attempt_id FROM delivery_attempts
                 WHERE repository_id = ?1 AND remote_message_id = ?2
                   AND state = 'accepted' AND attempt_id <> ?3
                 LIMIT 1",
                params![request.repository_id, message_id, request.attempt_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| {
                DeliveryError::State(StateError::Sqlite {
                    path: None,
                    operation: "check accepted message uniqueness",
                    message: error.to_string(),
                })
            })?;
        if duplicate.is_some() {
            return Err(DeliveryError::RemoteMessageConflict);
        }
    }

    let mut metadata = claim_metadata_for_attempt(transaction, &state_attempt)?;
    metadata.completed_at = Some(request.completed_at.clone());
    metadata.code = request.error_code.clone();
    metadata.remote_message_id = request.remote_message_id.clone();

    let changed = if request.state == DeliveryState::RetryWait {
        transaction.execute(
            "UPDATE delivery_attempts
             SET completed_at = ?3, failure_code = ?4
             WHERE repository_id = ?1 AND attempt_id = ?2 AND state = 'claimed'",
            params![
                request.repository_id,
                request.attempt_id,
                request.completed_at,
                request.error_code
            ],
        )?
    } else {
        transaction.execute(
            "UPDATE delivery_attempts
             SET state = ?3, completed_at = ?4, remote_message_id = ?5, failure_code = ?6
             WHERE repository_id = ?1 AND attempt_id = ?2 AND state = 'claimed'",
            params![
                request.repository_id,
                request.attempt_id,
                request.state.as_str(),
                request.completed_at,
                request.remote_message_id,
                request.error_code
            ],
        )?
    };
    if changed != 1 {
        return Err(DeliveryError::StaleInput {
            field: "attempt state",
        });
    }
    append_transition_audit(transaction, request, &metadata)?;
    let updated = transaction
        .repositories()
        .delivery_attempts()
        .get(&request.repository_id, &request.attempt_id)?
        .ok_or(DeliveryError::AttemptNotFound)?;
    load_attempt_in_transaction(transaction, &updated)
}

fn validate_transition_request(request: &TransitionRequest) -> Result<(), DeliveryError> {
    validate_identifier("repository id", &request.repository_id)?;
    validate_identifier("draft id", &request.draft_id)?;
    validate_identifier("attempt id", &request.attempt_id)?;
    validate_identifier("actor kind", &request.actor_kind)?;
    validate_audit_timestamp(&request.completed_at)?;
    match request.state {
        DeliveryState::Accepted => {
            if request.error_code.is_some() {
                return Err(DeliveryError::InvalidInput {
                    field: "accepted error code",
                });
            }
            if request.remote_message_id.is_none() {
                return Err(DeliveryError::InvalidInput {
                    field: "remote message id",
                });
            }
        }
        DeliveryState::Failed | DeliveryState::Unknown | DeliveryState::RetryWait => {
            if request.error_code.is_none() {
                return Err(DeliveryError::InvalidInput {
                    field: "error code",
                });
            }
            if request.remote_message_id.is_some() {
                return Err(DeliveryError::InvalidInput {
                    field: "remote message id",
                });
            }
        }
        DeliveryState::Unclaimed
        | DeliveryState::Claimed
        | DeliveryState::ReconciledAccepted
        | DeliveryState::ReconciledAbsent
        | DeliveryState::Unresolved => {
            return Err(DeliveryError::InvalidInput {
                field: "transition state",
            });
        }
    }
    if let Some(code) = request.error_code.as_deref() {
        validate_error_code(code)?;
    }
    if let Some(message_id) = request.remote_message_id.as_deref() {
        validate_identifier("remote message id", message_id)?;
    }
    Ok(())
}

fn validate_error_code(value: &str) -> Result<(), DeliveryError> {
    validate_identifier("error code", value)?;
    if value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:-".contains(&byte))
    {
        return Err(DeliveryError::InvalidInput {
            field: "error code",
        });
    }
    Ok(())
}

fn transition_matches(current: &DeliveryAttempt, request: &TransitionRequest) -> bool {
    current.completed_at.as_deref() == Some(request.completed_at.as_str())
        && current.error_code == request.error_code
        && current.remote_message_id == request.remote_message_id
}

fn claim_metadata_for_attempt(
    transaction: &StateTransaction<'_>,
    state_attempt: &DeliveryAttemptRecord,
) -> Result<PersistedAttemptMetadata, DeliveryError> {
    let event = latest_event_of_type(
        transaction,
        &state_attempt.repository_id,
        &state_attempt.attempt_id,
        "claimed",
    )?
    .ok_or(DeliveryError::CorruptAttempt)?;
    Ok(serde_json::from_str(&event.metadata_json)?)
}

fn append_transition_audit(
    transaction: &StateTransaction<'_>,
    request: &TransitionRequest,
    metadata: &PersistedAttemptMetadata,
) -> Result<(), DeliveryError> {
    let event_id = audit_event_id(
        request.state.as_str(),
        &request.repository_id,
        &request.draft_id,
        &request.attempt_id,
    );
    if transaction
        .repositories()
        .audit()
        .get(&request.repository_id, &event_id)?
        .is_some()
    {
        return Err(DeliveryError::AuditConflict);
    }
    let event = AuditEvent::with_metadata(
        &request.repository_id,
        event_id,
        "delivery_attempt",
        &request.attempt_id,
        request.state.as_str(),
        &request.completed_at,
        &request.actor_kind,
        request.state.as_str(),
        metadata.to_value()?,
    );
    let input: AuditEventInput = event.to_state_input()?;
    transaction.append_audit_event(&input)?;
    Ok(())
}

fn row_to_attempt(row: &rusqlite::Row<'_>) -> rusqlite::Result<DeliveryAttemptRecord> {
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
