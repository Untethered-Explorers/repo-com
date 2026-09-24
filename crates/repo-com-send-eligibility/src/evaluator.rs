use std::fmt::Write as _;

use repo_com_approval::{
    APPROVAL_LIFETIME_SECONDS, ApprovalBindingField, ApprovalCheck, ApprovalDisposition,
    ApprovalInvalidReason, ApprovalPreview, ApprovalRecord, PreviewInvalidReason,
};
use repo_com_config::{ResolvedConfig, ResolvedDestination};
use repo_com_draft_content::{ContentRenderer, RenderedMessage};
use repo_com_draft_model::DraftRevision;
use repo_com_draft_safety::{SecretScanResult, SecretScanner};
use repo_com_foundation::TtyMode;
use repo_com_policy::{PolicyDecision, PolicyTuple, exact_policy_match, is_sha256_hex};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::{
    EligibilityAuthority, EligibilityBlocker, EligibilityDecision, OutboundCorrection,
    RevalidationFacts,
};

/// Borrowed current-state inputs for one pure eligibility decision.
///
/// The approval check, policy decision, and scan result must all be freshly
/// obtained for the same current state. This evaluator only reads them; it
/// cannot create approval, activate policy, override a finding, claim a send,
/// or perform network I/O.
pub struct EligibilityInput<'a> {
    /// Injected current Unix time used for expiry boundaries.
    pub now_unix_seconds: u64,
    /// Explicit caller-supplied terminal mode.
    pub tty_mode: TtyMode,
    /// Current resolved repository configuration.
    pub config: &'a ResolvedConfig,
    /// Immutable draft revision being evaluated.
    pub revision: &'a DraftRevision,
    /// Complete current approval preview used as a cross-check.
    pub approval_preview: &'a ApprovalPreview,
    /// Read-only current approval check.
    pub approval_check: &'a ApprovalCheck,
    /// Read-only current exact-policy decision.
    pub policy_decision: &'a PolicyDecision,
    /// Read-only current redacted secret-scan result.
    pub scan_result: &'a SecretScanResult,
}

/// Stateless pure eligibility evaluator.
#[derive(Clone, Copy, Debug, Default)]
pub struct EligibilityEvaluator;

impl EligibilityEvaluator {
    /// Creates the stateless evaluator.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Evaluates one exact revision without mutation or I/O.
    #[must_use]
    pub fn evaluate(&self, input: &EligibilityInput<'_>) -> EligibilityDecision {
        evaluate(input)
    }
}

/// Evaluates one exact revision without mutation or I/O.
#[must_use]
pub fn evaluate(input: &EligibilityInput<'_>) -> EligibilityDecision {
    let rendered = ContentRenderer::new().render_revision(
        input.revision,
        input.revision.expiry().created_at_unix_seconds(),
    );
    let exact_text_hash = rendered.as_ref().map_or_else(
        |_| input.approval_preview.exact_text_hash().to_owned(),
        |message| sha256_bytes(message.exact_text().as_bytes()),
    );
    let facts = revalidation_facts(input, &exact_text_hash);

    if input.config.config.repository_id != input.revision.repository_id() {
        return blocked(facts, EligibilityBlocker::RepositoryScopeChanged);
    }
    if input.revision.canonical_hash() != input.revision.content_hash() {
        return blocked(facts, EligibilityBlocker::RevisionChanged);
    }
    if input.now_unix_seconds >= input.revision.expiry().expires_at_unix_seconds() {
        return blocked(facts, EligibilityBlocker::DraftExpired);
    }

    let Some(current_destination) = input
        .config
        .destination(input.revision.destination_alias().as_str())
    else {
        return blocked(facts, EligibilityBlocker::DestinationChanged);
    };
    if current_destination != input.revision.resolved_destination() {
        return blocked(facts, EligibilityBlocker::DestinationChanged);
    }

    let Ok(rendered) = rendered else {
        return blocked(facts, EligibilityBlocker::CurrentStateInvalid);
    };
    if !rendered_matches_revision(&rendered, input.revision) {
        return blocked(facts, EligibilityBlocker::RevisionChanged);
    }

    let recomputed_scan = SecretScanner::new().scan_rendered(&rendered);
    if &recomputed_scan != input.scan_result {
        return blocked(facts, EligibilityBlocker::SecretScanChanged);
    }
    if let Some(blocker) = validate_approval_preview(input, &rendered, &facts) {
        return blocked(facts, blocker);
    }
    if let Some(blocker) = validate_approval_check_snapshot(input, &facts) {
        return blocked(facts, blocker);
    }

    let approval = validate_approval(input, &facts);
    let policy = validate_policy(input, &facts);

    if input.scan_result.is_blocked() && approval.is_err() {
        return blocked(facts, EligibilityBlocker::UnresolvedSecretFinding);
    }
    if let Ok(authority) = approval {
        return eligible(facts, authority);
    }
    if let Ok(authority) = policy {
        return eligible(facts, authority);
    }

    let approval_blocker = approval.expect_err("an ineligible approval has a blocker");
    let policy_blocker = policy.expect_err("an ineligible policy has a blocker");
    let blocker = if approval_blocker == EligibilityBlocker::AuthorityMissing {
        policy_blocker
    } else {
        approval_blocker
    };
    let blocker = if blocker == EligibilityBlocker::AuthorityMissing && !input.tty_mode.is_tty() {
        EligibilityBlocker::OperatorActionRequired
    } else {
        blocker
    };
    blocked(facts, blocker)
}

fn eligible(
    revalidation: RevalidationFacts,
    authority: EligibilityAuthority,
) -> EligibilityDecision {
    EligibilityDecision::Eligible {
        revalidation,
        authority,
        correction: OutboundCorrection::NewDraft,
    }
}

fn blocked(revalidation: RevalidationFacts, blocker: EligibilityBlocker) -> EligibilityDecision {
    EligibilityDecision::Blocked {
        revalidation,
        blocker,
    }
}

fn revalidation_facts(input: &EligibilityInput<'_>, exact_text_hash: &str) -> RevalidationFacts {
    let alias = input.revision.destination_alias().as_str();
    let current_destination = input.config.destination(alias);
    let repository = RepositoryBinding {
        repository_id: &input.config.config.repository_id,
        workspace_id: &input.config.config.discord.workspace_id,
    };
    let destination = DestinationBinding {
        current: current_destination,
        revision: input.revision.resolved_destination(),
    };

    RevalidationFacts {
        repository_id: input.config.config.repository_id.clone(),
        repository_hash: sha256_json(&repository),
        workspace_id: input.config.config.discord.workspace_id.clone(),
        draft_id: input.approval_preview.draft_id().to_owned(),
        revision: input.revision.number(),
        revision_hash: input.revision.content_hash().to_owned(),
        destination_alias: alias.to_owned(),
        destination_hash: sha256_json(&destination),
        exact_text_hash: exact_text_hash.to_owned(),
        metadata_hash: sha256_json(input.revision.metadata()),
        config_hash: input.config.canonical_hash(),
        policy_basis_hash: sha256_json(input.policy_decision),
        scan_hash: sha256_json(input.scan_result),
        approval_preview_hash: input.approval_preview.preview_hash().to_owned(),
        draft_created_at_unix_seconds: input.revision.expiry().created_at_unix_seconds(),
        draft_expires_at_unix_seconds: input.revision.expiry().expires_at_unix_seconds(),
        evaluated_at_unix_seconds: input.now_unix_seconds,
    }
}

fn rendered_matches_revision(rendered: &RenderedMessage, revision: &DraftRevision) -> bool {
    rendered.repository_id() == revision.repository_id()
        && rendered.revision() == revision.number()
        && rendered.content_hash() == revision.content_hash()
        && rendered.destination_alias() == revision.destination_alias().as_str()
        && rendered.resolved_destination() == revision.resolved_destination()
        && rendered.metadata() == revision.metadata()
        && rendered.expiry() == revision.expiry()
}

fn validate_approval_preview(
    input: &EligibilityInput<'_>,
    rendered: &RenderedMessage,
    facts: &RevalidationFacts,
) -> Option<EligibilityBlocker> {
    let preview = input.approval_preview;
    if preview.repository_id() != facts.repository_id
        || preview.draft_id() != facts.draft_id
        || preview.revision() != facts.revision
        || preview.revision_hash() != facts.revision_hash
        || preview.destination_alias() != facts.destination_alias
        || preview.resolved_destination() != rendered.resolved_destination()
        || preview.exact_text() != rendered.exact_text()
        || preview.exact_text_hash() != facts.exact_text_hash
        || preview.metadata() != rendered.metadata()
        || preview.metadata_hash() != facts.metadata_hash
        || preview.event_type() != input.revision.event_type().as_str()
        || preview.severity() != input.revision.severity().as_str()
        || preview.reply_reference() != input.revision.reply_reference()
        || preview.draft_created_at_unix_seconds() != facts.draft_created_at_unix_seconds
        || preview.draft_expires_at_unix_seconds() != facts.draft_expires_at_unix_seconds
    {
        return Some(EligibilityBlocker::RevisionChanged);
    }
    if preview.config_hash() != facts.config_hash {
        return Some(EligibilityBlocker::ConfigChanged);
    }
    if preview.destination_hash() != facts.destination_hash {
        return Some(EligibilityBlocker::DestinationChanged);
    }
    if preview.scan_result() != input.scan_result || preview.scan_hash() != facts.scan_hash {
        return Some(EligibilityBlocker::SecretScanChanged);
    }
    if preview.policy_basis() != input.policy_decision
        || preview.policy_basis_hash() != facts.policy_basis_hash
    {
        return Some(EligibilityBlocker::PolicyBasisChanged);
    }
    if !is_sha256_hex(preview.preview_hash()) {
        return Some(EligibilityBlocker::CurrentStateInvalid);
    }
    None
}

fn validate_approval_check_snapshot(
    input: &EligibilityInput<'_>,
    facts: &RevalidationFacts,
) -> Option<EligibilityBlocker> {
    let revalidation = input.approval_check.revalidation();
    if revalidation.repository_id != facts.repository_id
        || revalidation.draft_id != facts.draft_id
        || revalidation.revision != facts.revision
        || revalidation.revision_hash != facts.revision_hash
        || revalidation.exact_text_hash != facts.exact_text_hash
        || revalidation.metadata_hash != facts.metadata_hash
        || revalidation.config_hash != facts.config_hash
        || revalidation.destination_hash != facts.destination_hash
        || revalidation.policy_basis_hash != facts.policy_basis_hash
        || revalidation.scan_hash != facts.scan_hash
        || revalidation.preview_hash != facts.approval_preview_hash
        || revalidation.draft_expires_at_unix_seconds != facts.draft_expires_at_unix_seconds
    {
        return Some(EligibilityBlocker::ApprovalStateChanged);
    }
    match (input.approval_check.is_valid(), &revalidation.approval_hash) {
        (true, Some(hash)) if is_sha256_hex(hash) => {}
        (false, None) => {}
        _ => return Some(EligibilityBlocker::ApprovalStateChanged),
    }
    None
}

fn validate_approval(
    input: &EligibilityInput<'_>,
    facts: &RevalidationFacts,
) -> Result<EligibilityAuthority, EligibilityBlocker> {
    let Some(record) = input.approval_check.approval() else {
        return Err(map_approval_disposition(input.approval_check.disposition()));
    };
    let approval_hash = input
        .approval_check
        .revalidation()
        .approval_hash
        .as_deref()
        .ok_or(EligibilityBlocker::ApprovalStateChanged)?;
    if record.hash().ok().as_deref() != Some(approval_hash)
        || !approval_matches_current(record, input.approval_preview, facts)
        || record.schema_version != 1
        || record.actor_kind != "operator"
    {
        return Err(EligibilityBlocker::ApprovalStateChanged);
    }

    let expected_expiry = record
        .approved_at_unix_seconds
        .saturating_add(APPROVAL_LIFETIME_SECONDS)
        .min(facts.draft_expires_at_unix_seconds);
    if record.draft_expires_at_unix_seconds != facts.draft_expires_at_unix_seconds
        || record.expires_at_unix_seconds != expected_expiry
        || record.approved_at_unix_seconds >= record.expires_at_unix_seconds
    {
        return Err(EligibilityBlocker::ApprovalStateChanged);
    }
    if input.now_unix_seconds >= record.expires_at_unix_seconds {
        return Err(EligibilityBlocker::ApprovalExpired);
    }
    match (input.scan_result.is_blocked(), &record.override_hash) {
        (true, Some(hash)) if is_sha256_hex(hash) => {}
        (true, _) => return Err(EligibilityBlocker::UnresolvedSecretFinding),
        (false, None) => {}
        (false, Some(_)) => return Err(EligibilityBlocker::ApprovalStateChanged),
    }

    Ok(EligibilityAuthority::HumanApproval {
        approval_id: record.approval_id.clone(),
        approval_hash: approval_hash.to_owned(),
        expires_at_unix_seconds: record.expires_at_unix_seconds,
    })
}

fn approval_matches_current(
    record: &ApprovalRecord,
    preview: &ApprovalPreview,
    facts: &RevalidationFacts,
) -> bool {
    record.repository_id == facts.repository_id
        && record.draft_id == facts.draft_id
        && record.revision == facts.revision
        && record.revision_hash == facts.revision_hash
        && record.exact_text_hash == facts.exact_text_hash
        && record.metadata_hash == facts.metadata_hash
        && record.config_hash == facts.config_hash
        && record.destination_hash == facts.destination_hash
        && record.policy_basis_hash == facts.policy_basis_hash
        && record.scan_hash == facts.scan_hash
        && record.preview_hash == facts.approval_preview_hash
        && preview.repository_id() == facts.repository_id
        && preview.draft_id() == facts.draft_id
        && preview.revision() == facts.revision
        && preview.revision_hash() == facts.revision_hash
}

fn map_approval_disposition(disposition: &ApprovalDisposition) -> EligibilityBlocker {
    match disposition {
        ApprovalDisposition::Valid(_) => EligibilityBlocker::ApprovalStateChanged,
        ApprovalDisposition::Invalid(reason) => match reason {
            ApprovalInvalidReason::ApprovalExpired => EligibilityBlocker::ApprovalExpired,
            ApprovalInvalidReason::SecretOverrideRequired => {
                EligibilityBlocker::UnresolvedSecretFinding
            }
            ApprovalInvalidReason::BindingChanged(field) => map_binding_field(field),
            ApprovalInvalidReason::PreviewInvalid(reason) => map_preview_reason(reason),
            ApprovalInvalidReason::ApprovalMissing => EligibilityBlocker::AuthorityMissing,
            ApprovalInvalidReason::PersistenceInconsistent
            | ApprovalInvalidReason::ApprovalRevoked => EligibilityBlocker::ApprovalStateChanged,
        },
    }
}

fn map_binding_field(field: &ApprovalBindingField) -> EligibilityBlocker {
    match field {
        ApprovalBindingField::Repository => EligibilityBlocker::RepositoryScopeChanged,
        ApprovalBindingField::Draft
        | ApprovalBindingField::Revision
        | ApprovalBindingField::RevisionHash
        | ApprovalBindingField::ExactText
        | ApprovalBindingField::Metadata
        | ApprovalBindingField::Expiry
        | ApprovalBindingField::Preview => EligibilityBlocker::RevisionChanged,
        ApprovalBindingField::Config => EligibilityBlocker::ConfigChanged,
        ApprovalBindingField::Destination => EligibilityBlocker::DestinationChanged,
        ApprovalBindingField::PolicyBasis => EligibilityBlocker::PolicyBasisChanged,
        ApprovalBindingField::SafetyScan => EligibilityBlocker::SecretScanChanged,
        ApprovalBindingField::ApprovalId => EligibilityBlocker::ApprovalStateChanged,
    }
}

fn map_preview_reason(reason: &PreviewInvalidReason) -> EligibilityBlocker {
    match reason {
        PreviewInvalidReason::RepositoryScopeChanged => EligibilityBlocker::RepositoryScopeChanged,
        PreviewInvalidReason::CurrentRevisionChanged
        | PreviewInvalidReason::RevisionContentChanged
        | PreviewInvalidReason::ExactTextChanged
        | PreviewInvalidReason::MetadataChanged => EligibilityBlocker::RevisionChanged,
        PreviewInvalidReason::DestinationChanged => EligibilityBlocker::DestinationChanged,
        PreviewInvalidReason::DraftExpired => EligibilityBlocker::DraftExpired,
        PreviewInvalidReason::DraftAlreadySent => EligibilityBlocker::CurrentStateInvalid,
    }
}

fn validate_policy(
    input: &EligibilityInput<'_>,
    facts: &RevalidationFacts,
) -> Result<EligibilityAuthority, EligibilityBlocker> {
    let tuple = PolicyTuple::new(
        input.revision.event_type().as_str().to_owned(),
        input.revision.destination_alias().as_str().to_owned(),
        input.revision.severity().as_str().to_owned(),
    );
    let exact = exact_policy_match(&input.config.config, &tuple)
        .map_err(|_| EligibilityBlocker::CurrentStateInvalid)?;
    let PolicyDecision::Eligible(snapshot) = input.policy_decision else {
        return Err(match input.policy_decision {
            PolicyDecision::Stale(_) => EligibilityBlocker::StalePolicyActivation,
            PolicyDecision::Ambiguous { .. } => EligibilityBlocker::PolicyAmbiguous,
            PolicyDecision::Deactivated(_) => EligibilityBlocker::PolicyNotActive,
            PolicyDecision::NotActivated
            | PolicyDecision::NoExactMatch
            | PolicyDecision::NotConfigured => EligibilityBlocker::AuthorityMissing,
            PolicyDecision::Eligible(_) => unreachable!("eligible was matched above"),
        });
    };
    let Some(exact) = exact else {
        return Err(EligibilityBlocker::AuthorityMissing);
    };
    if snapshot.repository_id != facts.repository_id
        || snapshot.tuple != tuple
        || !snapshot.is_current()
        || !snapshot.active
        || snapshot.deactivated_at.is_some()
        || snapshot.recorded_config_hash != facts.config_hash
        || snapshot.current_config_hash != facts.config_hash
        || snapshot.recorded_tuple_hash != exact.tuple_hash
        || snapshot.current_tuple_hash != exact.tuple_hash
        || exact.config_hash != facts.config_hash
        || !is_sha256_hex(&snapshot.recorded_config_hash)
        || !is_sha256_hex(&snapshot.recorded_tuple_hash)
    {
        return Err(EligibilityBlocker::StalePolicyActivation);
    }

    Ok(EligibilityAuthority::ActivatedPolicy {
        activation_id: snapshot.activation_id.clone(),
        activation_hash: sha256_json(snapshot),
        config_hash: snapshot.current_config_hash.clone(),
        tuple_hash: snapshot.current_tuple_hash.clone(),
        activated_at: snapshot.activated_at.clone(),
    })
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

fn sha256_json<T: Serialize>(value: &T) -> String {
    sha256_bytes(
        &serde_json::to_vec(value).expect("eligibility bindings contain serializable values"),
    )
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        write!(&mut output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}
