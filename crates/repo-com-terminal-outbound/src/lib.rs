#![forbid(unsafe_code)]
#![doc = "Deterministic, accessible outbound terminal presentation for repo-com."]

pub mod preview;
pub mod prompt;
pub mod render;
pub mod width;

#[cfg(test)]
#[path = "../tests/terminal_outbound_contract.rs"]
mod terminal_outbound_contract;

pub use preview::{
    ApprovalState, ApprovalView, DEFAULT_APPROVAL_EXPIRY_SECONDS, DeliveryOutcome, DeliveryView,
    DestinationView, ErrorView, MetadataView, OutboundPreview, PolicyActivationView, PolicyState,
    PolicyTuple, PolicyView, Provenance, ReplyReferenceView, SafetyFindingView, SafetyState,
    SafetyView,
};
pub use prompt::{
    ApprovalPrompt, ConfirmationSyntax, ExactConfirmation, ExactPreviewIdentity, KeyboardPrompt,
    PromptAction, PromptAdapter, PromptInput, PromptInputError, PromptRequest, PromptResult,
    SecretOverridePrompt, StdinPrompt,
};
pub use render::{
    ApprovalStatusView, OutboundRenderer, OutboundView, PolicyStatusView, RenderOptions,
    SecretFindingStatusView, is_ansi_free, render_approval, render_delivery, render_error,
    render_machine, render_machine_failure, render_policy, render_preview, render_secret_finding,
    render_view, rendered_width,
};
pub use width::{
    MIN_TERMINAL_WIDTH, TerminalWidth, block_lines, display_width, field_lines, lines_fit,
    strip_ansi, wrap_text,
};
