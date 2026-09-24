use serde::{Deserialize, Serialize};

/// The exact current-state facts a delivery coordinator must repeat inside its
/// atomic claim immediately before network I/O.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RevalidationFacts {
    /// Repository identity that must still resolve locally.
    pub repository_id: String,
    /// Deterministic hash of repository and workspace identity.
    pub repository_hash: String,
    /// Workspace identity resolved from current configuration.
    pub workspace_id: String,
    /// Exact draft identity.
    pub draft_id: String,
    /// Immutable revision number.
    pub revision: u64,
    /// Canonical immutable revision hash.
    pub revision_hash: String,
    /// Configured alias that must be resolved again.
    pub destination_alias: String,
    /// Hash of the current and revision destination snapshots.
    pub destination_hash: String,
    /// Hash of the exact final rendered text.
    pub exact_text_hash: String,
    /// Hash of bounded draft metadata.
    pub metadata_hash: String,
    /// Hash of the complete normalized current configuration.
    pub config_hash: String,
    /// Hash of the exact current policy decision.
    pub policy_basis_hash: String,
    /// Hash of the redacted secret-scan result.
    pub scan_hash: String,
    /// Exact approval preview that was independently revalidated.
    pub approval_preview_hash: String,
    /// Original draft creation time.
    pub draft_created_at_unix_seconds: u64,
    /// Exclusive draft expiry boundary.
    pub draft_expires_at_unix_seconds: u64,
    /// Injected instant at which this pure decision was evaluated.
    pub evaluated_at_unix_seconds: u64,
}

/// One exact, already-existing authority that permits evaluation of one draft
/// revision. It is not a claim and does not itself perform a send.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum EligibilityAuthority {
    /// A current exact-revision human approval.
    HumanApproval {
        /// Stable approval identifier.
        approval_id: String,
        /// Hash of the exact approval record.
        approval_hash: String,
        /// Exclusive approval boundary.
        expires_at_unix_seconds: u64,
    },
    /// A current exact-tuple policy activation.
    ActivatedPolicy {
        /// Stable policy activation identifier.
        activation_id: String,
        /// Hash of the exact activation snapshot.
        activation_hash: String,
        /// Current complete configuration hash.
        config_hash: String,
        /// Current exact event/destination/severity tuple hash.
        tuple_hash: String,
        /// Operator activation timestamp retained in local state.
        activated_at: String,
    },
}

/// A stable reason that current state cannot grant send eligibility.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum EligibilityBlocker {
    /// Repository or workspace scope no longer matches.
    RepositoryScopeChanged,
    /// The immutable revision, exact text, or metadata changed.
    RevisionChanged,
    /// The complete normalized configuration hash changed.
    ConfigChanged,
    /// The destination alias no longer resolves to the revision snapshot.
    DestinationChanged,
    /// The draft expiry boundary has been reached.
    DraftExpired,
    /// The supplied scan no longer matches the exact rendered revision.
    SecretScanChanged,
    /// The supplied policy decision differs from the approval preview basis.
    PolicyBasisChanged,
    /// Approval evidence is missing, incoherent, or no longer exact.
    ApprovalStateChanged,
    /// The 15-minute or earlier draft approval boundary has been reached.
    ApprovalExpired,
    /// An activation exists but its config or exact tuple binding is stale.
    StalePolicyActivation,
    /// Multiple current activation records could broaden authority.
    PolicyAmbiguous,
    /// The matching policy was explicitly deactivated or is inactive.
    PolicyNotActive,
    /// A secret finding has no matching exact TTY override.
    UnresolvedSecretFinding,
    /// Automation has no pre-existing exact authority and cannot create any.
    OperatorActionRequired,
    /// No exact approval or activated policy exists.
    AuthorityMissing,
    /// Current inputs cannot form the canonical revalidation snapshot.
    CurrentStateInvalid,
}

impl EligibilityBlocker {
    /// Returns a stable machine-readable reason code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::RepositoryScopeChanged => "repository-scope-changed",
            Self::RevisionChanged => "revision-changed",
            Self::ConfigChanged => "config-changed",
            Self::DestinationChanged => "destination-chestination-changed",
            Self::DraftExpired => "draft-expired",
            Self::SecretScanChanged => "secret-scan-changed",
            Self::PolicyBasisChanged => "policy-basis-changed",
            Self::ApprovalStateChanged => "approval-state-changed",
            Self::ApprovalExpired => "approval-expired",
            Self::StalePolicyActivation => "stale-policy-activation",
            Self::PolicyAmbiguous => "policy-ambiguous",
            Self::PolicyNotActive => "policy-not-active",
            Self::UnresolvedSecretFinding => "unresolved-secret-finding",
            Self::OperatorActionRequired => "operator-action-required",
            Self::AuthorityMissing => "authority-missing",
            Self::CurrentStateInvalid => "current-state-invalid",
        }
    }
}

/// The only supported correction boundary for an immutable outbound message.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutboundCorrection {
    /// Create a new immutable draft revision.
    NewDraft,
    /// Create a separately validated threaded reply draft.
    ValidatedThreadedReply,
}

/// A typed, side-effect-free send-eligibility decision.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "kebab-case")]
pub enum EligibilityDecision {
    /// The exact revision may proceed to the delivery coordinator's separate
    /// atomic claim, where every fact must be revalidated again.
    Eligible {
        /// All facts required at the final delivery boundary.
        revalidation: RevalidationFacts,
        /// The exact pre-existing authority used for this decision.
        authority: EligibilityAuthority,
        /// Corrections must use this immutable boundary.
        correction: OutboundCorrection,
    },
    /// Current state fails closed.
    Blocked {
        /// The observed current facts, retained for safe diagnostics.
        revalidation: RevalidationFacts,
        /// Stable blocking reason.
        blocker: EligibilityBlocker,
    },
}

impl EligibilityDecision {
    /// Returns whether this exact revision is eligible for a separate claim.
    #[must_use]
    pub const fn is_eligible(&self) -> bool {
        matches!(self, Self::Eligible { .. })
    }

    /// Returns the exact authority for an eligible result.
    #[must_use]
    pub const fn authority(&self) -> Option<&EligibilityAuthority> {
        match self {
            Self::Eligible { authority, .. } => Some(authority),
            Self::Blocked { .. } => None,
        }
    }

    /// Returns the stable blocker for a denied result.
    #[must_use]
    pub const fn blocker(&self) -> Option<EligibilityBlocker> {
        match self {
            Self::Eligible { .. } => None,
            Self::Blocked { blocker, .. } => Some(*blocker),
        }
    }

    /// Returns all facts that must be repeated at the atomic delivery claim.
    #[must_use]
    pub const fn revalidation(&self) -> &RevalidationFacts {
        match self {
            Self::Eligible { revalidation, .. } | Self::Blocked { revalidation, .. } => {
                revalidation
            }
        }
    }

    /// Returns the only supported immutable correction paths.
    #[must_use]
    pub const fn correction(&self) -> Option<OutboundCorrection> {
        match self {
            Self::Eligible { correction, .. } => Some(*correction),
            Self::Blocked { .. } => None,
        }
    }

    /// Eligible output always requires a separate atomic delivery-claim
    /// revalidation immediately before network I/O.
    #[must_use]
    pub const fn requires_atomic_revalidation(&self) -> bool {
        self.is_eligible()
    }

    /// This crate never creates a durable send claim.
    #[must_use]
    pub const fn is_send_claim(&self) -> bool {
        false
    }
}
