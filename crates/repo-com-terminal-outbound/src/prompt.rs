//! Keyboard-only, non-authoritative outbound prompt adapters.
//!
//! A prompt collects an operator's exact response.  It does not read or mutate
//! repository state, call a domain service, or create approval/override
//! authority.  The command/domain layer must revalidate the preview hash,
//! revision, expiry, current state, and TTY mode after this adapter returns.

use std::collections::VecDeque;
use std::fmt;
use std::io::{self, BufRead, Write};

use repo_com_foundation::TtyMode;
use serde::{Deserialize, Serialize};

use crate::preview::OutboundPreview;
use crate::render::{RenderOptions, render_preview};
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

/// A keyboard input dependency.  Implementations should read one line from an
/// interactive terminal; tests can inject deterministic scripted input.
pub trait PromptInput {
    /// Reads one line, returning `None` at end of input.
    fn read_line(&mut self, prompt: &str) -> Result<Option<String>, PromptInputError>;
}

/// Production stdin adapter.  It is called only after a TTY check succeeds.
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

/// A deterministic keyboard input useful to adapters and contract tests.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ScriptedPromptInput {
    lines: VecDeque<String>,
    prompts: Vec<String>,
}

impl ScriptedPromptInput {
    /// Creates a scripted input from lines in keyboard order.
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

    /// Returns all prompt text observed by this input.
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

/// The permission-widening action being confirmed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromptAction {
    /// Confirm the complete exact preview for human approval.
    Approve,
    /// Confirm an exact redacted secret-finding override.
    OverrideSecretFinding,
}

impl PromptAction {
    /// Returns the action label shown in the prompt.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "Approve exact preview",
            Self::OverrideSecretFinding => "Override exact secret finding",
        }
    }

    /// Returns the exact token accepted in hash-binding mode.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::OverrideSecretFinding => "override",
        }
    }
}

impl fmt::Display for PromptAction {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The response grammar used by a keyboard prompt.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ConfirmationSyntax {
    /// `y`/`yes` confirms after the complete preview; Enter defaults to no.
    #[default]
    YesNo,
    /// The operator must type the action plus the exact preview hash.
    ExactHash,
}

/// The stable identity displayed immediately before an operator response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactPreviewIdentity {
    /// Repository identity.
    pub repository_id: String,
    /// Draft identity.
    pub draft_id: String,
    /// Immutable revision number.
    pub revision: u64,
    /// Complete preview hash.
    pub preview_hash: String,
    /// Exclusive expiry boundary.
    pub expires_at_unix_seconds: u64,
}

impl ExactPreviewIdentity {
    /// Projects identity from a complete preview.
    #[must_use]
    pub fn from_preview(preview: &OutboundPreview) -> Self {
        Self {
            repository_id: preview.repository_id.clone(),
            draft_id: preview.draft_id.clone(),
            revision: preview.revision,
            preview_hash: preview.preview_hash.clone(),
            expires_at_unix_seconds: preview.expires_at_unix_seconds,
        }
    }
}

/// A caller-owned prompt request.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PromptRequest {
    /// Complete preview to render before asking for input.
    pub preview: OutboundPreview,
    /// Action being confirmed.
    pub action: PromptAction,
    /// Response grammar.
    pub syntax: ConfirmationSyntax,
}

impl PromptRequest {
    /// Creates a normal yes/no approval request.
    #[must_use]
    pub fn approval(preview: OutboundPreview) -> Self {
        Self {
            preview,
            action: PromptAction::Approve,
            syntax: ConfirmationSyntax::YesNo,
        }
    }

    /// Creates an exact-hash approval request.
    #[must_use]
    pub fn exact_approval(preview: OutboundPreview) -> Self {
        Self {
            preview,
            action: PromptAction::Approve,
            syntax: ConfirmationSyntax::ExactHash,
        }
    }

    /// Creates an exact-hash secret override request.
    #[must_use]
    pub fn secret_override(preview: OutboundPreview) -> Self {
        Self {
            preview,
            action: PromptAction::OverrideSecretFinding,
            syntax: ConfirmationSyntax::ExactHash,
        }
    }
}

/// A non-authoritative exact confirmation intent returned to the caller.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExactConfirmation {
    /// Action that was requested.
    pub action: PromptAction,
    /// Repository identity shown in the preview.
    pub repository_id: String,
    /// Draft identity shown in the preview.
    pub draft_id: String,
    /// Revision shown in the preview.
    pub revision: u64,
    /// Hash the domain must revalidate.
    pub preview_hash: String,
}

impl ExactConfirmation {
    /// Returns whether this intent is bound to the supplied identity.
    #[must_use]
    pub fn matches(&self, identity: &ExactPreviewIdentity) -> bool {
        self.repository_id == identity.repository_id
            && self.draft_id == identity.draft_id
            && self.revision == identity.revision
            && self.preview_hash == identity.preview_hash
    }
}

/// A typed keyboard prompt result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PromptResult {
    /// The operator supplied the requested confirmation.
    Confirmed(ExactConfirmation),
    /// The operator explicitly cancelled.
    Cancelled,
    /// The operator pressed Enter; the safe default is cancellation.
    Defaulted,
    /// The response was not in the documented grammar.
    InvalidInput {
        /// The redacted input value, if it was safe to retain.
        input: String,
    },
    /// The exact hash did not match the displayed preview.
    PreviewHashMismatch {
        /// The supplied hash, if present.
        supplied: Option<String>,
    },
    /// The preview expiry boundary had already been reached.
    Expired,
    /// The adapter was unavailable because the stream was non-interactive.
    NonTty,
    /// The input dependency failed or reached EOF.
    InputUnavailable {
        /// Redacted input detail.
        detail: String,
    },
}

impl PromptResult {
    /// Returns whether the operator confirmed the exact preview.
    #[must_use]
    pub const fn is_confirmed(&self) -> bool {
        matches!(self, Self::Confirmed(_))
    }

    /// Returns whether the operator explicitly cancelled.
    #[must_use]
    pub const fn is_cancelled(&self) -> bool {
        matches!(self, Self::Cancelled)
    }

    /// Returns whether the safe Enter default was used.
    #[must_use]
    pub const fn is_defaulted(&self) -> bool {
        matches!(self, Self::Defaulted)
    }

    /// Returns whether input was invalid or hash-mismatched.
    #[must_use]
    pub const fn is_invalid(&self) -> bool {
        matches!(
            self,
            Self::InvalidInput { .. } | Self::PreviewHashMismatch { .. }
        )
    }

    /// Returns whether expiry was detected before input.
    #[must_use]
    pub const fn is_expired(&self) -> bool {
        matches!(self, Self::Expired)
    }

    /// Returns whether the prompt was unavailable because of a non-TTY stream.
    #[must_use]
    pub const fn is_non_tty(&self) -> bool {
        matches!(self, Self::NonTty)
    }

    /// Returns the exact confirmation intent, if any.
    #[must_use]
    pub const fn confirmation(&self) -> Option<&ExactConfirmation> {
        match self {
            Self::Confirmed(confirmation) => Some(confirmation),
            _ => None,
        }
    }
}

/// A reusable keyboard prompt adapter over an injected input dependency.
#[derive(Clone, Debug)]
pub struct KeyboardPrompt<I> {
    input: I,
    tty_mode: TtyMode,
    now_unix_seconds: u64,
    render_options: RenderOptions,
}

impl<I> KeyboardPrompt<I> {
    /// Creates a prompt adapter with an explicit stream mode and time.
    #[must_use]
    pub const fn new(input: I, tty_mode: TtyMode, now_unix_seconds: u64) -> Self {
        Self {
            input,
            tty_mode,
            now_unix_seconds,
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

    /// Returns the injected current time.
    #[must_use]
    pub const fn now_unix_seconds(&self) -> u64 {
        self.now_unix_seconds
    }

    /// Returns read-only access to the input dependency.
    #[must_use]
    pub const fn input(&self) -> &I {
        &self.input
    }
}

impl<I: PromptInput> KeyboardPrompt<I> {
    /// Renders the complete exact preview and keyboard instructions.
    #[must_use]
    pub fn prompt_text(&self, request: &PromptRequest) -> String {
        let mut text = render_preview(&request.preview, self.render_options);
        let instruction = match request.syntax {
            ConfirmationSyntax::YesNo => {
                format!(
                    "{} — keyboard: Y/yes = confirm exact preview, N/no/Esc = cancel, Enter = cancel (default).",
                    request.action
                )
            }
            ConfirmationSyntax::ExactHash => {
                format!(
                    "{} — keyboard: type `{} <preview-hash>` to confirm this exact preview, N/Esc = cancel, Enter = cancel (default).",
                    request.action,
                    request.action.token()
                )
            }
        };
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
            "Prompt result is non-authoritative; the domain must revalidate current state.",
            self.render_options.columns(),
        ) {
            text.push_str(&line);
            text.push('\n');
        }
        text
    }

    /// Runs one keyboard request.  Non-TTY and expired previews return before
    /// the input dependency is touched.
    pub fn request(&mut self, request: &PromptRequest) -> PromptResult {
        if self.tty_mode.is_non_tty() {
            return PromptResult::NonTty;
        }
        if request.preview.is_prompt_expired_at(self.now_unix_seconds) {
            return PromptResult::Expired;
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

    /// Runs a normal yes/no approval request.
    pub fn request_approval(&mut self, preview: OutboundPreview) -> PromptResult {
        self.request(&PromptRequest::approval(preview))
    }

    /// Runs an exact-hash approval request.
    pub fn request_exact_approval(&mut self, preview: OutboundPreview) -> PromptResult {
        self.request(&PromptRequest::exact_approval(preview))
    }

    /// Runs an exact-hash secret override request.
    pub fn request_secret_override(&mut self, preview: OutboundPreview) -> PromptResult {
        self.request(&PromptRequest::secret_override(preview))
    }
}

/// Readable aliases for the two permission-widening prompt surfaces.
pub type ApprovalPrompt<I> = KeyboardPrompt<I>;
/// Readable alias for the secret override prompt surface.
pub type SecretOverridePrompt<I> = KeyboardPrompt<I>;
/// General name for the injected keyboard prompt adapter.
pub type PromptAdapter<I> = KeyboardPrompt<I>;

fn parse_response(request: &PromptRequest, response: String) -> PromptResult {
    let trimmed = response.trim();
    if trimmed.is_empty() {
        return PromptResult::Defaulted;
    }
    let lower = trimmed.to_ascii_lowercase();
    let identity = ExactPreviewIdentity::from_preview(&request.preview);
    let confirmation = || ExactConfirmation {
        action: request.action,
        repository_id: identity.repository_id.clone(),
        draft_id: identity.draft_id.clone(),
        revision: identity.revision,
        preview_hash: identity.preview_hash.clone(),
    };

    match request.syntax {
        ConfirmationSyntax::YesNo => match lower.as_str() {
            "y" | "yes" => PromptResult::Confirmed(confirmation()),
            "n" | "no" | "q" | "cancel" | "esc" | "\u{1b}" => PromptResult::Cancelled,
            _ => PromptResult::InvalidInput {
                input: safe_input_value(trimmed),
            },
        },
        ConfirmationSyntax::ExactHash => {
            let mut parts = trimmed.split_whitespace();
            let action = parts.next().unwrap_or_default().to_ascii_lowercase();
            let supplied = parts.next().map(str::to_owned);
            if parts.next().is_some() || action != request.action.token() {
                return PromptResult::InvalidInput {
                    input: safe_input_value(trimmed),
                };
            }
            match supplied {
                None => PromptResult::InvalidInput {
                    input: safe_input_value(trimmed),
                },
                Some(supplied) if supplied == identity.preview_hash => {
                    PromptResult::Confirmed(confirmation())
                }
                Some(supplied) => PromptResult::PreviewHashMismatch {
                    supplied: Some(supplied),
                },
            }
        }
    }
}

fn safe_input_value(_value: &str) -> String {
    "invalid input (redacted)".to_owned()
}

#[cfg(test)]
mod tests {
    use super::{ConfirmationSyntax, PromptAction, PromptInputError, ScriptedPromptInput};
    use crate::preview::{
        ApprovalView, DestinationView, MetadataView, OutboundPreview, PolicyView, Provenance,
        SafetyView,
    };
    use crate::prompt::{ExactPreviewIdentity, KeyboardPrompt, PromptRequest, PromptResult};
    use repo_com_foundation::TtyMode;

    fn preview() -> OutboundPreview {
        OutboundPreview {
            repository_id: "acme/widgets".to_owned(),
            draft_id: "draft-1".to_owned(),
            revision: 3,
            revision_hash: "a".repeat(64),
            destination: DestinationView::new("release", "100", "200", Vec::new()),
            exact_text: "please investigate".to_owned(),
            exact_text_hash: "b".repeat(64),
            metadata: MetadataView::default(),
            event_type: "build_failed".to_owned(),
            severity: "high".to_owned(),
            reply_reference: None,
            created_at_unix_seconds: 1_000,
            expires_at_unix_seconds: 2_000,
            approval: ApprovalView::default(),
            policy: PolicyView::default(),
            safety: SafetyView::default(),
            preview_hash: "c".repeat(64),
            provenance: Provenance::LocalDecision,
        }
    }

    #[test]
    fn exact_hash_confirmation_is_bound_to_identity() {
        let input = ScriptedPromptInput::new([format!("approve {}", "c".repeat(64))]);
        let mut prompt = KeyboardPrompt::new(input, TtyMode::Tty, 1_500);
        let result = prompt.request(&PromptRequest::exact_approval(preview()));
        let confirmation = result.confirmation().expect("confirmation");
        assert!(confirmation.matches(&ExactPreviewIdentity::from_preview(&preview())));
        assert_eq!(confirmation.action, PromptAction::Approve);
    }

    #[test]
    fn non_tty_never_reads_input() {
        let input = ScriptedPromptInput::new(["y"]);
        let mut prompt = KeyboardPrompt::new(input, TtyMode::NonTty, 1_500);
        let result = prompt.request(&PromptRequest::approval(preview()));
        assert_eq!(result, PromptResult::NonTty);
        assert_eq!(prompt.input().calls(), 0);
    }

    #[test]
    fn input_error_is_typed_and_not_authority() {
        struct FailingInput;
        impl super::PromptInput for FailingInput {
            fn read_line(&mut self, _prompt: &str) -> Result<Option<String>, PromptInputError> {
                Err(PromptInputError::new("synthetic failure"))
            }
        }
        let mut prompt = KeyboardPrompt::new(FailingInput, TtyMode::Tty, 1_500);
        let result = prompt.request(&PromptRequest::approval(preview()));
        assert!(matches!(result, PromptResult::InputUnavailable { .. }));
    }

    #[test]
    fn exact_hash_syntax_rejects_a_short_yes() {
        let input = ScriptedPromptInput::new(["yes"]);
        let mut prompt = KeyboardPrompt::new(input, TtyMode::Tty, 1_500);
        let result = prompt.request(&PromptRequest {
            preview: preview(),
            action: PromptAction::Approve,
            syntax: ConfirmationSyntax::ExactHash,
        });
        assert!(matches!(result, PromptResult::InvalidInput { .. }));
    }
}
