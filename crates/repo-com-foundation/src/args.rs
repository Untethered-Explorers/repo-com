use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::PathBuf;

use crate::error::RepoComError;

/// The output representation selected by a command.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum OutputFormat {
    /// Human-readable labeled output.
    #[default]
    Human,
    /// One protocol-version-1 JSON object.
    Json,
}

impl fmt::Display for OutputFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Human => "human",
            Self::Json => "json",
        })
    }
}

/// The color policy selected by a command.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum ColorChoice {
    /// Let the presentation layer choose based on its explicit terminal mode.
    #[default]
    Auto,
    /// Request color output.
    Always,
    /// Request plain-text output.
    Never,
}

impl ColorChoice {
    /// Returns whether this choice permits color output.
    #[must_use]
    pub const fn permits_color(self) -> bool {
        matches!(self, Self::Auto | Self::Always)
    }
}

impl fmt::Display for ColorChoice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Auto => "auto",
            Self::Always => "always",
            Self::Never => "never",
        })
    }
}

/// The diagnostics policy selected by a command.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum DiagnosticsChoice {
    /// Do not emit diagnostics.
    #[default]
    Off,
    /// Allow opt-in diagnostics on the presentation layer's diagnostic stream.
    On,
}

impl DiagnosticsChoice {
    /// Returns whether diagnostics were explicitly enabled.
    #[must_use]
    pub const fn enabled(self) -> bool {
        matches!(self, Self::On)
    }
}

impl fmt::Display for DiagnosticsChoice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Off => "off",
            Self::On => "on",
        })
    }
}

/// Global options shared by command handlers and the final executable.
///
/// This type only carries values. It does not probe the process environment,
/// open files, contact a terminal, or perform any other external I/O.
#[derive(Args, Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct GlobalArgs {
    /// Optional explicit configuration path.
    #[arg(long = "config", global = true, value_name = "PATH")]
    pub config_path: Option<PathBuf>,

    /// Output representation for the command.
    #[arg(long = "output", value_enum, default_value = "human", global = true)]
    pub output_format: OutputFormat,

    /// Color policy for human presentation.
    #[arg(long = "color", value_enum, default_value = "auto", global = true)]
    pub color: ColorChoice,

    /// Whether opt-in diagnostics may be emitted.
    #[arg(long = "diagnostics", value_enum, default_value = "off", global = true)]
    pub diagnostics: DiagnosticsChoice,
}

impl GlobalArgs {
    /// Returns the default global options.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            config_path: None,
            output_format: OutputFormat::Human,
            color: ColorChoice::Auto,
            diagnostics: DiagnosticsChoice::Off,
        }
    }

    /// Returns a copy with an explicit configuration path.
    #[must_use]
    pub fn with_config_path(path: impl Into<PathBuf>) -> Self {
        Self {
            config_path: Some(path.into()),
            ..Self::new()
        }
    }

    /// Returns whether this invocation selected machine output.
    #[must_use]
    pub const fn is_machine_mode(&self) -> bool {
        matches!(self.output_format, OutputFormat::Json)
    }

    /// Returns whether diagnostics were explicitly enabled.
    #[must_use]
    pub const fn diagnostics_enabled(&self) -> bool {
        self.diagnostics.enabled()
    }
}

/// The terminal mode supplied by the caller to an interactive boundary.
///
/// A non-TTY value is the safe default. The foundation does not inspect stdin
/// or stdout; callers pass the result of their own explicit stream decision.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TtyMode {
    /// Both relevant streams are interactive.
    Tty,
    /// At least one relevant stream is non-interactive.
    #[default]
    NonTty,
}

impl TtyMode {
    /// A readable alias for callers that use “interactive” terminology.
    pub const INTERACTIVE: Self = Self::Tty;

    /// A readable alias for callers that use “non-interactive” terminology.
    pub const NON_INTERACTIVE: Self = Self::NonTty;

    /// Combines explicit stdin and stdout stream decisions.
    #[must_use]
    pub const fn from_stream_states(stdin_is_tty: bool, stdout_is_tty: bool) -> Self {
        if stdin_is_tty && stdout_is_tty {
            Self::Tty
        } else {
            Self::NonTty
        }
    }

    /// Returns whether both relevant streams are interactive.
    #[must_use]
    pub const fn is_tty(self) -> bool {
        matches!(self, Self::Tty)
    }

    /// Returns whether at least one relevant stream is non-interactive.
    #[must_use]
    pub const fn is_non_tty(self) -> bool {
        matches!(self, Self::NonTty)
    }

    /// Returns whether an interactive prompt may be requested.
    #[must_use]
    pub const fn can_prompt(self) -> bool {
        self.is_tty()
    }

    /// Returns whether an interactive prompt may be requested.
    #[must_use]
    pub const fn allows_prompt(self) -> bool {
        self.can_prompt()
    }

    /// Enforces the fail-closed non-TTY prompt boundary without prompting.
    pub fn require_prompt_allowed(self) -> Result<(), RepoComError> {
        if self.can_prompt() {
            Ok(())
        } else {
            Err(RepoComError::operator_action_required(
                "interactive operator action requires a TTY",
            ))
        }
    }

    /// Alias for [`TtyMode::require_prompt_allowed`].
    pub fn request_prompt(self) -> Result<(), RepoComError> {
        self.require_prompt_allowed()
    }

    /// Returns the stable serialized spelling of this mode.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tty => "tty",
            Self::NonTty => "non-tty",
        }
    }
}

impl fmt::Display for TtyMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Compatibility name for consumers that call the color policy a mode.
pub type ColorMode = ColorChoice;

/// Compatibility name for consumers that call diagnostics a mode.
pub type DiagnosticsMode = DiagnosticsChoice;
