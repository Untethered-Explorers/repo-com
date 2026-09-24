//! Inbound item and transition inputs used by the local state boundary.
//!
//! The remote fetch layer supplies opaque identifiers and content.  This
//! module deliberately does not interpret that content or provide a remote
//! operation; it only groups the data that may be committed locally.

use repo_com_state::{InboundCurrentSnapshotInput, InboundItemInput, InboundTransitionInput};

/// A first remote observation for an inbound item.
pub type FirstSnapshot = InboundItemInput;
/// An inbound item's first remote observation.
pub type InboundItem = InboundItemInput;
/// The current remote snapshot, including a possible deletion marker.
pub type CurrentSnapshot = InboundCurrentSnapshotInput;
/// One local inbound edit/deletion/lifecycle transition.
pub type InboundTransition = InboundTransitionInput;

/// A transition and the optional current snapshot that must accompany it.
///
/// When `current` is `None`, the state repository derives the current value
/// for the ordinary `edited` and `deleted` transition kinds.  Supplying a
/// snapshot is useful when an attachment indicator changed without changing
/// the message text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransitionUpdate {
    /// The transition evidence to append.
    pub transition: InboundTransitionInput,
    /// The current snapshot to write in the same local transaction.
    pub current: Option<InboundCurrentSnapshotInput>,
}

impl TransitionUpdate {
    /// Creates a transition update without an explicit current snapshot.
    #[must_use]
    pub fn new(transition: InboundTransitionInput) -> Self {
        Self {
            transition,
            current: None,
        }
    }

    /// Creates a transition update with an explicit current snapshot.
    #[must_use]
    pub fn with_current(
        transition: InboundTransitionInput,
        current: InboundCurrentSnapshotInput,
    ) -> Self {
        Self {
            transition,
            current: Some(current),
        }
    }

    /// Creates an update from the common state transition input.
    #[must_use]
    pub fn from_input(transition: InboundTransitionInput) -> Self {
        Self::new(transition)
    }

    /// Returns the transition input.
    #[must_use]
    pub const fn input(&self) -> &InboundTransitionInput {
        &self.transition
    }

    /// Returns the optional current snapshot input.
    #[must_use]
    pub const fn current_snapshot(&self) -> Option<&InboundCurrentSnapshotInput> {
        self.current.as_ref()
    }
}

impl From<InboundTransitionInput> for TransitionUpdate {
    fn from(transition: InboundTransitionInput) -> Self {
        Self::new(transition)
    }
}

/// One accepted item in a fetched page.
///
/// `first` is written once.  `current` is the latest state known at the time
/// the page is committed, and `transitions` records any edit/deletion events
/// observed for the item.  Later acknowledgement, archive, and reply-link
/// operations are deliberately separate from this fetch page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageItem {
    /// Immutable first remote snapshot.
    pub first: InboundItemInput,
    /// Current remote snapshot at page commit time.
    pub current: InboundCurrentSnapshotInput,
    /// Reconciliation transitions for this item.
    pub transitions: Vec<TransitionUpdate>,
}

impl PageItem {
    /// Creates a page item with no additional reconciliation events.
    #[must_use]
    pub fn new(first: InboundItemInput, current: InboundCurrentSnapshotInput) -> Self {
        Self {
            first,
            current,
            transitions: Vec::new(),
        }
    }

    /// Creates a page item with reconciliation events.
    #[must_use]
    pub fn with_transitions(
        first: InboundItemInput,
        current: InboundCurrentSnapshotInput,
        transitions: Vec<TransitionUpdate>,
    ) -> Self {
        Self {
            first,
            current,
            transitions,
        }
    }

    /// Adds one reconciliation event to this item.
    pub fn add_transition(&mut self, transition: impl Into<TransitionUpdate>) -> &mut Self {
        self.transitions.push(transition.into());
        self
    }

    /// Adds one reconciliation event with an explicit current snapshot.
    pub fn add_transition_with_current(
        &mut self,
        transition: InboundTransitionInput,
        current: InboundCurrentSnapshotInput,
    ) -> &mut Self {
        self.transitions
            .push(TransitionUpdate::with_current(transition, current));
        self
    }
}

/// Compatibility name for [`PageItem`].
pub type FetchedItem = PageItem;
/// Compatibility name for [`PageItem`].
pub type InboundPageItem = PageItem;
/// Compatibility name for [`TransitionUpdate`].
pub type InboundTransitionUpdate = TransitionUpdate;
