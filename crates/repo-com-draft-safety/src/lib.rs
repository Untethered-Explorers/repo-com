#![forbid(unsafe_code)]
#![doc = "Pure, deterministic, redacted pre-send credential detection for repo-com drafts. This is defense in depth, not complete data-loss prevention."]

#[cfg(test)]
#[path = "../tests/draft_safety_contract.rs"]
mod draft_safety_contract;

pub mod patterns;
pub mod scanner;

pub use patterns::{
    Finding, FindingLocation, MatchSource, MetadataField, ReasonCode, SecretFinding,
    SecretLocation, SecretReasonCode,
};
pub use repo_com_draft_content::RenderedMessage;
pub use repo_com_draft_model::DraftMetadata;

pub use scanner::{
    ScanInput, ScanResult, SecretScanDecision, SecretScanInput, SecretScanOutcome,
    SecretScanRequest, SecretScanResult, SecretScanStatus, SecretScanner, scan, scan_text,
};
