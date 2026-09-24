use serde::{Deserialize, Serialize};

use crate::service::ApprovalError;

/// A closed set of redacted reasons for reviewing a high-confidence finding.
///
/// Free-form operator text is deliberately not accepted: audit evidence can
/// contain only this stable code and the exact non-secret hashes below.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OverrideReasonCode {
    /// The operator reviewed the exact preview and determined the finding to be
    /// a false positive.
    ReviewedFalsePositive,
}

impl OverrideReasonCode {
    /// Returns the stable machine-readable reason code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::ReviewedFalsePositive => "reviewed-false-positive",
        }
    }
}

impl std::fmt::Display for OverrideReasonCode {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}

/// Redacted, exact-revision evidence for one secret-scan override.
///
/// The matched value is neither accepted nor stored. Changing the revision,
/// scan, preview, configuration, destination, policy basis, or reason requires
/// a different override.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SecretOverrideRecord {
    /// Persistence schema version.
    pub schema_version: u8,
    /// Stable audit event identifier.
    pub event_id: String,
    /// Repository scope.
    pub repository_id: String,
    /// Exact draft identifier.
    pub draft_id: String,
    /// Exact immutable revision number.
    pub revision: u64,
    /// Canonical immutable draft revision hash.
    pub revision_hash: String,
    /// Hash of the exact redacted scan result.
    pub scan_hash: String,
    /// Hash of the complete preview reviewed by the operator.
    pub preview_hash: String,
    /// Closed reason code; no free-form or matched secret value.
    #[serde(rename = "reason")]
    pub reason_code: OverrideReasonCode,
    /// Injected approval time in Unix seconds.
    pub created_at_unix_seconds: u64,
    /// Canonical UTC audit timestamp.
    pub created_at_utc: String,
}

impl SecretOverrideRecord {
    /// Returns the deterministic record hash used by approval revalidation.
    pub fn hash(&self) -> Result<String, ApprovalError> {
        crate::service::sha256_json(self)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct OverrideAuditMetadata {
    pub(crate) schema_version: u8,
    pub(crate) override_record: PersistedOverride,
}

impl OverrideAuditMetadata {
    pub(crate) fn new(record: SecretOverrideRecord) -> Self {
        Self {
            schema_version: 1,
            override_record: PersistedOverride::from_record(record),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct PersistedOverride {
    pub(crate) schema_version: u8,
    pub(crate) event_id: String,
    pub(crate) repository_id: String,
    pub(crate) draft_id: String,
    pub(crate) revision: u64,
    pub(crate) revision_hash: String,
    pub(crate) scan_hash: String,
    pub(crate) preview_hash: String,
    #[serde(rename = "reason")]
    pub(crate) reason_code: OverrideReasonCode,
    pub(crate) created_at_unix_seconds: u64,
}

impl PersistedOverride {
    fn from_record(record: SecretOverrideRecord) -> Self {
        Self {
            schema_version: record.schema_version,
            event_id: record.event_id,
            repository_id: record.repository_id,
            draft_id: record.draft_id,
            revision: record.revision,
            revision_hash: record.revision_hash,
            scan_hash: record.scan_hash,
            preview_hash: record.preview_hash,
            reason_code: record.reason_code,
            created_at_unix_seconds: record.created_at_unix_seconds,
        }
    }

    pub(crate) fn into_record(self, created_at_utc: String) -> SecretOverrideRecord {
        SecretOverrideRecord {
            schema_version: self.schema_version,
            event_id: self.event_id,
            repository_id: self.repository_id,
            draft_id: self.draft_id,
            revision: self.revision,
            revision_hash: self.revision_hash,
            scan_hash: self.scan_hash,
            preview_hash: self.preview_hash,
            reason_code: self.reason_code,
            created_at_unix_seconds: self.created_at_unix_seconds,
            created_at_utc,
        }
    }
}
