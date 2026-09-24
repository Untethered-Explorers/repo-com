//! Send and read-only setup command ports.
//!
//! The handler validates exact identity and delegates all eligibility,
//! claiming, transport, and remote setup rules to the supplied domain owner.

use repo_com_config::ResolvedConfig;
use repo_com_discord_client::SetupReport;
use repo_com_foundation::{RepoComError, TtyMode};
use repo_com_terminal_outbound::DeliveryView;
use serde::{Deserialize, Serialize};

use crate::input::{SendInput, SetupCheckInput};
use crate::{DraftIdentity, MessagingResult};

/// Validated send command passed to the delivery owner.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SendRequest {
    /// Repository scope.
    pub repository_id: String,
    /// Draft identity.
    pub draft_id: String,
    /// Exact immutable revision.
    pub revision: u64,
    /// Explicit stream mode used by the domain eligibility decision.
    pub tty_mode: TtyMode,
    /// Injected current time used by the domain decision.
    pub now_unix_seconds: u64,
}

impl SendRequest {
    /// Adds the explicit process context to a parsed send input.
    #[must_use]
    pub fn from_input(value: SendInput, tty_mode: TtyMode, now_unix_seconds: u64) -> Self {
        Self {
            repository_id: value.repository_id,
            draft_id: value.draft_id,
            revision: value.revision,
            tty_mode,
            now_unix_seconds,
        }
    }

    /// Returns the exact draft identity represented by this request.
    #[must_use]
    pub fn identity(&self) -> DraftIdentity {
        DraftIdentity {
            repository_id: self.repository_id.clone(),
            draft_id: self.draft_id.clone(),
            revision: self.revision,
        }
    }
}

/// Domain port that retains eligibility, atomic claim, and delivery safety.
#[allow(async_fn_in_trait)]
pub trait SendService {
    /// Dispatches one exact send through the complete domain pipeline.
    async fn send(
        &mut self,
        request: SendRequest,
        config: &ResolvedConfig,
    ) -> MessagingResult<DeliveryView>;
}

/// Domain port for the read-only Discord setup check.
#[allow(async_fn_in_trait)]
pub trait SetupService {
    /// Performs the read-only setup check for one resolved repository.
    async fn check_setup(&mut self, config: &ResolvedConfig) -> MessagingResult<SetupReport>;
}

/// Repository-scoped setup result retained by the command layer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetupCheckView {
    /// Exact repository identifier supplied to the command.
    pub repository_id: String,
    /// Canonical hash of the resolved configuration used for the check.
    pub config_hash: String,
    /// Complete redacted Discord setup report.
    pub report: SetupReport,
}

/// Sends one exact revision through the supplied domain owner.
pub async fn send<S>(
    service: &mut S,
    request: SendRequest,
    config: &ResolvedConfig,
) -> MessagingResult<DeliveryView>
where
    S: SendService + ?Sized,
{
    if request.repository_id != config.config.repository_id {
        return Err(RepoComError::usage(
            "send repository does not match the resolved configuration",
        ));
    }
    service.send(request, config).await
}

/// Runs setup after checking that the caller supplied the exact repository.
pub async fn setup_check<S>(
    service: &mut S,
    input: &SetupCheckInput,
    config: &ResolvedConfig,
) -> MessagingResult<SetupCheckView>
where
    S: SetupService + ?Sized,
{
    if input.repository_id != config.config.repository_id {
        return Err(RepoComError::usage(
            "setup repository does not match the resolved configuration",
        ));
    }
    let report = service.check_setup(config).await?;
    Ok(SetupCheckView {
        repository_id: config.config.repository_id.clone(),
        config_hash: config.canonical_hash(),
        report,
    })
}
