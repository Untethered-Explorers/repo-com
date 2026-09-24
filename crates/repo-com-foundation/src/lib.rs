#![forbid(unsafe_code)]

pub mod args;
pub mod error;
pub mod protocol;

#[cfg(test)]
#[path = "../tests/foundation_contract.rs"]
mod foundation_contract;

pub use args::{
    ColorChoice, ColorMode, DiagnosticsChoice, DiagnosticsMode, GlobalArgs, OutputFormat, TtyMode,
};
pub use error::{ErrorCategory, ErrorCode, ProcessCategory, RepoComError};
pub use protocol::{
    CommandOutcome, CommandStatus, OutcomeStatus, OutputBundle, OutputStreams, PROTOCOL_VERSION,
    ProtocolOutput,
};
