use std::collections::BTreeSet;

use repo_com_config::{RepositoryConfig, is_secret_like_field};
use repo_com_state::PolicyActivationRecord;
use serde::{Deserialize, Serialize};

use crate::PolicyError;
use crate::hash::{
    PolicyHashes, PolicyTuple, canonical_config_hash, canonical_tuple_hash, is_sha256_hex,
};

/// The exact configured policy selected for a request.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PolicyMatch {
    /// The exact policy tuple.
    pub tuple: PolicyTuple,
    /// Hash of the complete normalized configuration.
    pub config_hash: String,
    /// Hash of the exact tuple.
    pub tuple_hash: String,
}

impl PolicyMatch {
    /// Creates a match result with freshly computed canonical hashes.
    #[must_use]
    pub fn new(config: &RepositoryConfig, tuple: &PolicyTuple) -> Self {
        Self {
            tuple: tuple.clone(),
            config_hash: canonical_config_hash(config),
            tuple_hash: canonical_tuple_hash(tuple),
        }
    }

    /// Returns whether the match is an exact tuple match.
    #[must_use]
    pub fn is_exact(&self, requested: &PolicyTuple) -> bool {
        &self.tuple == requested
    }

    /// Returns the hashes that downstream evaluation must revalidate.
    #[must_use]
    pub fn hashes(&self) -> PolicyHashes {
        PolicyHashes {
            config_hash: self.config_hash.clone(),
            tuple_hash: self.tuple_hash.clone(),
        }
    }
}

/// Why a previously active binding no longer grants policy eligibility.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum StaleReason {
    /// The complete normalized configuration hash changed.
    ConfigHashChanged,
    /// The exact policy tuple hash changed.
    TupleHashChanged,
    /// Both hashes changed.
    ConfigAndTupleHashChanged,
    /// The stored binding is malformed and cannot be trusted.
    MalformedBinding,
}

impl StaleReason {
    /// Returns whether the complete configuration binding changed.
    #[must_use]
    pub const fn config_changed(self) -> bool {
        matches!(
            self,
            Self::ConfigHashChanged | Self::ConfigAndTupleHashChanged
        )
    }

    /// Returns whether the exact tuple binding changed.
    #[must_use]
    pub const fn tuple_changed(self) -> bool {
        matches!(
            self,
            Self::TupleHashChanged | Self::ConfigAndTupleHashChanged
        )
    }
}

/// A typed snapshot of one stored activation and its current hash comparison.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ActivationSnapshot {
    /// Stable activation identifier.
    pub activation_id: String,
    /// Repository scope.
    pub repository_id: String,
    /// Exact tuple stored by the activation.
    pub tuple: PolicyTuple,
    /// Configuration hash stored by the activation.
    pub recorded_config_hash: String,
    /// Tuple hash stored by the activation.
    pub recorded_tuple_hash: String,
    /// Current complete configuration hash.
    pub current_config_hash: String,
    /// Current exact tuple hash.
    pub current_tuple_hash: String,
    /// Activation timestamp retained in user state.
    pub activated_at: String,
    /// Deactivation timestamp, when present.
    pub deactivated_at: Option<String>,
    /// Whether the stored row is still marked active.
    pub active: bool,
    /// Stale reason, when the row is not currently eligible.
    pub stale_reason: Option<StaleReason>,
}

impl ActivationSnapshot {
    /// Returns the current hash binding.
    #[must_use]
    pub fn current_hashes(&self) -> PolicyHashes {
        PolicyHashes {
            config_hash: self.current_config_hash.clone(),
            tuple_hash: self.current_tuple_hash.clone(),
        }
    }

    /// Returns whether this snapshot is active and current.
    #[must_use]
    pub fn is_current(&self) -> bool {
        self.active && self.stale_reason.is_none()
    }
}

/// Read-only status of an exact policy binding.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PolicyStatus {
    /// The requested tuple is not present in the current configuration and no
    /// prior activation was found.
    NotConfigured,
    /// The tuple is configured but no activation exists.
    NotActivated,
    /// Exactly one current activation is active.
    Active(ActivationSnapshot),
    /// An activation exists, but its current hash binding is stale.
    Stale(ActivationSnapshot),
    /// The latest matching row was explicitly deactivated.
    Deactivated(ActivationSnapshot),
    /// More than one matching active binding could grant authority.
    Ambiguous {
        /// All matching snapshots, in deterministic activation-ID order.
        activations: Vec<ActivationSnapshot>,
    },
}

impl PolicyStatus {
    /// Returns whether status is currently active.
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active(_))
    }

    /// Returns whether status is stale.
    #[must_use]
    pub fn is_stale(&self) -> bool {
        matches!(self, Self::Stale(_))
    }

    /// Returns the sole snapshot for non-ambiguous statuses.
    #[must_use]
    pub fn snapshot(&self) -> Option<&ActivationSnapshot> {
        match self {
            Self::Active(snapshot) | Self::Stale(snapshot) | Self::Deactivated(snapshot) => {
                Some(snapshot)
            }
            Self::Ambiguous { activations } => activations.first(),
            Self::NotConfigured | Self::NotActivated => None,
        }
    }
}

/// Result of evaluating a draft tuple against the policy registry.
///
/// `Eligible` means only that the exact policy gate is currently satisfied. It
/// is not final send eligibility: approval, safety, destination, revision, and
/// delivery gates must still revalidate current state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PolicyDecision {
    /// The exact configured tuple has one current active activation.
    Eligible(ActivationSnapshot),
    /// The exact configured tuple has a stale activation.
    Stale(ActivationSnapshot),
    /// The tuple is configured but no activation exists.
    NotActivated,
    /// The requested tuple has no exact configured policy.
    NoExactMatch,
    /// The requested tuple is not present in the current configuration and no
    /// prior activation was found.
    NotConfigured,
    /// The matching activation was explicitly deactivated.
    Deactivated(ActivationSnapshot),
    /// Multiple matching active records exist, so authority is denied.
    Ambiguous {
        /// All matching snapshots.
        activations: Vec<ActivationSnapshot>,
    },
}

impl PolicyDecision {
    /// Returns whether the policy gate is currently eligible.
    #[must_use]
    pub fn is_eligible(&self) -> bool {
        matches!(self, Self::Eligible(_))
    }

    /// Returns whether the policy binding is stale.
    #[must_use]
    pub fn is_stale(&self) -> bool {
        matches!(self, Self::Stale(_))
    }

    /// Returns the activation snapshot for a non-ambiguous decision.
    #[must_use]
    pub fn snapshot(&self) -> Option<&ActivationSnapshot> {
        match self {
            Self::Eligible(snapshot) | Self::Stale(snapshot) | Self::Deactivated(snapshot) => {
                Some(snapshot)
            }
            Self::Ambiguous { activations } => activations.first(),
            Self::NotActivated | Self::NoExactMatch | Self::NotConfigured => None,
        }
    }
}

/// Compatibility name for callers that use the word evaluation.
pub type PolicyEvaluation = PolicyDecision;

/// Validates the policy-relevant portion of a configuration.
///
/// The configuration crate remains the authority for the full schema. This
/// check additionally ensures that a manually constructed model cannot turn a
/// wildcard or duplicate tuple into policy authority.
pub fn validate_policy_configuration(config: &RepositoryConfig) -> Result<(), PolicyError> {
    if config.schema_version != repo_com_config::SCHEMA_VERSION || config.repository_id.is_empty() {
        return Err(PolicyError::InvalidConfiguration);
    }

    let mut seen = BTreeSet::new();
    for entry in &config.auto_send {
        let tuple = PolicyTuple::from_entry(entry);
        tuple
            .validate()
            .map_err(|_| PolicyError::InvalidPolicyTuple)?;
        if !config.destinations.contains_key(&tuple.destination_alias)
            || !is_valid_destination_alias(&tuple.destination_alias)
            || is_secret_like_field(&tuple.destination_alias)
        {
            return Err(PolicyError::InvalidConfiguration);
        }
        if !seen.insert(tuple) {
            return Err(PolicyError::AmbiguousPolicy);
        }
    }
    Ok(())
}

/// Returns all valid policy tuples in deterministic order.
pub fn configured_policy_tuples(
    config: &RepositoryConfig,
) -> Result<Vec<PolicyTuple>, PolicyError> {
    validate_policy_configuration(config)?;
    let mut tuples = config
        .auto_send
        .iter()
        .map(PolicyTuple::from_entry)
        .collect::<Vec<_>>();
    tuples.sort();
    Ok(tuples)
}

/// Finds the one configured tuple exactly equal to `requested`.
///
/// This function performs equality only. It does not compare prefixes, rank
/// severities, or interpret wildcard characters.
pub fn find_exact_policy(
    config: &RepositoryConfig,
    requested: &PolicyTuple,
) -> Result<Option<PolicyTuple>, PolicyError> {
    validate_policy_configuration(config)?;
    requested
        .validate()
        .map_err(|_| PolicyError::InvalidPolicyTuple)?;
    let mut matches = Vec::new();
    for entry in &config.auto_send {
        let tuple = PolicyTuple::from_entry(entry);
        tuple
            .validate()
            .map_err(|_| PolicyError::InvalidPolicyTuple)?;
        if tuple.matches_exact(requested) {
            matches.push(tuple);
        }
    }
    if matches.len() > 1 {
        return Err(PolicyError::AmbiguousPolicy);
    }
    Ok(matches.into_iter().next())
}

/// Returns the exact match, if one exists, with its canonical hashes.
pub fn exact_policy_match(
    config: &RepositoryConfig,
    requested: &PolicyTuple,
) -> Result<Option<PolicyMatch>, PolicyError> {
    find_exact_policy(config, requested)?
        .map(|tuple| Ok(PolicyMatch::new(config, &tuple)))
        .transpose()
}

/// Compatibility alias for [`exact_policy_match`].
pub fn match_exact_policy(
    config: &RepositoryConfig,
    requested: &PolicyTuple,
) -> Result<Option<PolicyMatch>, PolicyError> {
    exact_policy_match(config, requested)
}

/// Classifies stored activation rows against current hashes without mutating
/// state.
pub fn classify_activation_records(
    records: &[PolicyActivationRecord],
    repository_id: &str,
    requested: &PolicyTuple,
    current: &PolicyHashes,
    configured: bool,
) -> PolicyStatus {
    let matching = records
        .iter()
        .filter(|record| record.repository_id == repository_id)
        .filter(|record| {
            PolicyTuple::new(
                record.event_type.clone(),
                record.destination_alias.clone(),
                record.severity.clone(),
            )
            .matches_exact(requested)
        })
        .collect::<Vec<_>>();

    if matching.is_empty() {
        return if configured {
            PolicyStatus::NotActivated
        } else {
            PolicyStatus::NotConfigured
        };
    }

    let snapshots = matching
        .iter()
        .map(|record| snapshot_for(record, current))
        .collect::<Vec<_>>();

    let current_active = matching
        .iter()
        .zip(snapshots.iter())
        .filter(|(record, snapshot)| record.active && snapshot.is_current())
        .collect::<Vec<_>>();

    if current_active.len() == 1 {
        return PolicyStatus::Active(current_active[0].1.clone());
    }
    if current_active.len() > 1 {
        return PolicyStatus::Ambiguous {
            activations: snapshots,
        };
    }

    let active = matching
        .iter()
        .zip(snapshots.iter())
        .filter(|(record, _)| record.active)
        .collect::<Vec<_>>();
    if active.len() == 1 {
        return PolicyStatus::Stale(active[0].1.clone());
    }
    if active.len() > 1 {
        return PolicyStatus::Ambiguous {
            activations: snapshots,
        };
    }

    if snapshots.len() == 1 {
        return PolicyStatus::Deactivated(snapshots[0].clone());
    }
    PolicyStatus::Ambiguous {
        activations: snapshots,
    }
}

/// Converts a read-only status into a policy evaluation result.
#[must_use]
pub fn decision_from_status(status: PolicyStatus) -> PolicyDecision {
    match status {
        PolicyStatus::NotConfigured => PolicyDecision::NotConfigured,
        PolicyStatus::NotActivated => PolicyDecision::NotActivated,
        PolicyStatus::Active(snapshot) => PolicyDecision::Eligible(snapshot),
        PolicyStatus::Stale(snapshot) => PolicyDecision::Stale(snapshot),
        PolicyStatus::Deactivated(snapshot) => PolicyDecision::Deactivated(snapshot),
        PolicyStatus::Ambiguous { activations } => PolicyDecision::Ambiguous { activations },
    }
}

fn is_valid_destination_alias(alias: &str) -> bool {
    !alias.is_empty()
        && alias.len() <= 64
        && alias
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && alias
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_.".contains(character))
}

fn snapshot_for(record: &PolicyActivationRecord, current: &PolicyHashes) -> ActivationSnapshot {
    let tuple = PolicyTuple::new(
        record.event_type.clone(),
        record.destination_alias.clone(),
        record.severity.clone(),
    );
    let lifecycle_is_coherent = if record.active {
        record.deactivated_at.is_none()
    } else {
        record.deactivated_at.is_some()
    };
    let well_formed = current.are_well_formed()
        && lifecycle_is_coherent
        && is_sha256_hex(&record.config_hash)
        && is_sha256_hex(&record.policy_tuple_hash);
    let config_changed = record.config_hash != current.config_hash;
    let tuple_changed = record.policy_tuple_hash != current.tuple_hash;
    let stale_reason = if !well_formed {
        Some(StaleReason::MalformedBinding)
    } else if config_changed && tuple_changed {
        Some(StaleReason::ConfigAndTupleHashChanged)
    } else if config_changed {
        Some(StaleReason::ConfigHashChanged)
    } else if tuple_changed {
        Some(StaleReason::TupleHashChanged)
    } else {
        None
    };
    ActivationSnapshot {
        activation_id: record.activation_id.clone(),
        repository_id: record.repository_id.clone(),
        tuple,
        recorded_config_hash: record.config_hash.clone(),
        recorded_tuple_hash: record.policy_tuple_hash.clone(),
        current_config_hash: current.config_hash.clone(),
        current_tuple_hash: current.tuple_hash.clone(),
        activated_at: record.activated_at.clone(),
        deactivated_at: record.deactivated_at.clone(),
        active: record.active,
        stale_reason,
    }
}
