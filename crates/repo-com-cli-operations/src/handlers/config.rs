//! Configuration validation command port.

use std::path::PathBuf;

use repo_com_config::{ConfigError, ResolvedConfig};
use repo_com_foundation::RepoComError;
use repo_com_terminal_operations::ConfigStatusView;

use crate::OperationsResult;
use crate::input::ConfigValidationInput;

/// A validated request for the configuration owner.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfigValidationRequest {
    /// Exact repository scope.
    pub repository_id: String,
    /// Optional explicit path selected by the process owner.
    pub config_path: Option<PathBuf>,
}

impl From<ConfigValidationInput> for ConfigValidationRequest {
    fn from(value: ConfigValidationInput) -> Self {
        let config_path = value.path();
        Self {
            repository_id: value.repository_id,
            config_path,
        }
    }
}

/// Domain port for configuration discovery, parsing, and validation.
///
/// The handler supplies the already-resolved configuration and the explicit
/// request identity. The service remains the authority for path discovery,
/// strict schema validation, secret rejection, and alias resolution.
pub trait ConfigService {
    /// Validates and projects one configuration without policy or state work.
    fn validate(
        &mut self,
        config: &ResolvedConfig,
        request: ConfigValidationRequest,
    ) -> OperationsResult<ConfigStatusView>;

    /// Compatibility alias for callers that use the longer method name.
    fn validate_config(
        &mut self,
        config: &ResolvedConfig,
        request: ConfigValidationRequest,
    ) -> OperationsResult<ConfigStatusView> {
        self.validate(config, request)
    }
}

/// Routes one explicit configuration validation request.
pub fn validate<S>(
    service: &mut S,
    config: &ResolvedConfig,
    request: ConfigValidationRequest,
) -> OperationsResult<ConfigStatusView>
where
    S: ConfigService + ?Sized,
{
    require_repository(request.repository_id.as_str(), config)?;
    service.validate(config, request)
}

/// Alias for [`validate`].
pub fn validate_config<S>(
    service: &mut S,
    config: &ResolvedConfig,
    request: ConfigValidationRequest,
) -> OperationsResult<ConfigStatusView>
where
    S: ConfigService + ?Sized,
{
    validate(service, config, request)
}

/// Converts a safe configuration-domain failure into the command category.
#[must_use]
pub fn map_config_error(error: &ConfigError) -> RepoComError {
    error.to_repo_com_error()
}

fn require_repository(repository_id: &str, config: &ResolvedConfig) -> Result<(), RepoComError> {
    if repository_id == config.config.repository_id {
        Ok(())
    } else {
        Err(RepoComError::usage(
            "configuration repository_id does not match the explicit resolved repository",
        ))
    }
}
