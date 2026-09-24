use repo_com_config::ResolvedDestination;
use serde::Serialize;

use crate::model::{
    AuthorizedReplyReference, DestinationAlias, DraftBody, DraftError, DraftExpiry, DraftMetadata,
    DraftModel, DraftRevision, EventType, Severity,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum BasisAuthority {
    HumanApproval,
    ActivatedPolicy,
    SecretScanner,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum BasisStatus {
    Unresolved { reason: UnresolvedBasisReason },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum UnresolvedBasisReason {
    NotEvaluatedByDraftModel,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct BasisFact {
    authority: BasisAuthority,
    status: BasisStatus,
}

impl BasisFact {
    #[must_use]
    pub const fn unresolved(authority: BasisAuthority) -> Self {
        Self {
            authority,
            status: BasisStatus::Unresolved {
                reason: UnresolvedBasisReason::NotEvaluatedByDraftModel,
            },
        }
    }

    #[must_use]
    pub const fn authority(&self) -> BasisAuthority {
        self.authority
    }

    #[must_use]
    pub const fn status(&self) -> BasisStatus {
        self.status
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct DecisionBases {
    approval: BasisFact,
    policy: BasisFact,
    safety: BasisFact,
}

impl DecisionBases {
    #[must_use]
    pub const fn unresolved() -> Self {
        Self {
            approval: BasisFact::unresolved(BasisAuthority::HumanApproval),
            policy: BasisFact::unresolved(BasisAuthority::ActivatedPolicy),
            safety: BasisFact::unresolved(BasisAuthority::SecretScanner),
        }
    }

    #[must_use]
    pub const fn approval(&self) -> BasisFact {
        self.approval
    }

    #[must_use]
    pub const fn policy(&self) -> BasisFact {
        self.policy
    }

    #[must_use]
    pub const fn safety(&self) -> BasisFact {
        self.safety
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum ExactTextSource {
    StoredChannelText,
    CallerRenderedText,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum SendBlocker {
    RevisionExpired,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub enum SendDecision {
    Unresolved { blockers: Vec<SendBlocker> },
    Blocked { blocker: SendBlocker },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DraftPreview {
    draft_id: String,
    revision: u64,
    repository_id: String,
    content_hash: String,
    destination_alias: DestinationAlias,
    resolved_destination: ResolvedDestination,
    exact_text: String,
    text_source: ExactTextSource,
    metadata: DraftMetadata,
    event_type: EventType,
    severity: Severity,
    reply_reference: Option<AuthorizedReplyReference>,
    expiry: DraftExpiry,
    bases: DecisionBases,
    send_decision: SendDecision,
}

impl DraftPreview {
    #[must_use]
    pub fn draft_id(&self) -> &str {
        &self.draft_id
    }

    #[must_use]
    pub fn revision(&self) -> u64 {
        self.revision
    }

    #[must_use]
    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }

    #[must_use]
    pub fn content_hash(&self) -> &str {
        &self.content_hash
    }

    #[must_use]
    pub fn destination_alias(&self) -> &DestinationAlias {
        &self.destination_alias
    }

    #[must_use]
    pub fn resolved_destination(&self) -> &ResolvedDestination {
        &self.resolved_destination
    }

    #[must_use]
    pub fn exact_text(&self) -> &str {
        &self.exact_text
    }

    #[must_use]
    pub fn text_source(&self) -> ExactTextSource {
        self.text_source
    }

    #[must_use]
    pub fn metadata(&self) -> &DraftMetadata {
        &self.metadata
    }

    #[must_use]
    pub fn event_type(&self) -> &EventType {
        &self.event_type
    }

    #[must_use]
    pub fn severity(&self) -> &Severity {
        &self.severity
    }

    #[must_use]
    pub fn reply_reference(&self) -> Option<&AuthorizedReplyReference> {
        self.reply_reference.as_ref()
    }

    #[must_use]
    pub fn expiry(&self) -> DraftExpiry {
        self.expiry
    }

    #[must_use]
    pub fn bases(&self) -> DecisionBases {
        self.bases
    }

    #[must_use]
    pub fn send_decision(&self) -> &SendDecision {
        &self.send_decision
    }
}

impl DraftModel {
    pub fn preview(&self, revision: u64, at_unix_seconds: u64) -> Result<DraftPreview, DraftError> {
        let snapshot = self
            .revision(revision)
            .ok_or_else(|| DraftError::RevisionNotFound {
                draft_id: self.draft_id().as_str().to_owned(),
                revision,
            })?;
        Self::preview_snapshot(
            self,
            snapshot,
            snapshot.exact_text().to_owned(),
            ExactTextSource::StoredChannelText,
            at_unix_seconds,
        )
    }

    pub fn preview_with_rendered_text(
        &self,
        revision: u64,
        exact_text: impl Into<String>,
        at_unix_seconds: u64,
    ) -> Result<DraftPreview, DraftError> {
        let snapshot = self
            .revision(revision)
            .ok_or_else(|| DraftError::RevisionNotFound {
                draft_id: self.draft_id().as_str().to_owned(),
                revision,
            })?;
        let exact_text = DraftBody::new(exact_text)?.into_string();
        Self::preview_snapshot(
            self,
            snapshot,
            exact_text,
            ExactTextSource::CallerRenderedText,
            at_unix_seconds,
        )
    }

    fn preview_snapshot(
        owner: &DraftModel,
        snapshot: &DraftRevision,
        exact_text: String,
        text_source: ExactTextSource,
        at_unix_seconds: u64,
    ) -> Result<DraftPreview, DraftError> {
        let send_decision = if snapshot.is_expired(at_unix_seconds) {
            SendDecision::Blocked {
                blocker: SendBlocker::RevisionExpired,
            }
        } else {
            SendDecision::Unresolved {
                blockers: Vec::new(),
            }
        };
        Ok(DraftPreview {
            draft_id: owner.draft_id().as_str().to_owned(),
            revision: snapshot.number(),
            repository_id: snapshot.repository_id().to_owned(),
            content_hash: snapshot.content_hash().to_owned(),
            destination_alias: snapshot.destination_alias().clone(),
            resolved_destination: snapshot.resolved_destination().clone(),
            exact_text,
            text_source,
            metadata: snapshot.metadata().clone(),
            event_type: snapshot.event_type().clone(),
            severity: snapshot.severity().clone(),
            reply_reference: snapshot.reply_reference().cloned(),
            expiry: snapshot.expiry(),
            bases: DecisionBases::unresolved(),
            send_decision,
        })
    }
}
