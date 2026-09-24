//! Focused operations handler modules and the non-authoritative UI boundary.

pub mod audit;
pub mod config;
pub mod policy;
pub mod purge;
pub mod state;

use repo_com_foundation::TtyMode;
use repo_com_terminal_operations::{
    ActivationPreviewView, KeyboardPrompt, PromptInput, PromptResult, PurgePlanView,
};

/// Operations presentation boundary used only for permission-widening and
/// destructive actions. Implementations collect an intent; they never mutate
/// policy or purge state.
pub trait OperationsUi {
    /// Requests an exact policy activation confirmation.
    fn request_activation(&mut self, preview: ActivationPreviewView) -> PromptResult;

    /// Requests an exact purge confirmation.
    fn request_purge(&mut self, plan: PurgePlanView) -> PromptResult;
}

/// Keyboard-only operations UI adapter over the existing prompt owner.
#[derive(Clone, Debug)]
pub struct KeyboardOperationsUi<I> {
    prompt: KeyboardPrompt<I>,
}

impl<I> KeyboardOperationsUi<I> {
    /// Creates an adapter with an explicit stream mode.
    #[must_use]
    pub const fn new(input: I, tty_mode: TtyMode) -> Self {
        Self {
            prompt: KeyboardPrompt::new(input, tty_mode),
        }
    }

    /// Returns the underlying prompt for inspection and scripted tests.
    #[must_use]
    pub const fn prompt(&self) -> &KeyboardPrompt<I> {
        &self.prompt
    }

    /// Consumes this adapter and returns its prompt.
    #[must_use]
    pub fn into_prompt(self) -> KeyboardPrompt<I> {
        self.prompt
    }
}

impl<I: PromptInput> OperationsUi for KeyboardOperationsUi<I> {
    fn request_activation(&mut self, preview: ActivationPreviewView) -> PromptResult {
        self.prompt.request_exact_activation(preview)
    }

    fn request_purge(&mut self, plan: PurgePlanView) -> PromptResult {
        self.prompt.request_exact_purge(plan)
    }
}

/// Readable aliases for the operations UI adapter.
pub type OperationsUiAdapter<I> = KeyboardOperationsUi<I>;
pub type KeyboardUi<I> = KeyboardOperationsUi<I>;

pub use audit::{AuditQueryRequest, AuditService, LocalAuditService, map_audit_error, query};
pub use config::{
    ConfigService, ConfigValidationRequest, map_config_error, validate, validate_config,
};
pub use policy::{
    PolicyActivationRequest, PolicyActivationView, PolicyService, PolicyStatusRequest, activate,
    map_policy_error, render_activation_result, status,
};
pub use purge::{
    LocalPurgeService, PurgeExecutionRequest, PurgePlanRequest, PurgeService, execute,
    map_purge_error, plan,
};
pub use state::{
    LifecycleInspectionRequest, LifecycleInspectionResult, StateService, StateVerificationRequest,
    VerificationOnlyService, inspect, map_lifecycle_error, project_verification, verify,
};
