//! Reply-draft command port.
//!
//! Reply target authorization and immutable draft creation remain owned by the
//! reply domain service.  This module only validates the explicit request and
//! routes it.

use repo_com_config::ResolvedConfig;
use repo_com_draft_model::{AuthorizedReplyReference, DraftMetadata};
use repo_com_foundation::RepoComError;
use repo_com_reply::{
    CreatedReply, ReplyDraftRequest as DomainReplyDraftRequest, ReplyServiceError,
};
use serde::{Deserialize, Serialize};

use crate::MessagingResult;
use crate::input::ReplyDraftCreateInput;

/// A validated reply-draft creation request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplyCreateRequest {
    /// Repository scope.
    pub repository_id: String,
    /// Stored inbound item used as the target lookup key.
    pub inbound_item_id: String,
    /// New reply draft identity.
    pub draft_id: String,
    /// Reply body.
    pub text: String,
    /// Exact event type.
    pub event_type: String,
    /// Exact severity.
    pub severity: String,
    /// Bounded metadata.
    pub metadata: DraftMetadata,
    /// Canonical UTC creation timestamp.
    pub created_at: String,
    /// Unix creation time.
    pub created_at_unix_seconds: u64,
    /// Optional expiry lifetime.
    pub expires_in_seconds: Option<u64>,
}

impl TryFrom<ReplyDraftCreateInput> for ReplyCreateRequest {
    type Error = RepoComError;

    fn try_from(value: ReplyDraftCreateInput) -> Result<Self, Self::Error> {
        Ok(Self {
            repository_id: value.repository_id,
            inbound_item_id: value.inbound_item_id,
            draft_id: value.draft_id,
            text: value.text,
            event_type: value.event_type,
            severity: value.severity,
            metadata: value.metadata.to_domain()?,
            created_at: value.created_at,
            created_at_unix_seconds: value.created_at_unix_seconds,
            expires_in_seconds: value.expires_in_seconds,
        })
    }
}

impl ReplyCreateRequest {
    /// Converts this validated command into the domain request.  The target is
    /// intentionally absent: the reply service resolves and authorizes it.
    #[must_use]
    pub fn to_domain(&self) -> DomainReplyDraftRequest {
        let request = DomainReplyDraftRequest::new(
            self.repository_id.clone(),
            self.inbound_item_id.clone(),
            self.draft_id.clone(),
            self.text.clone(),
            self.event_type.clone(),
            self.severity.clone(),
            self.created_at.clone(),
            self.created_at_unix_seconds,
        )
        .with_metadata(self.metadata.clone());
        match self.expires_in_seconds {
            Some(seconds) => request.with_expiry_seconds(seconds),
            None => request,
        }
    }
}

/// Safe projection of the local result of reply-draft creation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReplyDraftView {
    /// Repository scope.
    pub repository_id: String,
    /// Inbound item linked to this draft.
    pub inbound_item_id: String,
    /// New draft identity.
    pub draft_id: String,
    /// Immutable revision number.
    pub revision: u64,
    /// Canonical revision hash.
    pub revision_hash: String,
    /// Configured destination selected by the authorized target.
    pub destination_alias: String,
    /// Resolved destination snapshot.
    pub resolved_destination: repo_com_config::ResolvedDestination,
    /// Validated Discord message reference, if present.
    pub message_reference: Option<AuthorizedReplyReference>,
    /// Draft creation time.
    pub created_at_unix_seconds: u64,
    /// Exclusive draft expiry.
    pub expires_at_unix_seconds: u64,
    /// Reply creation is always draft-only.
    pub draft_only: bool,
}

impl ReplyDraftView {
    /// Projects a domain-created reply without retaining untrusted content.
    #[must_use]
    pub fn from_created(created: &CreatedReply) -> Self {
        let draft = created.draft();
        let revision = draft.current_revision();
        Self {
            repository_id: created.target().repository_id().to_owned(),
            inbound_item_id: created.target().inbound_item_id().to_owned(),
            draft_id: draft.draft_id().as_str().to_owned(),
            revision: revision.number(),
            revision_hash: revision.content_hash().to_owned(),
            destination_alias: revision.destination_alias().as_str().to_owned(),
            resolved_destination: revision.resolved_destination().clone(),
            message_reference: revision.reply_reference().cloned(),
            created_at_unix_seconds: revision.expiry().created_at_unix_seconds(),
            expires_at_unix_seconds: revision.expiry().expires_at_unix_seconds(),
            draft_only: true,
        }
    }
}

/// Domain port that retains target authorization and draft-only creation.
#[allow(async_fn_in_trait)]
pub trait ReplyService {
    /// Creates and links one validated reply draft.
    async fn create_reply(
        &mut self,
        request: ReplyCreateRequest,
        config: &ResolvedConfig,
    ) -> MessagingResult<ReplyDraftView>;
}

/// Routes reply-draft creation through the domain owner.
pub async fn create<S>(
    service: &mut S,
    request: ReplyCreateRequest,
    config: &ResolvedConfig,
) -> MessagingResult<ReplyDraftView>
where
    S: ReplyService + ?Sized,
{
    if request.repository_id != config.config.repository_id {
        return Err(RepoComError::usage(
            "reply repository does not match the resolved configuration",
        ));
    }
    service.create_reply(request, config).await
}

/// Converts the domain's stable service error into a foundation error while
/// keeping the typed domain cause available to the composed binary.
#[must_use]
pub fn map_reply_error(error: &ReplyServiceError) -> RepoComError {
    match error {
        ReplyServiceError::Target(target) => match target {
            repo_com_reply::ReplyTargetError::AuthorizationDenied
            | repo_com_reply::ReplyTargetError::ChannelNotConfigured
            | repo_com_reply::ReplyTargetError::WorkspaceMismatch
            | repo_com_reply::ReplyTargetError::RepositoryMismatch
            | repo_com_reply::ReplyTargetError::TargetExpired
            | repo_com_reply::ReplyTargetError::TargetDeleted
            | repo_com_reply::ReplyTargetError::TargetChanged
            | repo_com_reply::ReplyTargetError::EligibilityBlocked(_)
            | repo_com_reply::ReplyTargetError::EligibilityFactsChanged => {
                RepoComError::policy_blocked("reply target is not currently authorized")
            }
            _ => RepoComError::usage("reply target request is invalid"),
        },
        ReplyServiceError::Draft(_) => RepoComError::usage("reply draft request is invalid"),
        ReplyServiceError::InvalidTimestamp => {
            RepoComError::usage("reply creation timestamp is invalid")
        }
        ReplyServiceError::InvalidAcceptedState
        | ReplyServiceError::MissingAcceptedRemoteMessage
        | ReplyServiceError::ReplyLinkMissing
        | ReplyServiceError::ReplyLinkMismatch
        | ReplyServiceError::AuditConflict
        | ReplyServiceError::State(_) => {
            RepoComError::storage_integrity("reply local state operation failed")
        }
        ReplyServiceError::Serialization(_) => {
            RepoComError::internal_failure("reply evidence could not be serialized")
        }
    }
}
