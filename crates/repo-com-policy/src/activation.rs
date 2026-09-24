use std::path::PathBuf;

use repo_com_config::RepositoryConfig;
use repo_com_foundation::{RepoComError, TtyMode};
use repo_com_state::{PolicyActivationInput, PolicyActivationRecord, StateError, StateStore};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::PolicyError;
use crate::evaluate::{
    PolicyDecision, PolicyStatus, classify_activation_records, decision_from_status,
    find_exact_policy, validate_policy_configuration,
};
use crate::hash::{
    PolicyHashes, PolicyTuple, canonical_config_hash, canonical_tuple_hash, sha256_hex,
};

/// A caller-supplied operator confirmation boundary.
///
/// The domain service never probes a terminal and never prompts itself. The
/// command/UI layer supplies the explicit mode after collecting the exact
/// operator confirmation. A non-TTY mode remains representable so activation
/// can fail closed without a prompt.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct OperatorConfirmation {
    tty_mode: TtyMode,
}

impl OperatorConfirmation {
    /// Records the explicit TTY mode. This does not itself prompt or mutate
    /// state.
    #[must_use]
    pub const fn from_tty(tty_mode: TtyMode) -> Self {
        Self { tty_mode }
    }

    /// Creates a confirmation only when the explicit mode permits prompting.
    pub fn confirmed(tty_mode: TtyMode) -> Result<Self, PolicyError> {
        tty_mode
            .require_prompt_allowed()
            .map_err(|_| PolicyError::TtyRequired)?;
        Ok(Self::from_tty(tty_mode))
    }

    /// Returns the explicit mode supplied by the caller.
    #[must_use]
    pub const fn tty_mode(self) -> TtyMode {
        self.tty_mode
    }

    /// Enforces the TTY requirement without probing ambient process state.
    pub fn require_tty(self) -> Result<(), PolicyError> {
        if self.tty_mode.is_tty() {
            Ok(())
        } else {
            Err(PolicyError::TtyRequired)
        }
    }
}

impl From<TtyMode> for OperatorConfirmation {
    fn from(tty_mode: TtyMode) -> Self {
        Self::from_tty(tty_mode)
    }
}

impl From<&TtyMode> for OperatorConfirmation {
    fn from(tty_mode: &TtyMode) -> Self {
        Self::from_tty(*tty_mode)
    }
}

/// A complete, non-persisted preview of the policy action an operator must
/// confirm. The hashes are the values persisted and later revalidated.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActivationPreview {
    /// Repository scope.
    pub repository_id: String,
    /// Exact policy tuple.
    pub tuple: PolicyTuple,
    /// Complete normalized configuration hash.
    pub config_hash: String,
    /// Exact tuple hash.
    pub tuple_hash: String,
    /// Stable activation identifier that will be used when no ID is supplied.
    pub activation_id: String,
}

impl ActivationPreview {
    /// Returns the hashes to display in an exact operator preview.
    #[must_use]
    pub fn hashes(&self) -> PolicyHashes {
        PolicyHashes {
            config_hash: self.config_hash.clone(),
            tuple_hash: self.tuple_hash.clone(),
        }
    }

    /// Compatibility alias returning the preview hash pair.
    #[must_use]
    pub fn hashes_owned(&self) -> PolicyHashes {
        self.hashes()
    }
}

/// The result of a successful policy activation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActivationReceipt {
    /// Stable activation identifier.
    pub activation_id: String,
    /// Repository scope.
    pub repository_id: String,
    /// Exact tuple activated by the operator.
    pub tuple: PolicyTuple,
    /// Canonical complete configuration hash persisted in state.
    pub config_hash: String,
    /// Canonical exact tuple hash persisted in state.
    pub tuple_hash: String,
    /// Activation timestamp persisted in state.
    pub activated_at: String,
}

impl ActivationReceipt {
    fn from_record(record: &PolicyActivationRecord) -> Self {
        Self {
            activation_id: record.activation_id.clone(),
            repository_id: record.repository_id.clone(),
            tuple: PolicyTuple::new(
                record.event_type.clone(),
                record.destination_alias.clone(),
                record.severity.clone(),
            ),
            config_hash: record.config_hash.clone(),
            tuple_hash: record.policy_tuple_hash.clone(),
            activated_at: record.activated_at.clone(),
        }
    }
}

/// A backend that can supply the repository-scoped state store.
///
/// Implementations are provided for an owned [`StateStore`] and for a mutable
/// borrow, allowing the same registry API to own user state or compose with a
/// caller that already owns it.
pub trait PolicyStateBackend {
    /// Returns a read-only state handle.
    fn state(&self) -> &StateStore;

    /// Returns a mutable state handle.
    fn state_mut(&mut self) -> &mut StateStore;
}

impl PolicyStateBackend for StateStore {
    fn state(&self) -> &StateStore {
        self
    }

    fn state_mut(&mut self) -> &mut StateStore {
        self
    }
}

impl PolicyStateBackend for &mut StateStore {
    fn state(&self) -> &StateStore {
        self
    }

    fn state_mut(&mut self) -> &mut StateStore {
        self
    }
}

/// Reads the current status of one exact tuple from a shared state store.
///
/// This is the read-only automation boundary. It has no mutation method and
/// never requires a TTY.
pub fn status_from_state(
    state: &StateStore,
    config: &RepositoryConfig,
    tuple: &PolicyTuple,
) -> Result<PolicyStatus, PolicyError> {
    validate_policy_configuration(config)?;
    tuple
        .validate()
        .map_err(|_| PolicyError::InvalidPolicyTuple)?;
    let configured = find_exact_policy(config, tuple)?.is_some();
    let current = PolicyHashes {
        config_hash: canonical_config_hash(config),
        tuple_hash: canonical_tuple_hash(tuple),
    };
    let records = query_records(state.connection(), &config.repository_id, state.path())?;
    Ok(classify_activation_records(
        &records,
        &config.repository_id,
        tuple,
        &current,
        configured,
    ))
}

/// Evaluates one exact tuple using a shared state store without mutation.
pub fn evaluate_from_state(
    state: &StateStore,
    config: &RepositoryConfig,
    tuple: &PolicyTuple,
) -> Result<PolicyDecision, PolicyError> {
    validate_policy_configuration(config)?;
    tuple
        .validate()
        .map_err(|_| PolicyError::InvalidPolicyTuple)?;
    if find_exact_policy(config, tuple)?.is_none() {
        return match status_from_state(state, config, tuple)? {
            PolicyStatus::Stale(snapshot) => Ok(PolicyDecision::Stale(snapshot)),
            PolicyStatus::Deactivated(snapshot) => Ok(PolicyDecision::Deactivated(snapshot)),
            _ => Ok(PolicyDecision::NoExactMatch),
        };
    }
    Ok(decision_from_status(status_from_state(
        state, config, tuple,
    )?))
}

/// Lists repository-scoped activation rows using a shared state store.
pub fn activation_records_from_state(
    state: &StateStore,
    repository_id: impl AsRef<str>,
) -> Result<Vec<PolicyActivationRecord>, PolicyError> {
    query_records(state.connection(), repository_id.as_ref(), state.path())
}

/// Exact policy activation and status registry over repository-scoped state.
pub struct PolicyRegistry<B = StateStore> {
    backend: B,
}

impl<B> PolicyRegistry<B>
where
    B: PolicyStateBackend,
{
    /// Creates a registry over an owned or mutably borrowed state backend.
    pub fn new(backend: B) -> Self {
        Self { backend }
    }

    /// Returns the underlying state store for read-only composition.
    pub fn state(&self) -> &StateStore {
        self.backend.state()
    }

    /// Returns the underlying state store for explicit repository setup.
    pub fn state_mut(&mut self) -> &mut StateStore {
        self.backend.state_mut()
    }

    /// Consumes the registry and returns its backend.
    pub fn into_backend(self) -> B {
        self.backend
    }

    /// Builds an exact activation preview without changing state.
    pub fn preview(
        &self,
        config: &RepositoryConfig,
        tuple: &PolicyTuple,
        activation_id: Option<&str>,
    ) -> Result<ActivationPreview, PolicyError> {
        validate_policy_configuration(config)?;
        tuple
            .validate()
            .map_err(|_| PolicyError::InvalidPolicyTuple)?;
        if find_exact_policy(config, tuple)?.is_none() {
            return Err(PolicyError::PolicyNotConfigured);
        }
        let config_hash = canonical_config_hash(config);
        let tuple_hash = canonical_tuple_hash(tuple);
        let activation_id = activation_id
            .map(str::to_owned)
            .unwrap_or_else(|| default_activation_id(&config_hash, &tuple_hash));
        validate_activation_id(&activation_id)?;
        Ok(ActivationPreview {
            repository_id: config.repository_id.clone(),
            tuple: tuple.clone(),
            config_hash,
            tuple_hash,
            activation_id,
        })
    }

    /// Activates one exact configured tuple through an explicit TTY operator
    /// confirmation and persists its canonical hashes and timestamp.
    pub fn activate<C>(
        &mut self,
        config: &RepositoryConfig,
        tuple: &PolicyTuple,
        confirmation: C,
        activated_at: impl Into<String>,
    ) -> Result<ActivationReceipt, PolicyError>
    where
        C: Into<OperatorConfirmation>,
    {
        self.activate_with_optional_id(
            config,
            tuple,
            None,
            confirmation.into(),
            activated_at.into(),
        )
    }

    /// Compatibility alias for [`PolicyRegistry::activate`].
    pub fn activate_policy<C>(
        &mut self,
        config: &RepositoryConfig,
        tuple: &PolicyTuple,
        confirmation: C,
        activated_at: impl Into<String>,
    ) -> Result<ActivationReceipt, PolicyError>
    where
        C: Into<OperatorConfirmation>,
    {
        self.activate(config, tuple, confirmation, activated_at)
    }

    /// Activates with a caller-selected stable ID. An existing active exact
    /// binding is returned idempotently; reusing an ID for a different or
    /// inactive binding fails closed.
    pub fn activate_with_id<C>(
        &mut self,
        config: &RepositoryConfig,
        tuple: &PolicyTuple,
        activation_id: impl Into<String>,
        confirmation: C,
        activated_at: impl Into<String>,
    ) -> Result<ActivationReceipt, PolicyError>
    where
        C: Into<OperatorConfirmation>,
    {
        self.activate_with_optional_id(
            config,
            tuple,
            Some(activation_id.into()),
            confirmation.into(),
            activated_at.into(),
        )
    }

    /// Returns the current status of one exact tuple. This operation is
    /// read-only and never requires a TTY.
    pub fn status(
        &self,
        config: &RepositoryConfig,
        tuple: &PolicyTuple,
    ) -> Result<PolicyStatus, PolicyError> {
        status_from_state(self.state(), config, tuple)
    }

    /// Compatibility alias for [`PolicyRegistry::status`].
    pub fn policy_status(
        &self,
        config: &RepositoryConfig,
        tuple: &PolicyTuple,
    ) -> Result<PolicyStatus, PolicyError> {
        self.status(config, tuple)
    }

    /// Evaluates one exact tuple against current configuration and state.
    pub fn evaluate(
        &self,
        config: &RepositoryConfig,
        tuple: &PolicyTuple,
    ) -> Result<PolicyDecision, PolicyError> {
        evaluate_from_state(self.state(), config, tuple)
    }

    /// Compatibility alias for [`PolicyRegistry::evaluate`].
    pub fn evaluate_policy(
        &self,
        config: &RepositoryConfig,
        tuple: &PolicyTuple,
    ) -> Result<PolicyDecision, PolicyError> {
        self.evaluate(config, tuple)
    }

    /// Lists all activation rows for a repository without changing state.
    pub fn activations(
        &self,
        repository_id: impl AsRef<str>,
    ) -> Result<Vec<PolicyActivationRecord>, PolicyError> {
        activation_records_from_state(self.state(), repository_id)
    }

    /// Deactivates one exact activation ID. Deactivation is permission
    /// reducing and is deliberately available without a TTY.
    pub fn deactivate(
        &mut self,
        repository_id: impl AsRef<str>,
        activation_id: impl AsRef<str>,
        deactivated_at: impl Into<String>,
    ) -> Result<PolicyActivationRecord, PolicyError> {
        let repository_id = repository_id.as_ref().to_owned();
        let activation_id = activation_id.as_ref().to_owned();
        let deactivated_at = deactivated_at.into();
        validate_activation_id(&activation_id)?;
        validate_timestamp(&deactivated_at)?;
        self.state_mut()
            .deactivate_policy(&repository_id, &activation_id, deactivated_at)
            .map_err(PolicyError::State)
    }

    /// Compatibility alias for [`PolicyRegistry::deactivate`].
    pub fn deactivate_policy(
        &mut self,
        repository_id: impl AsRef<str>,
        activation_id: impl AsRef<str>,
        deactivated_at: impl Into<String>,
    ) -> Result<PolicyActivationRecord, PolicyError> {
        self.deactivate(repository_id, activation_id, deactivated_at)
    }

    /// Deactivates all active rows for one exact tuple, including stale rows.
    /// This operation never widens permission and therefore needs no TTY.
    pub fn deactivate_for(
        &mut self,
        repository_id: impl AsRef<str>,
        tuple: &PolicyTuple,
        deactivated_at: impl Into<String>,
    ) -> Result<Vec<PolicyActivationRecord>, PolicyError> {
        let repository_id = repository_id.as_ref().to_owned();
        tuple
            .validate()
            .map_err(|_| PolicyError::InvalidPolicyTuple)?;
        let deactivated_at = deactivated_at.into();
        validate_timestamp(&deactivated_at)?;
        let records = self.records(&repository_id)?;
        let activation_ids = records
            .iter()
            .filter(|record| record.active)
            .filter(|record| {
                PolicyTuple::new(
                    record.event_type.clone(),
                    record.destination_alias.clone(),
                    record.severity.clone(),
                )
                .matches_exact(tuple)
            })
            .map(|record| record.activation_id.clone())
            .collect::<Vec<_>>();
        if activation_ids.is_empty() {
            return Err(PolicyError::NoActiveActivation);
        }
        activation_ids
            .into_iter()
            .map(|activation_id| {
                self.state_mut().deactivate_policy(
                    &repository_id,
                    &activation_id,
                    deactivated_at.clone(),
                )
            })
            .collect::<Result<Vec<_>, _>>()
            .map_err(PolicyError::State)
    }

    fn activate_with_optional_id(
        &mut self,
        config: &RepositoryConfig,
        tuple: &PolicyTuple,
        requested_id: Option<String>,
        confirmation: OperatorConfirmation,
        activated_at: String,
    ) -> Result<ActivationReceipt, PolicyError> {
        // Check the explicit interaction boundary before reading or mutating
        // policy state. In particular, a non-TTY caller cannot probe or widen
        // authority by supplying a malformed configuration.
        confirmation.require_tty()?;
        let preview = self.preview(config, tuple, requested_id.as_deref())?;
        validate_timestamp(&activated_at)?;
        let current = PolicyHashes {
            config_hash: preview.config_hash.clone(),
            tuple_hash: preview.tuple_hash.clone(),
        };
        let records = self.records(&config.repository_id)?;
        let current_active = records
            .iter()
            .filter(|record| record.active)
            .filter(|record| {
                record.active
                    && record.deactivated_at.is_none()
                    && record.config_hash == current.config_hash
                    && record.policy_tuple_hash == current.tuple_hash
                    && PolicyTuple::new(
                        record.event_type.clone(),
                        record.destination_alias.clone(),
                        record.severity.clone(),
                    )
                    .matches_exact(tuple)
            })
            .collect::<Vec<_>>();
        if current_active.len() > 1 {
            return Err(PolicyError::AmbiguousPolicy);
        }
        if let Some(record) = current_active.first() {
            return Ok(ActivationReceipt::from_record(record));
        }

        let activation_id = choose_activation_id(
            requested_id.as_deref(),
            &preview.activation_id,
            &activated_at,
            &records,
        )?;
        let input = PolicyActivationInput::new(
            config.repository_id.clone(),
            activation_id,
            current.config_hash,
            current.tuple_hash,
            tuple.event_type.clone(),
            tuple.destination_alias.clone(),
            tuple.severity.clone(),
            activated_at,
        );
        let record = self
            .state_mut()
            .activate_policy(&input)
            .map_err(PolicyError::State)?;
        Ok(ActivationReceipt::from_record(&record))
    }

    fn records(&self, repository_id: &str) -> Result<Vec<PolicyActivationRecord>, PolicyError> {
        let state = self.state();
        query_records(state.connection(), repository_id, state.path())
    }
}

impl PolicyRegistry<StateStore> {
    /// Creates an owned registry from an already opened user-level store.
    #[must_use]
    pub fn from_store(store: StateStore) -> Self {
        Self::new(store)
    }
}

fn query_records(
    connection: &Connection,
    repository_id: &str,
    path: Option<&std::path::Path>,
) -> Result<Vec<PolicyActivationRecord>, PolicyError> {
    let mut statement = connection
        .prepare(
            "SELECT repository_id, activation_id, config_hash, policy_tuple_hash,
                    event_type, destination_alias, severity, activated_at,
                    deactivated_at, active
             FROM policy_activations
             WHERE repository_id = ?1
             ORDER BY activation_id",
        )
        .map_err(|error| query_state_error(path, "prepare policy status", error))?;
    let rows = statement
        .query_map([repository_id], |row| {
            Ok(PolicyActivationRecord {
                repository_id: row.get(0)?,
                activation_id: row.get(1)?,
                config_hash: row.get(2)?,
                policy_tuple_hash: row.get(3)?,
                event_type: row.get(4)?,
                destination_alias: row.get(5)?,
                severity: row.get(6)?,
                activated_at: row.get(7)?,
                deactivated_at: row.get(8)?,
                active: row.get::<_, i64>(9)? == 1,
            })
        })
        .map_err(|error| query_state_error(path, "query policy status", error))?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row.map_err(|error| query_state_error(path, "read policy status", error))?);
    }
    Ok(records)
}

fn query_state_error(
    path: Option<&std::path::Path>,
    operation: &'static str,
    error: rusqlite::Error,
) -> PolicyError {
    PolicyError::State(StateError::Sqlite {
        path: path.map(PathBuf::from),
        operation,
        message: error.to_string(),
    })
}

fn choose_activation_id(
    requested_id: Option<&str>,
    default_id: &str,
    activated_at: &str,
    records: &[PolicyActivationRecord],
) -> Result<String, PolicyError> {
    if let Some(requested_id) = requested_id {
        validate_activation_id(requested_id)?;
        if records
            .iter()
            .any(|record| record.activation_id == requested_id)
        {
            return Err(PolicyError::ActivationIdConflict);
        }
        return Ok(requested_id.to_owned());
    }

    if !records
        .iter()
        .any(|record| record.activation_id == default_id)
    {
        return Ok(default_id.to_owned());
    }

    let suffix = sha256_hex(activated_at.as_bytes());
    let base = format!("{default_id}-{}", &suffix[..16]);
    if !records.iter().any(|record| record.activation_id == base) {
        return Ok(base);
    }
    let mut sequence = 2_u32;
    loop {
        let candidate = format!("{base}-{sequence}");
        if !records
            .iter()
            .any(|record| record.activation_id == candidate)
        {
            return Ok(candidate);
        }
        sequence = sequence
            .checked_add(1)
            .ok_or(PolicyError::ActivationIdConflict)?;
    }
}

fn default_activation_id(config_hash: &str, tuple_hash: &str) -> String {
    format!("policy-{config_hash}-{tuple_hash}")
}

fn validate_activation_id(value: &str) -> Result<(), PolicyError> {
    if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(PolicyError::InvalidActivationId);
    }
    Ok(())
}

fn validate_timestamp(value: &str) -> Result<(), PolicyError> {
    if value.is_empty() || value.len() > 128 || value.contains('\0') {
        return Err(PolicyError::InvalidTimestamp);
    }
    Ok(())
}

/// Converts a policy error into the foundation's stable operator-facing
/// category without exposing configuration contents.
#[must_use]
pub fn policy_error_to_repo_com_error(error: &PolicyError) -> RepoComError {
    error.to_repo_com_error()
}
