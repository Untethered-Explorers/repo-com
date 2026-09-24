#![forbid(unsafe_code)]
#![doc = "Pure, fail-closed current-state send eligibility for one exact repo-com draft revision."]

#[cfg(test)]
#[path = "../tests/send_eligibility_contract.rs"]
mod send_eligibility_contract;

mod decision;
mod evaluator;

pub use decision::{
    EligibilityAuthority, EligibilityBlocker, EligibilityDecision, OutboundCorrection,
    RevalidationFacts,
};
pub use evaluator::{EligibilityEvaluator, EligibilityInput, evaluate};
