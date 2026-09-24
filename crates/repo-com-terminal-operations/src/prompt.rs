//! Keyboard-only, non-authoritative operations prompt adapters.
//!
//! Policy activation and confirmed purge are permission-widening or
//! destructive boundaries.  These adapters collect an exact operator response
//! and return an intent only.  They do not activate policy, delete state, open
//! SQLite, probe a terminal, or create reusable authority; the domain layer
//! must revalidate the complete identity and current state.

use std::collections::VecDeque;
use std::fmt;
use std::io::{self, BufRead, Write};

use repo_com_foundation::TtyMode;
use repo_com_policy::PolicyTuple;
use serde::{Deserialize, Serialize};

use crate::purge::{PurgeConfirmation, PurgeConfirmationIdentity, PurgePlanView};
use crate::render::{ActivationPreviewView, RenderOptions, render_activation, render_purge_plan};
use crate::width::wrap_text;

/// A safe input failure from an injected keyboard source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptInputError {
    /// Redacted detail suitable for a local diagnostic.
    pub detail: String,
}

impl PromptInputError {
    /// Creates an input error.
    #[must_use]
    pub fn new(detail: impl Into<String>) -> Self {
        Self {
            detail: detail.into(),
        }
    }
}

impl fmt::Display for PromptInputError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl std::error::Error for PromptInputError {}

/// A keyboard input dependency. Tests can inject deterministic lines.
pub trait PromptInput {
    /// Reads one line, returning `None` at end of input.
    fn read_line(&mut self, prompt: &str) -> Result<Option<String>, PromptInputError>;
}

/// Production stdin adapter, called only after an explicit TTY check.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StdinPrompt;

impl PromptInput for StdinPrompt {
    fn read_line(&mut self, prompt: &str) -> Result<Option<String>, PromptInputError> {
        print!("{prompt}");
        io::stdout()
            .flush()
            .map_err(|error| PromptInputError::new(format!("prompt output failed: {error}")))?;
        let mut line = String::new();
        let bytes = io::stdin()
            .lock()
            .read_line(&mut line)
            .map_err(|error| PromptInputError::new(format!("prompt input failed: {error}")))?;
        if bytes == 0 { Ok(None) } else { Ok(Some(line)) }
    }
}

/// Deterministic keyboard input useful to adapters and contract tests.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScriptedPromptInput {
    lines: VecDeque<String>,
    prompts: Vec<String>,
}

impl ScriptedPromptInput {
    /// Creates scripted input in keyboard order.
    #[must_use]
    pub fn new<I, S>(lines: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            lines: lines.into_iter().map(Into::into).collect(),
            prompts: Vec::new(),
        }
    }

    /// Returns the number of input reads attempted.
    #[must_use]
    pub fn calls(&self) -> usize {
        self.prompts.len()
    }

    /// Returns all prompt text observed by the input.
    #[must_use]
    pub fn prompts(&self) -> &[String] {
        &self.prompts
    }
}

impl PromptInput for ScriptedPromptInput {
    fn read_line(&mut self, prompt: &str) -> Result<Option<String>, PromptInputError> {
        self.prompts.push(prompt.to_owned());
        Ok(self.lines.pop_front())
    }
}

/// Permission-widening operations action being confirmed.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromptAction {
    /// Activate one exact policy tuple and hash binding.
    ActivatePolicy,
    /// Confirm one exact local purge plan.
    ConfirmPurge,
}

impl PromptAction {
    /// Returns the human-readable action label.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ActivatePolicy => "Activate exact policy",
            Self::ConfirmPurge => "Confirm exact purge",
        }
    }

    /// Returns the token accepted by exact-scope syntax.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::ActivatePolicy => "activate",
            Self::ConfirmPurge => "purge",
        }
    }
}

impl fmt::Display for PromptAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Response grammar used by a keyboard prompt.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfirmationSyntax {
    /// Y/yes confirms after the complete preview; Enter and N cancel.
    #[default]
    YesNo,
    /// The operator types the complete exact scope and hash identity.
    ExactScope,
}

/// Compatibility name for the exact-scope response grammar.
pub type ExactScope = ConfirmationSyntax;

/// A non-authoritative exact confirmation intent.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ExactConfirmation {
    /// Action requested.
    pub action: PromptAction,
    /// Repository scope shown in the preview.
    pub repository_id: String,
    /// Activation ID or purge object label shown in the preview.
    pub object_id: String,
    /// Exact policy tuple or purge scope, when applicable.
    pub scope: Option<String>,
    /// Current configuration hash shown in the preview.
    pub config_hash: Option<String>,
    /// Exact policy tuple hash, when applicable.
    pub tuple_hash: Option<String>,
    /// Exact purge plan hash, when applicable.
    pub plan_hash: Option<String>,
}

impl ExactConfirmation {
    /// Returns whether the confirmation is an activation intent.
    #[must_use]
    pub const fn is_activation(&self) -> bool {
        matches!(self.action, PromptAction::ActivatePolicy)
    }

    /// Returns whether the confirmation is a purge intent.
    #[must_use]
    pub const fn is_purge(&self) -> bool {
        matches!(self.action, PromptAction::ConfirmPurge)
    }
}

/// Exact policy activation identity echoed by a confirmation adapter.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActivationConfirmationIdentity {
    /// Repository scope shown in the preview.
    pub repository_id: String,
    /// Activation ID shown in the preview.
    pub activation_id: String,
    /// Exact policy tuple shown in the preview.
    pub tuple: PolicyTuple,
    /// Current configuration hash shown in the preview.
    pub config_hash: String,
    /// Current tuple hash shown in the preview.
    pub tuple_hash: String,
}

impl ActivationConfirmationIdentity {
    /// Projects identity from an activation preview.
    #[must_use]
    pub fn from_preview(preview: &ActivationPreviewView) -> Self {
        Self {
            repository_id: preview.repository_id.clone(),
            activation_id: preview.activation_id.clone(),
            tuple: preview.tuple.clone(),
            config_hash: preview.config_hash.clone(),
            tuple_hash: preview.tuple_hash.clone(),
        }
    }

    /// Returns whether a collected intent is bound to this exact identity.
    #[must_use]
    pub fn matches(&self, confirmation: &ExactConfirmation) -> bool {
        let tuple = self.tuple.to_string();
        confirmation.is_activation()
            && confirmation.repository_id == self.repository_id
            && confirmation.object_id == self.activation_id
            && confirmation.scope.as_deref() == Some(tuple.as_str())
            && confirmation.config_hash.as_deref() == Some(self.config_hash.as_str())
            && confirmation.tuple_hash.as_deref() == Some(self.tuple_hash.as_str())
    }
}

/// Compatibility names for the exact activation identity.
pub type ExactActivationIdentity = ActivationConfirmationIdentity;
pub type PolicyActivationIdentity = ActivationConfirmationIdentity;

/// A prompt request containing the complete view that must be shown first.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "request", rename_all = "kebab-case")]
pub enum PromptRequest {
    /// Exact policy activation request.
    Activation {
        /// Complete activation preview.
        preview: ActivationPreviewView,
        /// Response grammar.
        syntax: ConfirmationSyntax,
    },
    /// Exact purge request.
    Purge {
        /// Complete non-mutating plan preview.
        plan: PurgePlanView,
        /// Response grammar.
        syntax: ConfirmationSyntax,
    },
}

impl PromptRequest {
    /// Creates a normal yes/no activation request.
    #[must_use]
    pub fn activation(preview: ActivationPreviewView) -> Self {
        Self::Activation {
            preview,
            syntax: ConfirmationSyntax::YesNo,
        }
    }

    /// Compatibility alias for [`Self::activation`].
    #[must_use]
    pub fn policy_activation(preview: ActivationPreviewView) -> Self {
        Self::activation(preview)
    }

    /// Creates an exact-scope activation request.
    #[must_use]
    pub fn exact_activation(preview: ActivationPreviewView) -> Self {
        Self::Activation {
            preview,
            syntax: ConfirmationSyntax::ExactScope,
        }
    }

    /// Compatibility alias for [`Self::exact_activation`].
    #[must_use]
    pub fn exact_policy_activation(preview: ActivationPreviewView) -> Self {
        Self::exact_activation(preview)
    }

    /// Creates a normal yes/no purge request.
    #[must_use]
    pub fn purge(plan: PurgePlanView) -> Self {
        Self::Purge {
            plan,
            syntax: ConfirmationSyntax::YesNo,
        }
    }

    /// Compatibility alias for [`Self::purge`].
    #[must_use]
    pub fn purge_confirmation(plan: PurgePlanView) -> Self {
        Self::purge(plan)
    }

    /// Compatibility alias for [`Self::exact_purge`].
    #[must_use]
    pub fn exact_purge_confirmation(plan: PurgePlanView) -> Self {
        Self::exact_purge(plan)
    }

    /// Creates an exact-scope purge request.
    #[must_use]
    pub fn exact_purge(plan: PurgePlanView) -> Self {
        Self::Purge {
            plan,
            syntax: ConfirmationSyntax::ExactScope,
        }
    }

    /// Returns the action associated with this request.
    #[must_use]
    pub const fn action(&self) -> PromptAction {
        match self {
            Self::Activation { .. } => PromptAction::ActivatePolicy,
            Self::Purge { .. } => PromptAction::ConfirmPurge,
        }
    }
}

/// A typed keyboard prompt result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromptResult {
    /// The exact response was collected.
    Confirmed(ExactConfirmation),
    /// The operator explicitly cancelled.
    Cancelled,
    /// Enter selected the safe cancellation default.
    Defaulted,
    /// Input did not follow the documented grammar.
    InvalidInput {
        /// Redacted input marker.
        input: String,
    },
    /// A purge plan hash changed after preview.
    PlanHashChanged {
        /// Hash shown in the preview.
        expected: String,
        /// Hash supplied by the operator.
        supplied: String,
    },
    /// A repository, scope, or other exact binding differed.
    ScopeMismatch {
        /// Safe mismatch class, without echoing operator input.
        reason: String,
    },
    /// The adapter was unavailable because the stream was non-interactive.
    NonTty,
    /// Input was unavailable or failed.
    InputUnavailable {
        /// Redacted input detail.
        detail: String,
    },
}

impl PromptResult {
    /// Returns whether the operator confirmed an exact intent.
    #[must_use]
    pub const fn is_confirmed(&self) -> bool {
        matches!(self, Self::Confirmed(_))
    }

    /// Returns whether the operator cancelled.
    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    /// Returns whether Enter selected the safe default.
    #[must_use]
    pub const fn is_defaulted(&self) -> bool {
        matches!(self, Self::Defaulted)
    }

    /// Returns whether input or an exact binding was invalid.
    #[must_use]
    pub const fn is_invalid(&self) -> bool {
        matches!(
            self,
            Self::InvalidInput { .. } | Self::PlanHashChanged { .. } | Self::ScopeMismatch { .. }
        )
    }

    /// Returns whether a purge plan hash changed.
    #[must_use]
    pub const fn is_plan_hash_changed(&self) -> bool {
        matches!(self, Self::PlanHashChanged { .. })
    }

    /// Returns whether a non-TTY stream prevented prompting.
    #[must_use]
    pub const fn is_non_tty(&self) -> bool {
        matches!(self, Self::NonTty)
    }

    /// Returns the exact intent, if any.
    #[must_use]
    pub const fn confirmation(&self) -> Option<&ExactConfirmation> {
        match self {
            Self::Confirmed(value) => Some(value),
            _ => None,
        }
    }
}

/// A reusable keyboard prompt adapter over injected input.
#[derive(Clone, Debug)]
pub struct KeyboardPrompt<I> {
    input: I,
    tty_mode: TtyMode,
    render_options: RenderOptions,
}

impl<I> KeyboardPrompt<I> {
    /// Creates an adapter with explicit stream mode.
    #[must_use]
    pub const fn new(input: I, tty_mode: TtyMode) -> Self {
        Self {
            input,
            tty_mode,
            render_options: RenderOptions::plain_text(),
        }
    }

    /// Returns a copy using caller-selected render options.
    #[must_use]
    pub const fn with_render_options(mut self, render_options: RenderOptions) -> Self {
        self.render_options = render_options;
        self
    }

    /// Returns the explicit stream mode.
    #[must_use]
    pub const fn tty_mode(&self) -> TtyMode {
        self.tty_mode
    }

    /// Returns read-only access to the injected input.
    #[must_use]
    pub const fn input(&self) -> &I {
        &self.input
    }
}

impl<I: PromptInput> KeyboardPrompt<I> {
    /// Renders the complete exact view and keyboard instructions.
    #[must_use]
    pub fn prompt_text(&self, request: &PromptRequest) -> String {
        let (preview, instruction) = match request {
            PromptRequest::Activation { preview, syntax } => {
                let text = render_activation(preview, self.render_options);
                let instruction = match syntax {
                    ConfirmationSyntax::YesNo => format!(
                        "{} — keyboard: Y/yes = confirm exact activation, N/no/Esc = cancel, Enter = cancel (default).",
                        PromptAction::ActivatePolicy
                    ),
                    ConfirmationSyntax::ExactScope => format!(
                        "{} — keyboard: type `activate <repository> <activation-id> <config-hash> <tuple-hash> <tuple>` to confirm this exact activation, N/Esc = cancel, Enter = cancel (default).",
                        PromptAction::ActivatePolicy
                    ),
                };
                (text, instruction)
            }
            PromptRequest::Purge { plan, syntax } => {
                let text = render_purge_plan(plan, self.render_options);
                let instruction = match syntax {
                    ConfirmationSyntax::YesNo => format!(
                        "{} — keyboard: Y/yes = confirm exact purge plan, N/no/Esc = cancel, Enter = cancel (default).",
                        PromptAction::ConfirmPurge
                    ),
                    ConfirmationSyntax::ExactScope => format!(
                        "{} — keyboard: type `purge <repository> <scope> <cutoff-unix> <config-hash> <plan-hash>` to confirm this exact plan, N/Esc = cancel, Enter = cancel (default).",
                        PromptAction::ConfirmPurge
                    ),
                };
                (text, instruction)
            }
        };
        let mut text = preview;
        for line in wrap_text(&instruction, self.render_options.columns()) {
            text.push_str(&line);
            text.push('\n');
        }
        for line in wrap_text(
            "Focus: keyboard input; no pointer selection is required.",
            self.render_options.columns(),
        ) {
            text.push_str(&line);
            text.push('\n');
        }
        for line in wrap_text(
            "Prompt result is non-authoritative; the domain must revalidate current state, TTY mode, scope, and hashes.",
            self.render_options.columns(),
        ) {
            text.push_str(&line);
            text.push('\n');
        }
        text
    }

    /// Runs one request. Non-TTY returns before the input dependency is read.
    pub fn request(&mut self, request: &PromptRequest) -> PromptResult {
        if self.tty_mode.is_non_tty() {
            return PromptResult::NonTty;
        }
        let prompt = self.prompt_text(request);
        match self.input.read_line(&prompt) {
            Ok(Some(response)) => parse_response(request, response),
            Ok(None) => PromptResult::InputUnavailable {
                detail: "keyboard input reached end of stream".to_owned(),
            },
            Err(error) => PromptResult::InputUnavailable {
                detail: error.detail,
            },
        }
    }

    /// Runs a normal yes/no policy activation request.
    pub fn request_activation(&mut self, preview: ActivationPreviewView) -> PromptResult {
        self.request(&PromptRequest::activation(preview))
    }

    /// Runs an exact-scope policy activation request.
    pub fn request_exact_activation(&mut self, preview: ActivationPreviewView) -> PromptResult {
        self.request(&PromptRequest::exact_activation(preview))
    }

    /// Compatibility alias for [`Self::request_activation`].
    pub fn request_policy_activation(&mut self, preview: ActivationPreviewView) -> PromptResult {
        self.request_activation(preview)
    }

    /// Runs a normal yes/no purge request.
    pub fn request_purge(&mut self, plan: PurgePlanView) -> PromptResult {
        self.request(&PromptRequest::purge(plan))
    }

    /// Runs an exact-scope purge request.
    pub fn request_exact_purge(&mut self, plan: PurgePlanView) -> PromptResult {
        self.request(&PromptRequest::exact_purge(plan))
    }

    /// Compatibility alias for [`Self::request_purge`].
    pub fn request_purge_confirmation(&mut self, plan: PurgePlanView) -> PromptResult {
        self.request_purge(plan)
    }
}

/// Readable aliases for the two permission-widening prompt surfaces.
pub type PolicyActivationPrompt<I> = KeyboardPrompt<I>;
pub type PurgeConfirmationPrompt<I> = KeyboardPrompt<I>;
/// General name for an injected operations prompt adapter.
pub type PromptAdapter<I> = KeyboardPrompt<I>;

fn parse_response(request: &PromptRequest, response: String) -> PromptResult {
    let trimmed = response.trim();
    if trimmed.is_empty() {
        return PromptResult::Defaulted;
    }
    let lower = trimmed.to_ascii_lowercase();
    let syntax = match request {
        PromptRequest::Activation { syntax, .. } | PromptRequest::Purge { syntax, .. } => *syntax,
    };
    if syntax == ConfirmationSyntax::YesNo {
        return match lower.as_str() {
            "y" | "yes" => confirmed_from_request(request),
            "n" | "no" | "q" | "cancel" | "esc" | "\u{1b}" => PromptResult::Cancelled,
            _ => PromptResult::InvalidInput {
                input: "invalid input (redacted)".to_owned(),
            },
        };
    }

    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    match request {
        PromptRequest::Activation { preview, .. } => {
            if parts
                .first()
                .map(|part| part.to_ascii_lowercase())
                .as_deref()
                != Some("activate")
                || parts.len() != 6
            {
                return PromptResult::InvalidInput {
                    input: "invalid input (redacted)".to_owned(),
                };
            }
            let repository_id = parts[1];
            let activation_id = parts[2];
            let config_hash = parts[3];
            let tuple_hash = parts[4];
            let tuple = parts[5];
            if repository_id != preview.repository_id
                || activation_id != preview.activation_id
                || config_hash != preview.config_hash
                || tuple_hash != preview.tuple_hash
                || tuple != preview.tuple.to_string()
            {
                return PromptResult::ScopeMismatch {
                    reason: "activation identity differs from the exact preview".to_owned(),
                };
            }
            confirmed_from_request(request)
        }
        PromptRequest::Purge { plan, .. } => {
            if parts
                .first()
                .map(|part| part.to_ascii_lowercase())
                .as_deref()
                != Some("purge")
                || parts.len() != 6
            {
                return PromptResult::InvalidInput {
                    input: "invalid input (redacted)".to_owned(),
                };
            }
            let repository_id = parts[1];
            let scope = parts[2];
            let cutoff = parts[3];
            let config_hash = parts[4];
            let supplied_plan_hash = parts[5];
            if supplied_plan_hash != plan.plan_hash {
                return PromptResult::PlanHashChanged {
                    expected: plan.plan_hash.clone(),
                    supplied: supplied_plan_hash.to_owned(),
                };
            }
            if repository_id != plan.repository_id
                || scope != plan.scope.as_str()
                || cutoff != plan.cutoff_unix_seconds.to_string()
                || config_hash != plan.config_hash
            {
                return PromptResult::ScopeMismatch {
                    reason: "purge identity differs from the exact plan".to_owned(),
                };
            }
            confirmed_from_request(request)
        }
    }
}

fn confirmed_from_request(request: &PromptRequest) -> PromptResult {
    match request {
        PromptRequest::Activation { preview, .. } => PromptResult::Confirmed(ExactConfirmation {
            action: PromptAction::ActivatePolicy,
            repository_id: preview.repository_id.clone(),
            object_id: preview.activation_id.clone(),
            scope: Some(preview.tuple.to_string()),
            config_hash: Some(preview.config_hash.clone()),
            tuple_hash: Some(preview.tuple_hash.clone()),
            plan_hash: None,
        }),
        PromptRequest::Purge { plan, .. } => PromptResult::Confirmed(ExactConfirmation {
            action: PromptAction::ConfirmPurge,
            repository_id: plan.repository_id.clone(),
            object_id: "purge-plan".to_owned(),
            scope: Some(plan.scope.as_str().to_owned()),
            config_hash: Some(plan.config_hash.clone()),
            tuple_hash: None,
            plan_hash: Some(plan.plan_hash.clone()),
        }),
    }
}

/// Converts a collected exact purge intent into the presentation confirmation
/// value consumed by a command adapter. It still grants no authority.
#[must_use]
pub fn purge_confirmation_from_intent(
    confirmation: &ExactConfirmation,
    identity: &PurgeConfirmationIdentity,
    tty_mode: TtyMode,
) -> Option<PurgeConfirmation> {
    if tty_mode.is_non_tty()
        || !confirmation.is_purge()
        || confirmation.repository_id != identity.repository_id
        || confirmation.scope.as_deref() != Some(identity.scope.as_str())
        || confirmation.config_hash.as_deref() != Some(identity.config_hash.as_str())
        || confirmation.plan_hash.as_deref() != Some(identity.plan_hash.as_str())
    {
        return None;
    }
    Some(PurgeConfirmation::from_identity(identity, tty_mode))
}

/// A stable prompt grammar description useful to command adapters and docs.
#[must_use]
pub fn exact_scope_instructions(
    action: PromptAction,
    identity: &PurgeConfirmationIdentity,
) -> String {
    match action {
        PromptAction::ConfirmPurge => format!(
            "purge {} {} {} {} {}",
            identity.repository_id,
            identity.scope.as_str(),
            identity.cutoff_unix_seconds,
            identity.config_hash,
            identity.plan_hash
        ),
        PromptAction::ActivatePolicy => {
            "activate <repository> <activation-id> <config-hash> <tuple-hash> <tuple>".to_owned()
        }
    }
}

/// Compatibility alias for callers that use the term intent.
pub type ConfirmationIntent = ExactConfirmation;

/// Re-export the operations view enum for prompt consumers that render through
/// the same deterministic renderer.
pub use crate::render::OperationsView as PromptOperationsView;
