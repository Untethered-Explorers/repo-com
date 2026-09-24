//! Creation of one normal immutable draft revision for a validated target.

use std::fmt;

use repo_com_config::ResolvedConfig;
use repo_com_draft_model::{AuthorizedReplyReference, DraftMetadata, DraftModel, DraftRequest};
use repo_com_inbox_state::{InboxState, ReplyLinkRecord, StateStore};
use repo_com_state::{
    AuditEventInput, AuditEventRecord, DraftInput, DraftRevisionInput, ReplyLinkInput, StateError,
};
use serde::Serialize;

use crate::{
    ReplyServiceError, ReplyTarget, ReplyTargetError, ReplyTargetRequest, ReplyTargetValidator,
    target::{format_rfc3339_utc, parse_canonical_utc},
};

/// Skill-supplied content for a reply. The target and destination are not
/// caller-controlled: both come from a freshly validated local target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplyDraftRequest {
    repository_id: String,
    inbound_item_id: String,
    draft_id: String,
    text: String,
    event_type: String,
    severity: String,
    metadata: DraftMetadata,
    expires_in_seconds: Option<u64>,
    created_at: String,
    created_at_unix_seconds: u64,
}

impl ReplyDraftRequest {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        repository_id: impl Into<String>,
        inbound_item_id: impl Into<String>,
        draft_id: impl Into<String>,
        text: impl Into<String>,
        event_type: impl Into<String>,
        severity: impl Into<String>,
        created_at: impl Into<String>,
        created_at_unix_seconds: u64,
    ) -> Self {
        Self {
            repository_id: repository_id.into(),
            inbound_item_id: inbound_item_id.into(),
            draft_id: draft_id.into(),
            text: text.into(),
            event_type: event_type.into(),
            severity: severity.into(),
            metadata: DraftMetadata::default(),
            expires_in_seconds: None,
            created_at: created_at.into(),
            created_at_unix_seconds,
        }
    }

    #[must_use]
    pub fn with_metadata(mut self, metadata: DraftMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    #[must_use]
    pub const fn with_expiry_seconds(mut self, seconds: u64) -> Self {
        self.expires_in_seconds = Some(seconds);
        self
    }

    #[must_use]
    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }

    #[must_use]
    pub fn inbound_item_id(&self) -> &str {
        &self.inbound_item_id
    }

    #[must_use]
    pub fn draft_id(&self) -> &str {
        &self.draft_id
    }

    #[must_use]
    pub fn created_at_unix_seconds(&self) -> u64 {
        self.created_at_unix_seconds
    }
}

/// Stateless factory for the ordinary draft model with one reply reference.
#[derive(Clone, Copy, Debug, Default)]
pub struct ReplyDraftFactory;

impl ReplyDraftFactory {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Creates exactly revision 1. Approval, policy, safety, rendering,
    /// eligibility, and delivery remain downstream state-machine operations.
    pub fn create(
        &self,
        target: &ReplyTarget,
        request: ReplyDraftRequest,
        config: &ResolvedConfig,
    ) -> Result<DraftModel, ReplyServiceError> {
        if parse_canonical_utc(&request.created_at) != Some(request.created_at_unix_seconds) {
            return Err(ReplyServiceError::InvalidTimestamp);
        }
        target.ensure_usable_for_draft(config, request.created_at_unix_seconds)?;

        let reference = AuthorizedReplyReference::from_validated_target(
            target.repository_id(),
            target.workspace_id(),
            target.channel_id(),
            target.inbound_item_id(),
            target.inbound_item_id(),
            target.authorization_reference(),
            target.validated_snapshot_hash(),
        )?;
        let mut draft_request = DraftRequest::new(
            &request.draft_id,
            target.destination_alias(),
            request.text,
            request.event_type,
            request.severity,
        )?
        .with_metadata(request.metadata)
        .with_reply_reference(reference);
        if let Some(seconds) = request.expires_in_seconds {
            draft_request = draft_request.with_expiry_seconds(seconds);
        }
        let draft = DraftModel::create(draft_request, config, request.created_at_unix_seconds)?;
        let revision = draft.current_revision();
        if revision.reply_reference().is_none() || revision.number() != 1 {
            return Err(ReplyServiceError::Target(ReplyTargetError::NotReplyDraft));
        }
        Ok(draft)
    }
}

/// Local command service that validates, creates, persists, and links a reply
/// draft. It has no Discord transport and no approval, policy, safety, scan, or
/// delivery-claim method.
pub struct ReplyCommandService {
    inbox: InboxState,
}

impl fmt::Debug for ReplyCommandService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReplyCommandService")
            .field("inbox", &self.inbox)
            .field("remote_send", &false)
            .finish()
    }
}

impl ReplyCommandService {
    #[must_use]
    pub const fn new(inbox: InboxState) -> Self {
        Self { inbox }
    }

    #[must_use]
    pub const fn from_state(state: StateStore) -> Self {
        Self::new(InboxState::new(state))
    }

    #[must_use]
    pub const fn state(&self) -> &InboxState {
        &self.inbox
    }

    pub const fn state_mut(&mut self) -> &mut InboxState {
        &mut self.inbox
    }

    #[must_use]
    pub fn into_state(self) -> InboxState {
        self.inbox
    }

    #[must_use]
    pub const fn creates_draft_only(&self) -> bool {
        true
    }

    #[must_use]
    pub const fn allows_remote_send(&self) -> bool {
        false
    }

    /// Creates and links one immutable draft without creating a delivery claim.
    pub fn create_reply(
        &mut self,
        config: &ResolvedConfig,
        request: ReplyDraftRequest,
    ) -> Result<CreatedReply, ReplyServiceError> {
        let target_request = ReplyTargetRequest::new(
            &request.repository_id,
            &request.inbound_item_id,
            request.created_at_unix_seconds,
        )?;
        let target = ReplyTargetValidator::new().validate(&self.inbox, config, &target_request)?;
        let draft = ReplyDraftFactory::new().create(&target, request, config)?;
        persist_reply_draft(&mut self.inbox, &target, draft.clone(), config)
    }
}

fn persist_reply_draft(
    inbox: &mut InboxState,
    target: &ReplyTarget,
    draft: DraftModel,
    config: &ResolvedConfig,
) -> Result<CreatedReply, ReplyServiceError> {
    let revision = draft.current_revision();
    let expiry_at = format_rfc3339_utc(revision.expiry().expires_at_unix_seconds())
        .ok_or(ReplyServiceError::InvalidTimestamp)?;
    let metadata_json = serde_json::to_string(revision.metadata())?;
    let resolved_destination = serde_json::to_string(revision.resolved_destination())?;
    let created_at = format_rfc3339_utc(revision.expiry().created_at_unix_seconds())
        .ok_or(ReplyServiceError::InvalidTimestamp)?;
    let draft_input = DraftInput {
        repository_id: target.repository_id().to_owned(),
        draft_id: draft.draft_id().as_str().to_owned(),
        event_type: revision.event_type().as_str().to_owned(),
        destination_alias: target.destination_alias().to_owned(),
        status: "draft".to_owned(),
        expiry_at: Some(expiry_at.clone()),
        reply_to_inbound_item_id: Some(target.inbound_item_id().to_owned()),
        metadata_json: metadata_json.clone(),
        created_at: created_at.clone(),
        updated_at: created_at.clone(),
    };
    let revision_input = DraftRevisionInput {
        repository_id: target.repository_id().to_owned(),
        draft_id: draft.draft_id().as_str().to_owned(),
        revision: i64::try_from(revision.number())
            .map_err(|_| ReplyServiceError::InvalidTimestamp)?,
        content_hash: revision.content_hash().to_owned(),
        body: revision.exact_text().to_owned(),
        metadata_json,
        destination_alias: target.destination_alias().to_owned(),
        resolved_destination,
        expiry_at: Some(expiry_at),
        lifecycle_state: "draft".to_owned(),
        reply_to_inbound_item_id: Some(target.inbound_item_id().to_owned()),
        created_at: created_at.clone(),
    };
    let link_input = ReplyLinkInput::new(
        target.repository_id(),
        target.inbound_item_id(),
        draft.draft_id().as_str(),
        created_at.clone(),
    );
    let link_audit = ReplyLinkAuditInput {
        schema_version: 1,
        repository_id: target.repository_id(),
        workspace_id: target.workspace_id(),
        destination_alias: target.destination_alias(),
        inbound_item_id: target.inbound_item_id(),
        target_message_id: target.inbound_item_id(),
        draft_id: draft.draft_id().as_str(),
        revision: revision.number(),
        revision_hash: revision.content_hash(),
        authorization_reference: target.authorization_reference(),
        validated_snapshot_hash: target.validated_snapshot_hash(),
        human_message_delivery_claimed: false,
    };
    let audit_input = AuditEventInput {
        repository_id: target.repository_id().to_owned(),
        event_id: reply_link_audit_event_id(revision.content_hash()),
        object_type: "inbound_item".to_owned(),
        object_id: target.inbound_item_id().to_owned(),
        transition: "reply_draft_linked".to_owned(),
        occurred_at: created_at,
        actor_kind: "agent".to_owned(),
        outcome: "draft_created".to_owned(),
        metadata_json: serde_json::to_string(&link_audit)?,
    };
    let config_hash = config.canonical_hash();

    let (link, audit) = inbox.state_mut().with_transaction(|transaction| {
        let repository = transaction
            .repositories()
            .repositories()
            .get(target.repository_id())?
            .ok_or_else(|| StateError::NotFound {
                entity: "reply repository",
                repository_id: target.repository_id().to_owned(),
                object_id: target.repository_id().to_owned(),
            })?;
        if repository.workspace_id != target.workspace_id() || repository.config_hash != config_hash
        {
            return Err(StateError::Constraint {
                entity: "reply target authorization",
                message: "repository authorization changed before draft commit".to_owned(),
            });
        }
        let item = transaction
            .repositories()
            .inbound()
            .item(target.repository_id(), target.inbound_item_id())?
            .ok_or_else(|| StateError::NotFound {
                entity: "reply inbound item",
                repository_id: target.repository_id().to_owned(),
                object_id: target.inbound_item_id().to_owned(),
            })?;
        let current = transaction
            .repositories()
            .inbound()
            .current(target.repository_id(), target.inbound_item_id())?
            .ok_or_else(|| StateError::NotFound {
                entity: "reply current snapshot",
                repository_id: target.repository_id().to_owned(),
                object_id: target.inbound_item_id().to_owned(),
            })?;
        target
            .ensure_current_records(&item, &current)
            .map_err(|error| StateError::Constraint {
                entity: "reply target revalidation",
                message: error.code().to_owned(),
            })?;

        transaction.repositories().drafts().create(&draft_input)?;
        transaction
            .repositories()
            .drafts()
            .insert_revision(&revision_input)?;
        let link = transaction
            .repositories()
            .inbound()
            .link_reply(&link_input)?;
        let audit = transaction.append_audit_event(&audit_input)?;
        Ok((link, audit))
    })?;

    Ok(CreatedReply {
        draft,
        target: target.clone(),
        link,
        link_audit: audit,
    })
}

pub(crate) fn reply_link_audit_event_id(revision_hash: &str) -> String {
    format!("reply-linked-{revision_hash}")
}

#[derive(Serialize)]
struct ReplyLinkAuditInput<'a> {
    schema_version: u8,
    repository_id: &'a str,
    workspace_id: &'a str,
    destination_alias: &'a str,
    inbound_item_id: &'a str,
    target_message_id: &'a str,
    draft_id: &'a str,
    revision: u64,
    revision_hash: &'a str,
    authorization_reference: &'a str,
    validated_snapshot_hash: &'a str,
    human_message_delivery_claimed: bool,
}

/// The complete local result of creating one reply draft.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreatedReply {
    draft: DraftModel,
    target: ReplyTarget,
    link: ReplyLinkRecord,
    link_audit: AuditEventRecord,
}

impl CreatedReply {
    #[must_use]
    pub fn draft(&self) -> &DraftModel {
        &self.draft
    }

    #[must_use]
    pub fn target(&self) -> &ReplyTarget {
        &self.target
    }

    #[must_use]
    pub fn link(&self) -> &ReplyLinkRecord {
        &self.link
    }

    #[must_use]
    pub fn link_audit(&self) -> &AuditEventRecord {
        &self.link_audit
    }

    #[must_use]
    pub fn into_draft(self) -> DraftModel {
        self.draft
    }
}
