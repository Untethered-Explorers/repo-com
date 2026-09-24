#![forbid(unsafe_code)]
#![doc = "TTY-only, time-bounded approval bound to one exact repo-com draft revision."]

#[cfg(test)]
#[path = "../tests/approval_contract.rs"]
mod approval_contract;

mod r#override;
mod service;

pub use r#override::{OverrideReasonCode, SecretOverrideRecord};
pub use service::{
    APPROVAL_LIFETIME_SECONDS, ApprovalBindingField, ApprovalCheck, ApprovalClock,
    ApprovalDisposition, ApprovalError, ApprovalInstant, ApprovalInvalidReason, ApprovalPreview,
    ApprovalRecord, ApprovalRevalidation, ApprovalService, OperatorConfirmation,
    PreviewInvalidReason,
};
