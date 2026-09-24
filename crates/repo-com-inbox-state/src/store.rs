use std::error::Error;
use std::fmt;
use std::path::Path;

use repo_com_audit::{AuditError, AuditEvent};
use repo_com_state::{
    AcknowledgementRecord, ArchiveRecord, AuditEventInput, InboundCurrentSnapshotInput,
    InboundCurrentSnapshotRecord, InboundCursorInput, InboundCursorRecord, InboundItemInput,
    InboundItemRecord, InboundTransitionInput, InboundTransitionRecord, ReplyLinkInput,
    ReplyLinkRecord, RepositoryInput, RepositoryRecord, StateError, StateStore, StateTransaction,
};

use crate::item::{PageItem, TransitionUpdate};

/// The result type used by the inbound state boundary.
pub type InboxResult<T> = Result<T, InboxStateError>;

/// A deterministic point at which a page commit can be failed for contract
/// testing.  The production API never schedules these points; callers opt in
/// explicitly through [`CommitPageOptions`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailurePoint {
    /// Fail immediately before the zero-based item is persisted.
    BeforeItem(usize),
    /// Fail after the requested number of items has been persisted.  Zero
    /// therefore fails before the first item.
    AfterItem(usize),
    /// Fail after all page evidence is present but before the cursor write.
    BeforeCursor,
    /// Fail after the cursor write but before the transaction commits.
    AfterCursor,
}

impl fmt::Display for FailurePoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BeforeItem(index) => write!(formatter, "before item {index}"),
            Self::AfterItem(count) => write!(formatter, "after {count} item(s)"),
            Self::BeforeCursor => formatter.write_str("before cursor"),
            Self::AfterCursor => formatter.write_str("after cursor"),
        }
    }
}

/// A typed failure from the local inbound state boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InboxStateError {
    /// The shared local state substrate rejected an operation.
    State(StateError),
    /// The redacted audit boundary rejected an event.
    Audit(AuditError),
    /// A page was internally inconsistent before any write was attempted.
    InvalidPage {
        /// The invalid page field, without its value.
        field: &'static str,
    },
    /// A transition identifier was reused with different evidence.
    TransitionConflict {
        /// The conflicting transition identifier.
        transition_id: String,
    },
    /// An explicitly requested test failure occurred inside the transaction.
    Injected {
        /// The exact transaction phase at which failure was injected.
        phase: FailurePoint,
    },
}

impl InboxStateError {
    /// Returns a stable, non-sensitive error category.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::InvalidPage { .. } => "inbox-state-usage",
            Self::TransitionConflict { .. } => "inbox-state-conflict",
            Self::Injected { .. } => "storage-integrity",
            Self::State(_) | Self::Audit(_) => "storage-integrity",
        }
    }

    /// Returns whether retrying the same local operation is appropriate.
    ///
    /// A busy lock, a transaction failure, and an explicitly injected test
    /// failure are retryable.  Validation, missing repository state, and
    /// evidence conflicts are not silently retried.
    #[must_use]
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Injected { .. } => true,
            Self::State(StateError::LockTimeout { .. } | StateError::Transaction { .. }) => true,
            Self::State(StateError::Sqlite { message, .. }) => {
                let lower = message.to_ascii_lowercase();
                lower.contains("busy") || lower.contains("locked")
            }
            Self::Audit(AuditError::State { .. }) => true,
            Self::State(_)
            | Self::Audit(_)
            | Self::InvalidPage { .. }
            | Self::TransitionConflict { .. } => false,
        }
    }

    /// Returns the underlying state error, when one is available.
    #[must_use]
    pub const fn state_error(&self) -> Option<&StateError> {
        match self {
            Self::State(error) => Some(error),
            _ => None,
        }
    }
}

impl fmt::Display for InboxStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::State(error) => write!(formatter, "inbound state operation failed: {error}"),
            Self::Audit(error) => write!(formatter, "inbound audit operation failed: {error}"),
            Self::InvalidPage { field } => {
                write!(formatter, "inbound page field is invalid: {field}")
            }
            Self::TransitionConflict { transition_id } => {
                write!(formatter, "inbound transition conflict: {transition_id}")
            }
            Self::Injected { phase } => write!(formatter, "injected inbound state failure {phase}"),
        }
    }
}

impl Error for InboxStateError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::State(error) => Some(error),
            Self::Audit(error) => Some(error),
            _ => None,
        }
    }
}

impl From<StateError> for InboxStateError {
    fn from(error: StateError) -> Self {
        Self::State(error)
    }
}

impl From<AuditError> for InboxStateError {
    fn from(error: AuditError) -> Self {
        Self::Audit(error)
    }
}

impl From<InboxStateError> for StateError {
    fn from(error: InboxStateError) -> Self {
        match error {
            InboxStateError::State(error) => error,
            other => StateError::Transaction {
                message: other.code().to_owned(),
            },
        }
    }
}

/// Options for one complete inbound page commit.
///
/// A page commit is atomic by default.  `write_audit` adds redacted local
/// audit evidence in that same transaction; disabling it is useful only for a
/// caller that owns a separate audit policy.  The failure fields are explicit
/// test seams and have no effect unless a caller sets them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommitPageOptions {
    /// Optional deterministic transaction failure point.
    pub failure_point: Option<FailurePoint>,
    /// Whether local audit events are appended in the same transaction.
    pub write_audit: bool,
}

impl Default for CommitPageOptions {
    fn default() -> Self {
        Self {
            failure_point: None,
            write_audit: true,
        }
    }
}

impl CommitPageOptions {
    /// Creates default options with audit evidence enabled.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            failure_point: None,
            write_audit: true,
        }
    }

    /// Disables automatic audit events while retaining atomic page storage.
    #[must_use]
    pub const fn without_audit() -> Self {
        Self {
            failure_point: None,
            write_audit: false,
        }
    }

    /// Enables or disables automatic audit events.
    #[must_use]
    pub const fn with_audit(mut self, enabled: bool) -> Self {
        self.write_audit = enabled;
        self
    }

    /// Selects a deterministic failure point.
    #[must_use]
    pub const fn with_failure_point(mut self, point: FailurePoint) -> Self {
        self.failure_point = Some(point);
        self
    }

    /// Fails after `count` page items have been persisted.
    #[must_use]
    pub const fn fail_after_item(count: usize) -> Self {
        Self {
            failure_point: Some(FailurePoint::AfterItem(count)),
            write_audit: true,
        }
    }

    /// Fails immediately before the zero-based item is persisted.
    #[must_use]
    pub const fn fail_before_item(index: usize) -> Self {
        Self {
            failure_point: Some(FailurePoint::BeforeItem(index)),
            write_audit: true,
        }
    }

    /// Fails after all page evidence is written but before cursor advancement.
    #[must_use]
    pub const fn fail_before_cursor() -> Self {
        Self {
            failure_point: Some(FailurePoint::BeforeCursor),
            write_audit: true,
        }
    }

    /// Fails after cursor advancement but before commit.
    #[must_use]
    pub const fn fail_after_cursor() -> Self {
        Self {
            failure_point: Some(FailurePoint::AfterCursor),
            write_audit: true,
        }
    }
}

/// A complete fetched page to persist locally.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InboxPage {
    /// Repository scope shared by every row in the page.
    pub repository_id: String,
    /// Repository-local inbound alias whose cursor is being advanced.
    pub alias: String,
    /// Next cursor value to make authoritative after all page evidence commits.
    pub cursor: String,
    /// Timestamp attached to the cursor update.
    pub updated_at: String,
    /// Accepted inbound items in deterministic fetch order.
    pub items: Vec<PageItem>,
    /// Reconciliation transitions for previously stored items or page items.
    pub transitions: Vec<TransitionUpdate>,
}

impl InboxPage {
    /// Creates an empty page for a repository alias and next cursor.
    #[must_use]
    pub fn new(
        repository_id: impl Into<String>,
        alias: impl Into<String>,
        cursor: impl Into<String>,
        updated_at: impl Into<String>,
    ) -> Self {
        Self {
            repository_id: repository_id.into(),
            alias: alias.into(),
            cursor: cursor.into(),
            updated_at: updated_at.into(),
            items: Vec::new(),
            transitions: Vec::new(),
        }
    }

    /// Compatibility alias for [`InboxPage::new`].
    #[must_use]
    pub fn empty(
        repository_id: impl Into<String>,
        alias: impl Into<String>,
        cursor: impl Into<String>,
        updated_at: impl Into<String>,
    ) -> Self {
        Self::new(repository_id, alias, cursor, updated_at)
    }

    /// Creates a page from complete owned parts.
    #[must_use]
    pub fn from_parts(
        repository_id: impl Into<String>,
        alias: impl Into<String>,
        cursor: impl Into<String>,
        updated_at: impl Into<String>,
        items: Vec<PageItem>,
        transitions: Vec<TransitionUpdate>,
    ) -> Self {
        Self {
            repository_id: repository_id.into(),
            alias: alias.into(),
            cursor: cursor.into(),
            updated_at: updated_at.into(),
            items,
            transitions,
        }
    }

    /// Adds one accepted item to the page.
    pub fn add_item(&mut self, item: PageItem) -> &mut Self {
        self.items.push(item);
        self
    }

    /// Adds one accepted item and returns the page for fluent construction.
    #[must_use]
    pub fn with_item(mut self, item: PageItem) -> Self {
        self.items.push(item);
        self
    }

    /// Adds one reconciliation transition to the page.
    pub fn add_transition(&mut self, transition: impl Into<TransitionUpdate>) -> &mut Self {
        self.transitions.push(transition.into());
        self
    }

    /// Adds one reconciliation transition and returns the page.
    #[must_use]
    pub fn with_transition(mut self, transition: impl Into<TransitionUpdate>) -> Self {
        self.transitions.push(transition.into());
        self
    }

    /// Returns the cursor update represented by this page.
    #[must_use]
    pub fn cursor_input(&self) -> InboundCursorInput {
        InboundCursorInput::new(
            self.repository_id.clone(),
            self.alias.clone(),
            self.cursor.clone(),
            self.updated_at.clone(),
        )
    }

    /// Checks all page-level scope relationships before opening a transaction.
    pub fn validate(&self) -> InboxResult<()> {
        validate_text("repository_id", &self.repository_id)?;
        validate_text("alias", &self.alias)?;
        validate_text("cursor", &self.cursor)?;
        validate_text("updated_at", &self.updated_at)?;
        for item in &self.items {
            validate_item_scope(&self.repository_id, item)?;
        }
        for transition in &self.transitions {
            validate_transition_scope(&self.repository_id, transition)?;
        }
        Ok(())
    }
}

/// The local result of one committed page.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PageCommitResult {
    /// Repository scope committed.
    pub repository_id: String,
    /// Alias whose cursor was advanced.
    pub alias: String,
    /// Authoritative cursor after the commit.
    pub cursor: String,
    /// Number of page items processed.
    pub stored_items: usize,
    /// Number of newly inserted reconciliation transitions.
    pub stored_transitions: usize,
}

impl PageCommitResult {
    /// Returns the cursor record's repository scope.
    #[must_use]
    pub fn repository_id(&self) -> &str {
        &self.repository_id
    }

    /// Returns the alias whose cursor was committed.
    #[must_use]
    pub fn alias(&self) -> &str {
        &self.alias
    }

    /// Returns the authoritative cursor value.
    #[must_use]
    pub fn cursor(&self) -> &str {
        &self.cursor
    }
}

/// A focused repository-scoped local inbound state service.
pub struct InboxState {
    state: StateStore,
}

impl fmt::Debug for InboxState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("InboxState")
            .field("state", &self.state)
            .finish()
    }
}

impl InboxState {
    /// Wraps an already opened shared state store.
    #[must_use]
    pub const fn from_state(state: StateStore) -> Self {
        Self { state }
    }

    /// Conventional constructor over an already opened shared state store.
    #[must_use]
    pub const fn new(state: StateStore) -> Self {
        Self::from_state(state)
    }

    /// Opens the OS user-data state store.
    pub fn open() -> Result<Self, StateError> {
        StateStore::open().map(Self::from_state)
    }

    /// Opens an explicit local state path.
    pub fn open_path(path: impl AsRef<Path>) -> Result<Self, StateError> {
        StateStore::open_path(path).map(Self::from_state)
    }

    /// Opens the release-gated OS user-data state store.
    pub fn open_for_release() -> Result<Self, StateError> {
        StateStore::open_for_release().map(Self::from_state)
    }

    /// Opens an explicit release-gated local state path.
    pub fn open_path_for_release(path: impl AsRef<Path>) -> Result<Self, StateError> {
        StateStore::open_path_for_release(path).map(Self::from_state)
    }

    /// Opens an in-memory store for focused contract tests.
    pub fn open_in_memory() -> Result<Self, StateError> {
        StateStore::open_in_memory().map(Self::from_state)
    }

    /// Returns read-only access to the shared substrate.
    #[must_use]
    pub const fn state(&self) -> &StateStore {
        &self.state
    }

    /// Returns mutable access for explicit repository setup and composition.
    pub const fn state_mut(&mut self) -> &mut StateStore {
        &mut self.state
    }

    /// Consumes this facade and returns the shared substrate.
    #[must_use]
    pub fn into_state(self) -> StateStore {
        self.state
    }

    /// Registers or updates one repository identity through the shared store.
    pub fn register_repository(
        &mut self,
        input: &RepositoryInput,
    ) -> InboxResult<RepositoryRecord> {
        self.state
            .upsert_repository(input)
            .map_err(InboxStateError::from)
    }

    /// Alias for [`InboxState::register_repository`].
    pub fn register(&mut self, input: &RepositoryInput) -> InboxResult<RepositoryRecord> {
        self.register_repository(input)
    }

    /// Reads an immutable first snapshot within an exact repository scope.
    pub fn item(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> InboxResult<Option<InboundItemRecord>> {
        self.state
            .inbound_item(repository_id, item_id)
            .map_err(InboxStateError::from)
    }

    /// Reads the separate current snapshot or deleted marker.
    pub fn current(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> InboxResult<Option<InboundCurrentSnapshotRecord>> {
        self.state
            .inbound_current(repository_id, item_id)
            .map_err(InboxStateError::from)
    }

    /// Alias for [`InboxState::current`].
    pub fn current_snapshot(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> InboxResult<Option<InboundCurrentSnapshotRecord>> {
        self.current(repository_id, item_id)
    }

    /// Lists one item's local transition evidence in deterministic order.
    pub fn transitions(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> InboxResult<Vec<InboundTransitionRecord>> {
        self.state
            .repositories()
            .inbound()
            .transitions(repository_id, item_id)
            .map_err(InboxStateError::from)
    }

    /// Returns the immutable first snapshot within an exact repository scope.
    pub fn first_snapshot(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> InboxResult<Option<InboundItemRecord>> {
        self.item(repository_id, item_id)
    }

    /// Stores one first/current pair in its own local transaction.
    pub fn store_item(
        &mut self,
        first: &InboundItemInput,
        current: &InboundCurrentSnapshotInput,
    ) -> InboxResult<InboundItemRecord> {
        self.state
            .store_inbound_item(first, current)
            .map_err(InboxStateError::from)
    }

    /// Alias for [`InboxState::store_item`].
    pub fn store_first_snapshot(
        &mut self,
        first: &InboundItemInput,
        current: &InboundCurrentSnapshotInput,
    ) -> InboxResult<InboundItemRecord> {
        self.store_item(first, current)
    }

    /// Records one edit/deletion transition and optional current snapshot.
    pub fn record_transition(
        &mut self,
        transition: &InboundTransitionInput,
        current: Option<&InboundCurrentSnapshotInput>,
    ) -> InboxResult<InboundTransitionRecord> {
        self.state
            .record_inbound_transition(transition, current)
            .map_err(InboxStateError::from)
    }

    /// Advances one alias cursor in a standalone local transaction.
    pub fn advance_cursor(
        &mut self,
        input: &InboundCursorInput,
    ) -> InboxResult<InboundCursorRecord> {
        self.state
            .advance_inbound_cursor(input)
            .map_err(InboxStateError::from)
    }

    /// Reads one alias cursor within an exact repository scope.
    pub fn cursor(
        &self,
        repository_id: impl AsRef<str>,
        alias: impl AsRef<str>,
    ) -> InboxResult<Option<InboundCursorRecord>> {
        self.state
            .inbound_cursor(repository_id, alias)
            .map_err(InboxStateError::from)
    }

    /// Alias for [`InboxState::cursor`].
    pub fn alias_cursor(
        &self,
        repository_id: impl AsRef<str>,
        alias: impl AsRef<str>,
    ) -> InboxResult<Option<InboundCursorRecord>> {
        self.cursor(repository_id, alias)
    }

    /// Acknowledges one or more items locally and idempotently.
    pub fn acknowledge(
        &mut self,
        repository_id: impl AsRef<str>,
        item_ids: &[impl AsRef<str>],
        acknowledged_at: impl Into<String>,
    ) -> InboxResult<Vec<AcknowledgementRecord>> {
        let repository_id = repository_id.as_ref().to_owned();
        let item_ids = item_ids
            .iter()
            .map(|item_id| item_id.as_ref().to_owned())
            .collect::<Vec<_>>();
        self.state
            .acknowledge_inbound(&repository_id, &item_ids, acknowledged_at.into())
            .map_err(InboxStateError::from)
    }

    /// Alias for [`InboxState::acknowledge`].
    pub fn acknowledge_inbound(
        &mut self,
        repository_id: impl AsRef<str>,
        item_ids: &[impl AsRef<str>],
        acknowledged_at: impl Into<String>,
    ) -> InboxResult<Vec<AcknowledgementRecord>> {
        self.acknowledge(repository_id, item_ids, acknowledged_at)
    }

    /// Alias for [`InboxState::acknowledge`].
    pub fn acknowledge_items(
        &mut self,
        repository_id: impl AsRef<str>,
        item_ids: &[impl AsRef<str>],
        acknowledged_at: impl Into<String>,
    ) -> InboxResult<Vec<AcknowledgementRecord>> {
        self.acknowledge(repository_id, item_ids, acknowledged_at)
    }

    /// Archives one or more items locally and idempotently.
    pub fn archive(
        &mut self,
        repository_id: impl AsRef<str>,
        item_ids: &[impl AsRef<str>],
        archived_at: impl Into<String>,
    ) -> InboxResult<Vec<ArchiveRecord>> {
        let repository_id = repository_id.as_ref().to_owned();
        let item_ids = item_ids
            .iter()
            .map(|item_id| item_id.as_ref().to_owned())
            .collect::<Vec<_>>();
        self.state
            .archive_inbound(&repository_id, &item_ids, archived_at.into())
            .map_err(InboxStateError::from)
    }

    /// Alias for [`InboxState::archive`].
    pub fn archive_inbound(
        &mut self,
        repository_id: impl AsRef<str>,
        item_ids: &[impl AsRef<str>],
        archived_at: impl Into<String>,
    ) -> InboxResult<Vec<ArchiveRecord>> {
        self.archive(repository_id, item_ids, archived_at)
    }

    /// Alias for [`InboxState::archive`].
    pub fn archive_items(
        &mut self,
        repository_id: impl AsRef<str>,
        item_ids: &[impl AsRef<str>],
        archived_at: impl Into<String>,
    ) -> InboxResult<Vec<ArchiveRecord>> {
        self.archive(repository_id, item_ids, archived_at)
    }

    /// Links an inbound item to an already-created local reply draft.
    pub fn link_reply(&mut self, input: &ReplyLinkInput) -> InboxResult<ReplyLinkRecord> {
        self.state
            .link_inbound_reply(input)
            .map_err(InboxStateError::from)
    }

    /// Alias for [`InboxState::link_reply`].
    pub fn link_inbound_reply(&mut self, input: &ReplyLinkInput) -> InboxResult<ReplyLinkRecord> {
        self.link_reply(input)
    }

    /// Reads a local acknowledgement marker.
    pub fn acknowledgement(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> InboxResult<Option<AcknowledgementRecord>> {
        self.state
            .inbound_acknowledgement(repository_id, item_id)
            .map_err(InboxStateError::from)
    }

    /// Reads a local archive marker.
    pub fn archive_record(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> InboxResult<Option<ArchiveRecord>> {
        self.state
            .inbound_archive(repository_id, item_id)
            .map_err(InboxStateError::from)
    }

    /// Reads a local reply link.
    pub fn reply_link(
        &self,
        repository_id: impl AsRef<str>,
        item_id: impl AsRef<str>,
    ) -> InboxResult<Option<ReplyLinkRecord>> {
        self.state
            .inbound_reply_link(repository_id, item_id)
            .map_err(InboxStateError::from)
    }

    /// Commits a complete page and its cursor in one explicit transaction.
    pub fn commit_page(&mut self, page: &InboxPage) -> InboxResult<PageCommitResult> {
        self.commit_page_with_options(page, CommitPageOptions::default())
    }

    /// Alias for [`InboxState::commit_page`].
    pub fn store_page(&mut self, page: &InboxPage) -> InboxResult<PageCommitResult> {
        self.commit_page(page)
    }

    /// Alias for [`InboxState::commit_page`].
    pub fn commit(&mut self, page: &InboxPage) -> InboxResult<PageCommitResult> {
        self.commit_page(page)
    }

    /// Commits a complete page with explicit audit and failure-injection
    /// options.  Every page item, reconciliation event, audit event, and
    /// cursor update shares the same transaction.
    pub fn commit_page_with_options(
        &mut self,
        page: &InboxPage,
        options: CommitPageOptions,
    ) -> InboxResult<PageCommitResult> {
        page.validate()?;
        let mut transaction = self
            .state
            .begin_transaction()
            .map_err(InboxStateError::from)?;
        let result = commit_page_transaction(&mut transaction, page, options);
        match result {
            Ok(value) => {
                transaction.commit().map_err(InboxStateError::from)?;
                Ok(value)
            }
            Err(error) => {
                let rollback = transaction.rollback();
                if let Err(rollback_error) = rollback {
                    return Err(InboxStateError::State(rollback_error));
                }
                Err(error)
            }
        }
    }
}

fn commit_page_transaction(
    transaction: &mut StateTransaction<'_>,
    page: &InboxPage,
    options: CommitPageOptions,
) -> InboxResult<PageCommitResult> {
    let mut stored_transitions = 0;
    let mut stored_items = 0;

    if options.failure_point == Some(FailurePoint::AfterItem(0)) {
        return Err(InboxStateError::Injected {
            phase: FailurePoint::AfterItem(0),
        });
    }

    for (index, item) in page.items.iter().enumerate() {
        if options.failure_point == Some(FailurePoint::BeforeItem(index)) {
            return Err(InboxStateError::Injected {
                phase: FailurePoint::BeforeItem(index),
            });
        }

        transaction
            .repositories()
            .inbound()
            .store_item(&item.first, &item.current)
            .map_err(InboxStateError::from)?;
        if options.write_audit {
            append_item_audit(transaction, &item.first)?;
        }

        for update in &item.transitions {
            if apply_transition(transaction, update)? {
                stored_transitions += 1;
            }
            if options.write_audit {
                append_transition_audit(transaction, &update.transition)?;
            }
        }
        stored_items += 1;

        if options.failure_point == Some(FailurePoint::AfterItem(index + 1)) {
            return Err(InboxStateError::Injected {
                phase: FailurePoint::AfterItem(index + 1),
            });
        }
    }

    for update in &page.transitions {
        if apply_transition(transaction, update)? {
            stored_transitions += 1;
        }
        if options.write_audit {
            append_transition_audit(transaction, &update.transition)?;
        }
    }

    if options.failure_point == Some(FailurePoint::BeforeCursor) {
        return Err(InboxStateError::Injected {
            phase: FailurePoint::BeforeCursor,
        });
    }

    let cursor_input = page.cursor_input();
    let cursor = transaction
        .repositories()
        .inbound()
        .advance_cursor(&cursor_input)
        .map_err(InboxStateError::from)?;
    if options.write_audit {
        append_cursor_audit(transaction, &cursor)?;
    }

    if options.failure_point == Some(FailurePoint::AfterCursor) {
        return Err(InboxStateError::Injected {
            phase: FailurePoint::AfterCursor,
        });
    }

    Ok(PageCommitResult {
        repository_id: page.repository_id.clone(),
        alias: page.alias.clone(),
        cursor: cursor.cursor,
        stored_items,
        stored_transitions,
    })
}

fn apply_transition(
    transaction: &StateTransaction<'_>,
    update: &TransitionUpdate,
) -> InboxResult<bool> {
    let transition = &update.transition;
    let existing = transaction
        .repositories()
        .inbound()
        .transitions(&transition.repository_id, &transition.item_id)
        .map_err(InboxStateError::from)?
        .into_iter()
        .find(|record| record.transition_id == transition.transition_id);
    if let Some(existing) = existing {
        if existing != *transition {
            return Err(InboxStateError::TransitionConflict {
                transition_id: transition.transition_id.clone(),
            });
        }
        return Ok(false);
    }

    transaction
        .repositories()
        .inbound()
        .record_transition(transition, update.current.as_ref())
        .map_err(InboxStateError::from)?;
    Ok(true)
}

fn append_item_audit(
    transaction: &StateTransaction<'_>,
    item: &InboundItemInput,
) -> InboxResult<()> {
    let event_id = stable_event_id(&["item", &item.repository_id, &item.item_id]);
    let event = AuditEvent::new(
        item.repository_id.clone(),
        event_id,
        "inbound_item",
        item.item_id.clone(),
        "stored",
        item.first_observed_at.clone(),
        "system",
        "success",
    );
    append_audit_once(transaction, event)
}

fn append_transition_audit(
    transaction: &StateTransaction<'_>,
    transition: &InboundTransitionInput,
) -> InboxResult<()> {
    let event_id = stable_event_id(&[
        "transition",
        &transition.repository_id,
        &transition.transition_id,
    ]);
    let event = AuditEvent::new(
        transition.repository_id.clone(),
        event_id,
        "inbound_item",
        transition.item_id.clone(),
        transition.transition_type.clone(),
        transition.occurred_at.clone(),
        "system",
        "success",
    );
    append_audit_once(transaction, event)
}

fn append_cursor_audit(
    transaction: &StateTransaction<'_>,
    cursor: &InboundCursorRecord,
) -> InboxResult<()> {
    let event_id = stable_event_id(&[
        "cursor",
        &cursor.repository_id,
        &cursor.alias,
        &cursor.cursor,
    ]);
    let event = AuditEvent::new(
        cursor.repository_id.clone(),
        event_id,
        "inbound_cursor",
        cursor.alias.clone(),
        "advanced",
        cursor.updated_at.clone(),
        "system",
        "success",
    );
    append_audit_once(transaction, event)
}

fn append_audit_once(transaction: &StateTransaction<'_>, event: AuditEvent) -> InboxResult<()> {
    let input = event.to_state_input().map_err(InboxStateError::Audit)?;
    if let Some(existing) = transaction
        .repositories()
        .audit()
        .get(&input.repository_id, &input.event_id)
        .map_err(InboxStateError::from)?
    {
        if audit_input_matches(&existing, &input) {
            return Ok(());
        }
        return Err(InboxStateError::TransitionConflict {
            transition_id: input.event_id,
        });
    }
    transaction
        .append_audit_event(&input)
        .map_err(InboxStateError::from)?;
    Ok(())
}

fn audit_input_matches(
    existing: &repo_com_state::AuditEventRecord,
    input: &AuditEventInput,
) -> bool {
    existing.repository_id == input.repository_id
        && existing.event_id == input.event_id
        && existing.object_type == input.object_type
        && existing.object_id == input.object_id
        && existing.transition == input.transition
        && existing.occurred_at == input.occurred_at
        && existing.actor_kind == input.actor_kind
        && existing.outcome == input.outcome
        && existing.metadata_json == input.metadata_json
}

fn stable_event_id(parts: &[&str]) -> String {
    // FNV-1a keeps identifiers deterministic without introducing a hashing
    // dependency or copying message content into audit metadata.
    let mut hash = 0xcbf29ce484222325_u64;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("inbox-{hash:016x}")
}

fn validate_item_scope(repository_id: &str, item: &PageItem) -> InboxResult<()> {
    if item.first.repository_id != repository_id {
        return Err(InboxStateError::InvalidPage {
            field: "item.repository_id",
        });
    }
    if item.current.repository_id != repository_id || item.current.item_id != item.first.item_id {
        return Err(InboxStateError::InvalidPage {
            field: "item.current",
        });
    }
    for transition in &item.transitions {
        validate_transition_scope(repository_id, transition)?;
        if transition.transition.item_id != item.first.item_id {
            return Err(InboxStateError::InvalidPage {
                field: "item.transition.item_id",
            });
        }
    }
    Ok(())
}

fn validate_transition_scope(repository_id: &str, update: &TransitionUpdate) -> InboxResult<()> {
    if update.transition.repository_id != repository_id {
        return Err(InboxStateError::InvalidPage {
            field: "transition.repository_id",
        });
    }
    if let Some(current) = &update.current
        && (current.repository_id != repository_id || current.item_id != update.transition.item_id)
    {
        return Err(InboxStateError::InvalidPage {
            field: "transition.current",
        });
    }
    Ok(())
}

fn validate_text(field: &'static str, value: &str) -> InboxResult<()> {
    if value.is_empty() || value.len() > 512 || value.contains('\0') {
        return Err(InboxStateError::InvalidPage { field });
    }
    Ok(())
}
