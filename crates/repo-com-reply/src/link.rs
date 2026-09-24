//! Local accepted-delivery linkage; no outbound or inbound remote mutation.

use repo_com_config::ResolvedDestination;
use repo_com_delivery::{DeliveryAttempt, DeliveryState};
use repo_com_state::{
    AuditEventInput, AuditEventRecord, DeliveryAttemptRecord, ReplyLinkRecord, StateError,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    ReplyCommandService, ReplyServiceError,
    target::{is_discord_id, parse_canonical_utc},
};

impl ReplyCommandService {
    /// Reads local linkage and replied status without contacting Discord.
    pub fn reply_status(
        &self,
        repository_id: &str,
        inbound_item_id: &str,
    ) -> Result<ReplyLinkStatus, ReplyServiceError> {
        let link = self
            .state()
            .reply_link(repository_id, inbound_item_id)
            .map_err(|error| ReplyServiceError::State(error.into()))?
            .ok_or(ReplyServiceError::ReplyLinkMissing)?;
        let event_id = accepted_reply_audit_event_id(&link.reply_draft_id);
        let Some(event) = self
            .state()
            .state()
            .audit_event(repository_id, &event_id)
            .map_err(ReplyServiceError::State)?
        else {
            return Ok(ReplyLinkStatus::linked_only(link));
        };
        let data: AcceptedReplyAudit = serde_json::from_str(&event.metadata_json)?;
        let durable = self
            .state()
            .state()
            .delivery_attempt(repository_id, &data.accepted_delivery_id)
            .map_err(ReplyServiceError::State)?
            .ok_or(ReplyServiceError::AuditConflict)?;
        if !accepted_audit_matches(
            &event,
            &data,
            repository_id,
            inbound_item_id,
            &link.reply_draft_id,
        ) || data.human_message_delivery_claimed
            || !matches!(
                DeliveryState::parse(&durable.state),
                Some(DeliveryState::Accepted | DeliveryState::ReconciledAccepted)
            )
            || durable.draft_id != data.draft_id
            || durable.revision != i64::try_from(data.revision).unwrap_or(-1)
            || durable.remote_message_id.as_deref() != Some(data.reply_remote_message_id.as_str())
        {
            return Err(ReplyServiceError::AuditConflict);
        }
        Ok(ReplyLinkStatus {
            link,
            replied: true,
            accepted_delivery_id: Some(data.accepted_delivery_id),
            reply_remote_message_id: Some(data.reply_remote_message_id),
            audit_event_id: Some(event_id),
        })
    }

    /// Marks the inbound item replied only after a matching durable delivery is
    /// already accepted and carries a Discord message ID.
    pub fn record_accepted_reply(
        &mut self,
        repository_id: &str,
        inbound_item_id: &str,
        attempt: &DeliveryAttempt,
    ) -> Result<ReplyAcceptedLink, ReplyServiceError> {
        if !matches!(
            attempt.state,
            DeliveryState::Accepted | DeliveryState::ReconciledAccepted
        ) {
            return Err(ReplyServiceError::InvalidAcceptedState);
        }
        let reply_remote_message_id = attempt
            .remote_message_id
            .as_deref()
            .ok_or(ReplyServiceError::MissingAcceptedRemoteMessage)?;
        if !is_discord_id(reply_remote_message_id) {
            return Err(ReplyServiceError::MissingAcceptedRemoteMessage);
        }
        let completed_at = attempt
            .completed_at
            .as_deref()
            .ok_or(ReplyServiceError::InvalidAcceptedState)?;
        parse_canonical_utc(completed_at).ok_or(ReplyServiceError::InvalidTimestamp)?;

        let link = self
            .state()
            .reply_link(repository_id, inbound_item_id)
            .map_err(|error| ReplyServiceError::State(error.into()))?
            .ok_or(ReplyServiceError::ReplyLinkMissing)?;
        if link.repository_id != attempt.repository_id || link.reply_draft_id != attempt.draft_id {
            return Err(ReplyServiceError::ReplyLinkMismatch);
        }
        let durable = self
            .state()
            .state()
            .delivery_attempt(repository_id, &attempt.attempt_id)
            .map_err(ReplyServiceError::State)?
            .ok_or(ReplyServiceError::ReplyLinkMismatch)?;
        if !persisted_attempt_matches(&durable, attempt) {
            return Err(ReplyServiceError::ReplyLinkMismatch);
        }

        let draft = self
            .state()
            .state()
            .draft(repository_id, &attempt.draft_id)
            .map_err(ReplyServiceError::State)?
            .ok_or(ReplyServiceError::ReplyLinkMismatch)?;
        if draft.reply_to_inbound_item_id.as_deref() != Some(inbound_item_id)
            || draft.current_revision != i64::try_from(attempt.revision).unwrap_or(-1)
        {
            return Err(ReplyServiceError::ReplyLinkMismatch);
        }
        let stored_revision = self
            .state()
            .state()
            .draft_revision(
                repository_id,
                &attempt.draft_id,
                i64::try_from(attempt.revision)
                    .map_err(|_| ReplyServiceError::ReplyLinkMismatch)?,
            )
            .map_err(ReplyServiceError::State)?
            .ok_or(ReplyServiceError::ReplyLinkMismatch)?;
        if stored_revision.content_hash != attempt.revision_hash
            || stored_revision.body != attempt.exact_content
            || stored_revision.destination_alias != attempt.destination_alias
            || stored_revision.reply_to_inbound_item_id.as_deref() != Some(inbound_item_id)
        {
            return Err(ReplyServiceError::ReplyLinkMismatch);
        }
        let stored_destination: ResolvedDestination =
            serde_json::from_str(&stored_revision.resolved_destination)
                .map_err(|_| ReplyServiceError::ReplyLinkMismatch)?;
        if attempt.resolved_destination != stored_destination {
            return Err(ReplyServiceError::ReplyLinkMismatch);
        }

        let data = AcceptedReplyAudit {
            schema_version: 1,
            repository_id: repository_id.to_owned(),
            inbound_item_id: inbound_item_id.to_owned(),
            target_message_id: inbound_item_id.to_owned(),
            draft_id: attempt.draft_id.clone(),
            revision: attempt.revision,
            revision_hash: attempt.revision_hash.clone(),
            accepted_delivery_id: attempt.attempt_id.clone(),
            reply_remote_message_id: reply_remote_message_id.to_owned(),
            human_message_delivery_claimed: false,
        };
        let metadata_json = serde_json::to_string(&data)?;
        let event_id = accepted_reply_audit_event_id(&attempt.draft_id);
        let audit_input = AuditEventInput {
            repository_id: repository_id.to_owned(),
            event_id: event_id.clone(),
            object_type: "inbound_item".to_owned(),
            object_id: inbound_item_id.to_owned(),
            transition: "replied".to_owned(),
            occurred_at: completed_at.to_owned(),
            actor_kind: "system".to_owned(),
            outcome: "accepted".to_owned(),
            metadata_json: metadata_json.clone(),
        };

        let event = self
            .state_mut()
            .state_mut()
            .with_transaction(|transaction| {
                let current_link = transaction
                    .repositories()
                    .inbound()
                    .reply_link(repository_id, inbound_item_id)?
                    .ok_or_else(|| StateError::NotFound {
                        entity: "reply link",
                        repository_id: repository_id.to_owned(),
                        object_id: inbound_item_id.to_owned(),
                    })?;
                if current_link != link {
                    return Err(StateError::Constraint {
                        entity: "reply link",
                        message: "reply link changed before accepted linkage".to_owned(),
                    });
                }
                let current_attempt = transaction
                    .repositories()
                    .delivery_attempts()
                    .get(repository_id, &attempt.attempt_id)?
                    .ok_or_else(|| StateError::NotFound {
                        entity: "accepted delivery",
                        repository_id: repository_id.to_owned(),
                        object_id: attempt.attempt_id.clone(),
                    })?;
                if !persisted_attempt_matches(&current_attempt, attempt) {
                    return Err(StateError::Constraint {
                        entity: "accepted delivery",
                        message: "delivery changed before accepted linkage".to_owned(),
                    });
                }
                if let Some(existing) = transaction
                    .repositories()
                    .audit()
                    .get(repository_id, &event_id)?
                {
                    if audit_event_matches(&existing, &audit_input, repository_id, inbound_item_id)
                    {
                        return Ok(existing);
                    }
                    return Err(StateError::Constraint {
                        entity: "accepted reply audit",
                        message: "accepted reply evidence conflicts with existing event".to_owned(),
                    });
                }
                transaction.append_audit_event(&audit_input)
            })?;

        let status = ReplyLinkStatus {
            link,
            replied: true,
            accepted_delivery_id: Some(attempt.attempt_id.clone()),
            reply_remote_message_id: Some(reply_remote_message_id.to_owned()),
            audit_event_id: Some(event.event_id.clone()),
        };
        Ok(ReplyAcceptedLink {
            status,
            audit: event,
        })
    }
}

/// Current local link and replied projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplyLinkStatus {
    link: ReplyLinkRecord,
    replied: bool,
    accepted_delivery_id: Option<String>,
    reply_remote_message_id: Option<String>,
    audit_event_id: Option<String>,
}

impl ReplyLinkStatus {
    fn linked_only(link: ReplyLinkRecord) -> Self {
        Self {
            link,
            replied: false,
            accepted_delivery_id: None,
            reply_remote_message_id: None,
            audit_event_id: None,
        }
    }

    #[must_use]
    pub fn link(&self) -> &ReplyLinkRecord {
        &self.link
    }

    #[must_use]
    pub fn draft_id(&self) -> &str {
        &self.link.reply_draft_id
    }

    #[must_use]
    pub const fn replied(&self) -> bool {
        self.replied
    }

    #[must_use]
    pub fn accepted_delivery_id(&self) -> Option<&str> {
        self.accepted_delivery_id.as_deref()
    }

    #[must_use]
    pub fn reply_remote_message_id(&self) -> Option<&str> {
        self.reply_remote_message_id.as_deref()
    }

    #[must_use]
    pub fn audit_event_id(&self) -> Option<&str> {
        self.audit_event_id.as_deref()
    }
}

/// Durable accepted-delivery evidence linked to one inbound item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplyAcceptedLink {
    status: ReplyLinkStatus,
    audit: AuditEventRecord,
}

impl ReplyAcceptedLink {
    #[must_use]
    pub fn status(&self) -> &ReplyLinkStatus {
        &self.status
    }

    #[must_use]
    pub fn audit(&self) -> &AuditEventRecord {
        &self.audit
    }

    #[must_use]
    pub fn draft_id(&self) -> &str {
        self.status.draft_id()
    }

    #[must_use]
    pub fn remote_message_id(&self) -> &str {
        self.status
            .reply_remote_message_id()
            .expect("an accepted reply always has a validated remote message ID")
    }

    #[must_use]
    pub fn accepted_delivery_id(&self) -> &str {
        self.status
            .accepted_delivery_id()
            .expect("an accepted reply always has a durable delivery ID")
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct AcceptedReplyAudit {
    schema_version: u8,
    repository_id: String,
    inbound_item_id: String,
    target_message_id: String,
    draft_id: String,
    revision: u64,
    revision_hash: String,
    accepted_delivery_id: String,
    reply_remote_message_id: String,
    human_message_delivery_claimed: bool,
}

fn persisted_attempt_matches(record: &DeliveryAttemptRecord, attempt: &DeliveryAttempt) -> bool {
    let revision = i64::try_from(attempt.revision).unwrap_or(-1);
    record.repository_id == attempt.repository_id
        && record.attempt_id == attempt.attempt_id
        && record.draft_id == attempt.draft_id
        && record.revision == revision
        && record.state == attempt.state.as_str()
        && record.completed_at == attempt.completed_at
        && record.remote_message_id == attempt.remote_message_id
}

fn accepted_audit_matches(
    event: &AuditEventRecord,
    data: &AcceptedReplyAudit,
    repository_id: &str,
    inbound_item_id: &str,
    draft_id: &str,
) -> bool {
    event.repository_id == repository_id
        && event.object_type == "inbound_item"
        && event.object_id == inbound_item_id
        && event.transition == "replied"
        && event.outcome == "accepted"
        && data.schema_version == 1
        && data.repository_id == repository_id
        && data.inbound_item_id == inbound_item_id
        && data.target_message_id == inbound_item_id
        && data.draft_id == draft_id
        && is_discord_id(&data.reply_remote_message_id)
}

fn audit_event_matches(
    existing: &AuditEventRecord,
    expected: &AuditEventInput,
    repository_id: &str,
    inbound_item_id: &str,
) -> bool {
    existing.repository_id == repository_id
        && existing.event_id == expected.event_id
        && existing.object_type == expected.object_type
        && existing.object_id == inbound_item_id
        && existing.transition == expected.transition
        && existing.occurred_at == expected.occurred_at
        && existing.actor_kind == expected.actor_kind
        && existing.outcome == expected.outcome
        && existing.metadata_json == expected.metadata_json
}

fn accepted_reply_audit_event_id(draft_id: &str) -> String {
    let digest = Sha256::digest(draft_id.as_bytes());
    let hash = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("inbound-replied-{hash}")
}
