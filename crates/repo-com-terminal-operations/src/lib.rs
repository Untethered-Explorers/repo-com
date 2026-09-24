#![forbid(unsafe_code)]
#![doc = "Deterministic, accessible operations terminal presentation for repo-com."]

pub mod prompt;
pub mod purge;
pub mod render;
pub mod width;

#[cfg(test)]
#[path = "../tests/terminal_operations_contract.rs"]
mod terminal_operations_contract;

pub use prompt::{
    ActivationConfirmationIdentity, ConfirmationIntent, ConfirmationSyntax,
    ExactActivationIdentity, ExactConfirmation, ExactScope, KeyboardPrompt,
    PolicyActivationIdentity, PolicyActivationPrompt, PromptAction, PromptAdapter, PromptInput,
    PromptInputError, PromptRequest, PromptResult, PurgeConfirmationPrompt, ScriptedPromptInput,
    StdinPrompt, exact_scope_instructions, purge_confirmation_from_intent,
};
pub use purge::{
    PurgeConfirmation, PurgeConfirmationIdentity, PurgeCountsView, PurgeExecutionResultView,
    PurgeExecutionState, PurgeExecutionView, PurgePlanStatusView, PurgePlanView, PurgeProvenance,
};
pub use render::{
    AcknowledgementStatusView, AcknowledgementView, ActivationPreviewView, ArchiveStatusView,
    ArchiveView, AuditEventView, AuditStatusView, AuditView, CheckState, ConfigStatus,
    ConfigStatusView, ConfigView, DetailField, ExactPolicyTuple, InboundCommitView,
    InboundItemStatusView, InboundItemView, InboundStatusView, InboundView, LifecyclePageView,
    LifecycleStatusView, LifecycleView, LocalErrorKind, LocalErrorView, LocalStateView,
    OperationView, OperationsRenderer, OperationsView, PolicyActivationSnapshotView,
    PolicyActivationStatusView, PolicyActivationView, PolicyState, PolicyStatusView, PolicyTuple,
    PolicyView, Provenance, PurgeResultView, PurgeView, ReadReceiptStatus, RemoteStateView,
    ReplyClaimStatus, ReplyLinkView, RetentionCountsView, RetentionPolicyView, RetentionStatus,
    RetentionStatusView, RetentionView, StateCheckView, StateVerificationStatusView,
    StateVerificationView, StateView, is_ansi_free, render_acknowledgement, render_activation,
    render_archive, render_audit, render_config, render_error, render_inbound,
    render_inbound_commit, render_lifecycle, render_lifecycle_page, render_machine,
    render_machine_failure, render_operations, render_policy, render_purge_execution,
    render_purge_plan, render_reply_link, render_retention, render_state_verification, render_view,
    rendered_width,
};
pub use width::{
    MIN_TERMINAL_WIDTH, TerminalWidth, block_lines, character_width, display_width, field_lines,
    lines_fit, sanitize_text, strip_ansi, wrap_text,
};
