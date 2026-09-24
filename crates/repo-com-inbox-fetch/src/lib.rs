#![forbid(unsafe_code)]
#![doc = "Bounded, read-only, untrusted Discord inbound retrieval and point reconciliation."]

mod boundary;
mod fetch;
mod filter;
mod reconcile;

#[cfg(test)]
#[path = "../tests/inbox_fetch_contract.rs"]
mod inbox_fetch_contract;

pub use boundary::{
    Boundary, BoundaryError, FetchBoundary, InboundBoundary, MAX_FETCH_PAGES, MAX_POINT_CHECKS,
    MAX_RAW_MESSAGES, MESSAGES_PER_PAGE, rfc3339_to_discord_snowflake, rfc3339_to_unix_millis,
};
pub use fetch::{
    ContinuationReason, DiscordInboundClient, FetchContinuation, FetchError, FetchRateLimitInfo,
    FetchRequest, FetchResult, InboundFetcher, MAX_DISCORD_DIRECTED_WAIT, RateLimitInfo, ReadError,
    ReadRateLimitInfo, ReadRateLimitScope, StoredFetchResult,
};
pub use filter::{
    AcceptedDelivery, AcceptedDeliveryError, AcceptedLocalDelivery, AttachmentIndicator,
    AuthorEvidence, FetchProvenance, FilterContext, FilterError, InboundEnvelope, InboundTrust,
    MentionEvidence, RemoteMessage, ReplyEvidence, UntrustedEnvelope, UntrustedInboundEnvelope,
    filter_messages, should_retain, to_untrusted_envelope,
};
pub use reconcile::{
    PointCheckResult, PointCheckState, PointReadResult, ProbeContinuation, ReconcileSummary,
    ReconciliationError, ReconciliationSummary, reconcile_page,
};
