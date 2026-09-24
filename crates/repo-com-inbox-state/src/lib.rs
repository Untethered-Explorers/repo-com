#![forbid(unsafe_code)]
#![doc = "Repository-scoped, transactional local state for untrusted inbound Discord data."]

mod cursor;
mod item;
mod store;

#[cfg(test)]
#[path = "../tests/inbox_state_contract.rs"]
mod inbox_state_contract;

pub use cursor::{
    AliasCursor, CursorInput, CursorRecord, compare_cursor_values, cursor_moves_forward,
    cursor_values_equal,
};
pub use item::{
    CurrentSnapshot, FetchedItem, FirstSnapshot, InboundItem, InboundPageItem, InboundTransition,
    InboundTransitionUpdate, PageItem, TransitionUpdate,
};
pub use store::{
    CommitPageOptions, FailurePoint, InboxPage, InboxResult, InboxState, InboxStateError,
    PageCommitResult,
};

/// Compatibility name matching the feature interface spelling.
pub type InboundState = InboxState;
/// Compatibility name matching the feature interface spelling.
pub type InboundPage = InboxPage;
/// Compatibility name matching the feature interface spelling.
pub type InboundStateError = InboxStateError;

/// Shared state types are re-exported so callers can prepare a repository and
/// a locally-created reply draft without bypassing the inbound boundary.
pub use repo_com_state::{
    AcknowledgementRecord, ArchiveRecord, AuditEventInput, DraftInput, InboundCurrentSnapshotInput,
    InboundCurrentSnapshotRecord, InboundCursorInput, InboundCursorRecord, InboundItemInput,
    InboundItemRecord, InboundTransitionInput, InboundTransitionRecord, ReplyLinkInput,
    ReplyLinkRecord, RepositoryInput, RepositoryRecord, StateError, StateResult, StateStore,
};

#[doc(hidden)]
pub use repo_com_state::{StateStoreOptions, StateTransaction};
