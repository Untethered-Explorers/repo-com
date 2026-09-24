//! Draft and approval command ports.
//!
//! These functions validate command identities, route work to the draft and
//! approval owners, and adapt the non-authoritative outbound keyboard result
//! into a domain confirmation.  They do not decide approval, scan safety, or
//! draft content policy.

use repo_com_approval::{ApprovalRecord, OverrideReasonCode, SecretOverrideRecord};
use repo_com_draft_model::{AuthorizedReplyReference, DraftMetadata, DraftRevision};
use repo_com_foundation::{RepoComError, TtyMode};
use repo_com_terminal_outbound::{
    ApprovalStatusView, ApprovalView, ExactConfirmation, ExactPreviewIdentity, KeyboardPrompt,
    OutboundPreview, PromptAction, PromptInput, PromptResult, SafetyState, SafetyView,
    SecretFindingStatusView,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::MessagingResult;
use crate::input::{DraftApprovalInput, DraftCreateInput, DraftIdentityInput, DraftUpdateInput};

/// Exact identity of one immutable draft revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DraftIdentity {
    /// Repository scope.
    pub repository_id: String,
    /// Draft identity.
    pub draft_id: String,
    /// Immutable revision number.
    pub revision: u64,
}

/// Validated create-draft command passed to a domain owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateDraft {
    /// Repository scope.
    pub repository_id: String,
    /// New draft identity.
    pub draft_id: String,
    /// Named configured destination.
    pub destination_alias: String,
    /// Draft body.
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

/// Validated immutable-revision update command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateDraft {
    /// Exact draft and current revision identity.
    pub identity: DraftIdentity,
    /// Named configured destination for the new revision.
    pub destination_alias: String,
    /// New body.
    pub text: String,
    /// Exact event type.
    pub event_type: String,
    /// Exact severity.
    pub severity: String,
    /// Bounded metadata.
    pub metadata: DraftMetadata,
    /// Canonical UTC creation timestamp for the new revision.
    pub created_at: String,
    /// Unix creation time for the new revision.
    pub created_at_unix_seconds: u64,
    /// Optional expiry lifetime.
    pub expires_in_seconds: Option<u64>,
}

impl TryFrom<DraftCreateInput> for CreateDraft {
    type Error = RepoComError;

    fn try_from(value: DraftCreateInput) -> Result<Self, Self::Error> {
        Ok(Self {
            repository_id: value.repository_id,
            draft_id: value.draft_id,
            destination_alias: value.destination_alias,
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

impl TryFrom<DraftUpdateInput> for UpdateDraft {
    type Error = RepoComError;

    fn try_from(value: DraftUpdateInput) -> Result<Self, Self::Error> {
        Ok(Self {
            identity: DraftIdentity {
                repository_id: value.repository_id,
                draft_id: value.draft_id,
                revision: value.revision,
            },
            destination_alias: value.destination_alias,
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

impl From<DraftIdentityInput> for DraftIdentity {
    fn from(value: DraftIdentityInput) -> Self {
        Self {
            repository_id: value.repository_id,
            draft_id: value.draft_id,
            revision: value.revision,
        }
    }
}

impl From<DraftApprovalInput> for DraftIdentity {
    fn from(value: DraftApprovalInput) -> Self {
        Self {
            repository_id: value.repository_id,
            draft_id: value.draft_id,
            revision: value.revision,
        }
    }
}

/// A complete safe projection of one stored draft revision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DraftView {
    /// Repository scope.
    pub repository_id: String,
    /// Draft identity.
    pub draft_id: String,
    /// Immutable revision number.
    pub revision: u64,
    /// Canonical immutable revision hash.
    pub revision_hash: String,
    /// Named configured destination.
    pub destination_alias: String,
    /// Resolved destination snapshot.
    pub resolved_destination: repo_com_config::ResolvedDestination,
    /// Exact text that would be rendered or stored.
    pub exact_text: String,
    /// Hash of exact text.
    pub exact_text_hash: String,
    /// Bounded metadata.
    pub metadata: DraftMetadata,
    /// Exact event type.
    pub event_type: String,
    /// Exact severity.
    pub severity: String,
    /// Optional validated reply reference.
    pub reply_reference: Option<AuthorizedReplyReference>,
    /// Draft creation time.
    pub created_at_unix_seconds: u64,
    /// Exclusive expiry boundary.
    pub expires_at_unix_seconds: u64,
}

impl DraftView {
    /// Projects a domain revision and its exact rendered/stored text.
    #[must_use]
    pub fn from_revision(
        draft_id: impl Into<String>,
        revision: &DraftRevision,
        exact_text: impl Into<String>,
    ) -> Self {
        let exact_text = exact_text.into();
        let exact_text_hash = sha256_text(&exact_text);
        Self {
            repository_id: revision.repository_id().to_owned(),
            draft_id: draft_id.into(),
            revision: revision.number(),
            revision_hash: revision.content_hash().to_owned(),
            destination_alias: revision.destination_alias().as_str().to_owned(),
            resolved_destination: revision.resolved_destination().clone(),
            exact_text,
            exact_text_hash,
            metadata: revision.metadata().clone(),
            event_type: revision.event_type().as_str().to_owned(),
            severity: revision.severity().as_str().to_owned(),
            reply_reference: revision.reply_reference().cloned(),
            created_at_unix_seconds: revision.expiry().created_at_unix_seconds(),
            expires_at_unix_seconds: revision.expiry().expires_at_unix_seconds(),
        }
    }

    /// Creates a projection for callers that already have the exact fields.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        repository_id: impl Into<String>,
        draft_id: impl Into<String>,
        revision: u64,
        revision_hash: impl Into<String>,
        destination_alias: impl Into<String>,
        resolved_destination: repo_com_config::ResolvedDestination,
        exact_text: impl Into<String>,
        metadata: DraftMetadata,
        event_type: impl Into<String>,
        severity: impl Into<String>,
        created_at_unix_seconds: u64,
        expires_at_unix_seconds: u64,
    ) -> Self {
        let exact_text = exact_text.into();
        Self {
            repository_id: repository_id.into(),
            draft_id: draft_id.into(),
            revision,
            revision_hash: revision_hash.into(),
            destination_alias: destination_alias.into(),
            resolved_destination,
            exact_text_hash: sha256_text(&exact_text),
            exact_text,
            metadata,
            event_type: event_type.into(),
            severity: severity.into(),
            reply_reference: None,
            created_at_unix_seconds,
            expires_at_unix_seconds,
        }
    }

    /// Projects this view into the outbound presentation model without
    /// evaluating approval, policy, or safety.
    #[must_use]
    pub fn to_outbound_preview(&self) -> OutboundPreview {
        OutboundPreview {
            repository_id: self.repository_id.clone(),
            draft_id: self.draft_id.clone(),
            revision: self.revision,
            revision_hash: self.revision_hash.clone(),
            destination: self.resolved_destination.clone().into(),
            exact_text: self.exact_text.clone(),
            exact_text_hash: self.exact_text_hash.clone(),
            metadata: self.metadata.clone().into(),
            event_type: self.event_type.clone(),
            severity: self.severity.clone(),
            reply_reference: self.reply_reference.as_ref().map(Into::into),
            created_at_unix_seconds: self.created_at_unix_seconds,
            expires_at_unix_seconds: self.expires_at_unix_seconds,
            approval: ApprovalView::default(),
            policy: Default::default(),
            safety: SafetyView {
                state: SafetyState::NotEvaluated,
                ..SafetyView::default()
            },
            preview_hash: String::new(),
            provenance: repo_com_terminal_outbound::Provenance::LocalDecision,
        }
    }
}

/// Domain port for draft CRUD and exact preview operations.
#[allow(async_fn_in_trait)]
pub trait DraftService {
    /// Creates one new draft through the domain owner.
    async fn create(&mut self, request: CreateDraft) -> MessagingResult<DraftView>;

    /// Reads one exact immutable revision.
    async fn show(&mut self, identity: DraftIdentity) -> MessagingResult<DraftView>;

    /// Creates a new immutable revision through the domain owner.
    async fn update(&mut self, request: UpdateDraft) -> MessagingResult<DraftView>;

    /// Builds a side-effect-free exact preview.
    async fn preview(
        &mut self,
        identity: DraftIdentity,
        now_unix_seconds: u64,
    ) -> MessagingResult<OutboundPreview>;

    /// Builds the complete approval preview without creating authority.
    async fn approval_preview(
        &mut self,
        identity: DraftIdentity,
        now_unix_seconds: u64,
    ) -> MessagingResult<OutboundPreview>;

    /// Records an exact approval after the UI confirmation.  The service must
    /// revalidate the current preview and consume the confirmation itself.
    async fn approve(
        &mut self,
        identity: DraftIdentity,
        confirmation: ExactConfirmation,
        tty_mode: TtyMode,
        now_unix_seconds: u64,
    ) -> MessagingResult<ApprovalRecord>;

    /// Records an exact redacted safety override after the UI confirmation.
    async fn override_secret_finding(
        &mut self,
        identity: DraftIdentity,
        confirmation: ExactConfirmation,
        tty_mode: TtyMode,
        reason: OverrideReasonCode,
        now_unix_seconds: u64,
    ) -> MessagingResult<SecretOverrideRecord>;
}

/// Non-authoritative outbound UI boundary used by approval and override.
pub trait OutboundUi {
    /// Requests an exact approval confirmation.
    fn request_approval(&mut self, preview: OutboundPreview) -> PromptResult;

    /// Requests an exact secret-finding override confirmation.
    fn request_secret_override(&mut self, preview: OutboundPreview) -> PromptResult;
}

/// Production outbound UI adapter over the repository's keyboard prompt.
#[derive(Clone, Debug)]
pub struct KeyboardOutboundUi<I> {
    prompt: KeyboardPrompt<I>,
}

impl<I> KeyboardOutboundUi<I> {
    /// Creates an adapter with an explicit input source, stream mode, and
    /// deterministic time source.
    #[must_use]
    pub const fn new(input: I, tty_mode: TtyMode, now_unix_seconds: u64) -> Self {
        Self {
            prompt: KeyboardPrompt::new(input, tty_mode, now_unix_seconds),
        }
    }

    /// Returns the underlying prompt adapter for read-only inspection.
    #[must_use]
    pub const fn prompt(&self) -> &KeyboardPrompt<I> {
        &self.prompt
    }

    /// Consumes this adapter and returns the prompt.
    #[must_use]
    pub fn into_prompt(self) -> KeyboardPrompt<I> {
        self.prompt
    }
}

impl<I: PromptInput> OutboundUi for KeyboardOutboundUi<I> {
    fn request_approval(&mut self, preview: OutboundPreview) -> PromptResult {
        self.prompt.request_exact_approval(preview)
    }

    fn request_secret_override(&mut self, preview: OutboundPreview) -> PromptResult {
        self.prompt.request_secret_override(preview)
    }
}

/// Creates a draft through the supplied domain port.
pub async fn create<S>(service: &mut S, request: CreateDraft) -> MessagingResult<DraftView>
where
    S: DraftService + ?Sized,
{
    service.create(request).await
}

/// Shows one exact revision through the supplied domain port.
pub async fn show<S>(service: &mut S, identity: DraftIdentity) -> MessagingResult<DraftView>
where
    S: DraftService + ?Sized,
{
    service.show(identity).await
}

/// Updates a draft through the supplied domain port.
pub async fn update<S>(service: &mut S, request: UpdateDraft) -> MessagingResult<DraftView>
where
    S: DraftService + ?Sized,
{
    service.update(request).await
}

/// Builds a preview through the supplied domain port.
pub async fn preview<S>(
    service: &mut S,
    identity: DraftIdentity,
    now_unix_seconds: u64,
) -> MessagingResult<OutboundPreview>
where
    S: DraftService + ?Sized,
{
    service.preview(identity, now_unix_seconds).await
}

/// Routes exact approval through the outbound UI and then the domain owner.
pub async fn approve<S, U>(
    service: &mut S,
    ui: &mut U,
    identity: DraftIdentity,
    tty_mode: TtyMode,
    now_unix_seconds: u64,
) -> MessagingResult<ApprovalStatusView>
where
    S: DraftService + ?Sized,
    U: OutboundUi + ?Sized,
{
    tty_mode.require_prompt_allowed()?;
    let preview = service
        .approval_preview(identity.clone(), now_unix_seconds)
        .await?;
    let confirmation =
        require_confirmation(ui.request_approval(preview.clone()), PromptAction::Approve)?;
    let expected = ExactPreviewIdentity::from_preview(&preview);
    if !confirmation.matches(&expected) {
        return Err(RepoComError::usage(
            "approval confirmation did not match the complete exact preview",
        ));
    }
    let record = service
        .approve(identity, confirmation, tty_mode, now_unix_seconds)
        .await?;
    Ok(approval_status_view(&preview, record))
}

/// Routes exact secret-finding override through the outbound UI and then the
/// approval owner.
pub async fn override_secret_finding<S, U>(
    service: &mut S,
    ui: &mut U,
    identity: DraftIdentity,
    tty_mode: TtyMode,
    now_unix_seconds: u64,
) -> MessagingResult<SecretFindingStatusView>
where
    S: DraftService + ?Sized,
    U: OutboundUi + ?Sized,
{
    tty_mode.require_prompt_allowed()?;
    let preview = service
        .approval_preview(identity.clone(), now_unix_seconds)
        .await?;
    if preview.safety.state == SafetyState::Clear {
        return Err(RepoComError::policy_blocked(
            "the exact preview has no secret finding requiring an override",
        ));
    }
    let confirmation = require_confirmation(
        ui.request_secret_override(preview.clone()),
        PromptAction::OverrideSecretFinding,
    )?;
    let expected = ExactPreviewIdentity::from_preview(&preview);
    if !confirmation.matches(&expected) {
        return Err(RepoComError::usage(
            "secret override confirmation did not match the complete exact preview",
        ));
    }
    let record = service
        .override_secret_finding(
            identity,
            confirmation,
            tty_mode,
            OverrideReasonCode::ReviewedFalsePositive,
            now_unix_seconds,
        )
        .await?;
    Ok(secret_status_view(&preview, record))
}

fn require_confirmation(
    result: PromptResult,
    action: PromptAction,
) -> Result<ExactConfirmation, RepoComError> {
    match result {
        PromptResult::Confirmed(confirmation) if confirmation.action == action => Ok(confirmation),
        PromptResult::Confirmed(_) => Err(RepoComError::usage(
            "outbound UI returned a confirmation for a different action",
        )),
        PromptResult::NonTty => Err(RepoComError::operator_action_required(
            "interactive approval or override requires a TTY",
        )),
        PromptResult::Expired => Err(RepoComError::policy_blocked(
            "the exact preview is expired; create a new immutable revision",
        )),
        PromptResult::Cancelled | PromptResult::Defaulted => Err(
            RepoComError::operator_action_required("operator did not confirm the exact action"),
        ),
        PromptResult::InvalidInput { .. } | PromptResult::PreviewHashMismatch { .. } => Err(
            RepoComError::usage("operator input did not match the exact confirmation grammar"),
        ),
        PromptResult::InputUnavailable { .. } => Err(RepoComError::operator_action_required(
            "operator input was unavailable for the exact action",
        )),
    }
}

fn approval_status_view(preview: &OutboundPreview, record: ApprovalRecord) -> ApprovalStatusView {
    let mut view = ApprovalStatusView::new(
        preview.repository_id.clone(),
        preview.draft_id.clone(),
        preview.revision,
        preview.destination.clone(),
        ApprovalView::from_record(&record),
    );
    view.policy = preview.policy.clone();
    view.safety = preview.safety.clone();
    view.preview_hash = Some(preview.preview_hash.clone());
    view
}

fn secret_status_view(
    preview: &OutboundPreview,
    record: SecretOverrideRecord,
) -> SecretFindingStatusView {
    let mut view = SecretFindingStatusView::new(
        preview.repository_id.clone(),
        preview.draft_id.clone(),
        preview.revision,
        preview.destination.clone(),
        preview.safety.clone().with_override(&record),
    );
    view.approval = preview.approval.clone();
    view.policy = preview.policy.clone();
    view.preview_hash = Some(preview.preview_hash.clone());
    view.exact_text_hash = Some(preview.exact_text_hash.clone());
    view.expires_at_unix_seconds = preview.expires_at_unix_seconds;
    view
}

fn sha256_text(value: &str) -> String {
    let digest = Sha256::digest(value.as_bytes());
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
