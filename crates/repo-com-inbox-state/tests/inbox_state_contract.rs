use crate::{
    CommitPageOptions, CurrentSnapshot, DraftInput, FailurePoint, InboundItemInput,
    InboundTransitionInput, InboxPage, InboxState, InboxStateError, PageItem, ReplyLinkInput,
    RepositoryInput, StateError, TransitionUpdate, compare_cursor_values,
};

fn state() -> InboxState {
    InboxState::open_in_memory().expect("open in-memory inbound state")
}

fn register(state: &mut InboxState, repository_id: &str) {
    state
        .register_repository(&RepositoryInput::new(
            repository_id,
            format!("workspace-{repository_id}"),
            format!("config-{repository_id}"),
            "2026-01-01T00:00:00Z",
        ))
        .expect("register repository");
}

fn first(repository_id: &str, item_id: &str, content: &str, observed_at: &str) -> InboundItemInput {
    InboundItemInput::new(
        repository_id,
        item_id,
        "channel-1",
        "human-1",
        content,
        observed_at,
    )
}

fn current(
    repository_id: &str,
    item_id: &str,
    content: Option<&str>,
    deleted: bool,
    observed_at: &str,
) -> CurrentSnapshot {
    CurrentSnapshot::new(
        repository_id,
        item_id,
        content.map(str::to_owned),
        deleted,
        observed_at,
    )
}

fn transition(
    repository_id: &str,
    transition_id: &str,
    item_id: &str,
    transition_type: &str,
    content: Option<&str>,
    occurred_at: &str,
) -> InboundTransitionInput {
    InboundTransitionInput::new(
        repository_id,
        transition_id,
        item_id,
        transition_type,
        content.map(str::to_owned),
        occurred_at,
    )
}

fn page(repository_id: &str, alias: &str, cursor: &str, updated_at: &str) -> InboxPage {
    InboxPage::new(repository_id, alias, cursor, updated_at)
}

#[test]
fn first_snapshot_survives_current_lifecycle_and_reply_transitions() {
    let mut inbox = state();
    register(&mut inbox, "repo-a");

    let initial = first("repo-a", "item-a", "first content", "2026-01-01T00:00:01Z");
    let initial_current = current(
        "repo-a",
        "item-a",
        Some("first content"),
        false,
        "2026-01-01T00:00:01Z",
    );
    let edit = transition(
        "repo-a",
        "edit-a",
        "item-a",
        "edited",
        Some("edited content"),
        "2026-01-01T00:00:02Z",
    );
    let mut fetched = page("repo-a", "inbound", "100", "2026-01-01T00:00:02Z");
    fetched.add_item(PageItem::with_transitions(
        initial.clone(),
        initial_current,
        vec![TransitionUpdate::new(edit)],
    ));
    let result = inbox.commit_page(&fetched).expect("commit fetched page");
    assert_eq!(result.stored_items, 1);
    assert_eq!(result.stored_transitions, 1);

    let first_record = inbox
        .item("repo-a", "item-a")
        .expect("read first snapshot")
        .expect("stored item");
    assert_eq!(first_record.first_content, "first content");
    assert_eq!(
        inbox
            .current("repo-a", "item-a")
            .expect("read current snapshot")
            .expect("current snapshot")
            .current_content
            .as_deref(),
        Some("edited content")
    );

    inbox
        .record_transition(
            &transition(
                "repo-a",
                "delete-a",
                "item-a",
                "deleted",
                None,
                "2026-01-01T00:00:03Z",
            ),
            None,
        )
        .expect("record deletion");
    assert!(
        inbox
            .current("repo-a", "item-a")
            .expect("read deleted marker")
            .expect("current row")
            .deleted
    );
    assert_eq!(
        inbox
            .item("repo-a", "item-a")
            .expect("read first after deletion")
            .expect("first row")
            .first_content,
        "first content"
    );

    let first_ack = inbox
        .acknowledge("repo-a", &["item-a"], "2026-01-01T00:00:04Z")
        .expect("acknowledge");
    let repeated_ack = inbox
        .acknowledge("repo-a", &["item-a"], "2026-01-01T00:00:05Z")
        .expect("repeat acknowledge");
    assert_eq!(
        first_ack[0].acknowledged_at,
        repeated_ack[0].acknowledged_at
    );
    assert_eq!(
        inbox
            .acknowledgement("repo-a", "item-a")
            .expect("read acknowledgement")
            .expect("ack row")
            .acknowledged_at,
        "2026-01-01T00:00:04Z"
    );

    let first_archive = inbox
        .archive("repo-a", &["item-a"], "2026-01-01T00:00:06Z")
        .expect("archive");
    let repeated_archive = inbox
        .archive("repo-a", &["item-a"], "2026-01-01T00:00:07Z")
        .expect("repeat archive");
    assert_eq!(
        first_archive[0].archived_at,
        repeated_archive[0].archived_at
    );

    inbox
        .state_mut()
        .create_draft(&DraftInput::new(
            "repo-a",
            "reply-a",
            "inbound_reply",
            "inbound",
            "2026-01-01T00:00:08Z",
        ))
        .expect("create local reply draft");
    let link = inbox
        .link_reply(&ReplyLinkInput::new(
            "repo-a",
            "item-a",
            "reply-a",
            "2026-01-01T00:00:09Z",
        ))
        .expect("link reply draft");
    assert_eq!(link.reply_draft_id, "reply-a");
    assert_eq!(
        inbox
            .item("repo-a", "item-a")
            .expect("read immutable first after lifecycle")
            .expect("first row")
            .first_content,
        "first content"
    );

    let transitions = inbox
        .transitions("repo-a", "item-a")
        .expect("read transition history");
    let transition_types = transitions
        .iter()
        .map(|record| record.transition_type.as_str())
        .collect::<Vec<_>>();
    assert!(transition_types.contains(&"created"));
    assert!(transition_types.contains(&"edited"));
    assert!(transition_types.contains(&"deleted"));
    assert!(transition_types.contains(&"acknowledged"));
    assert!(transition_types.contains(&"archived"));
    assert!(transition_types.contains(&"reply_linked"));
}

#[test]
fn injected_item_failure_rolls_back_page_and_preserves_previous_cursor() {
    let mut inbox = state();
    register(&mut inbox, "repo-a");

    let initial = page("repo-a", "inbound", "100", "2026-01-01T00:00:01Z");
    inbox.commit_page(&initial).expect("commit initial cursor");

    let mut retry_page = page("repo-a", "inbound", "200", "2026-01-01T00:00:02Z");
    retry_page
        .add_item(PageItem::new(
            first("repo-a", "item-a", "a", "2026-01-01T00:00:02Z"),
            current("repo-a", "item-a", Some("a"), false, "2026-01-01T00:00:02Z"),
        ))
        .add_item(PageItem::new(
            first("repo-a", "item-b", "b", "2026-01-01T00:00:02Z"),
            current("repo-a", "item-b", Some("b"), false, "2026-01-01T00:00:02Z"),
        ));

    let failure =
        inbox.commit_page_with_options(&retry_page, CommitPageOptions::fail_after_item(1));
    assert!(matches!(
        failure,
        Err(InboxStateError::Injected {
            phase: FailurePoint::AfterItem(1)
        })
    ));
    assert_eq!(
        inbox
            .cursor("repo-a", "inbound")
            .expect("read previous cursor")
            .expect("cursor row")
            .cursor,
        "100"
    );
    assert!(
        inbox
            .item("repo-a", "item-a")
            .expect("read rolled-back item a")
            .is_none()
    );
    assert!(
        inbox
            .item("repo-a", "item-b")
            .expect("read rolled-back item b")
            .is_none()
    );

    let committed = inbox.commit_page(&retry_page).expect("retry complete page");
    assert_eq!(committed.cursor, "200");
    assert_eq!(committed.stored_items, 2);
    assert!(
        inbox
            .item("repo-a", "item-a")
            .expect("read committed item a")
            .is_some()
    );
    assert!(
        inbox
            .item("repo-a", "item-b")
            .expect("read committed item b")
            .is_some()
    );
}

#[test]
fn cursors_are_per_alias_and_per_repository_and_only_move_forward() {
    let mut inbox = state();
    register(&mut inbox, "repo-a");
    register(&mut inbox, "repo-b");

    inbox
        .commit_page(&page("repo-a", "alpha", "10", "2026-01-01T00:00:01Z"))
        .expect("repo a alpha");
    inbox
        .commit_page(&page("repo-a", "beta", "20", "2026-01-01T00:00:02Z"))
        .expect("repo a beta");
    inbox
        .commit_page(&page("repo-b", "alpha", "5", "2026-01-01T00:00:03Z"))
        .expect("repo b alpha");

    assert_eq!(
        inbox
            .cursor("repo-a", "alpha")
            .expect("read a alpha")
            .expect("a alpha")
            .cursor,
        "10"
    );
    assert_eq!(
        inbox
            .cursor("repo-a", "beta")
            .expect("read a beta")
            .expect("a beta")
            .cursor,
        "20"
    );
    assert_eq!(
        inbox
            .cursor("repo-b", "alpha")
            .expect("read b alpha")
            .expect("b alpha")
            .cursor,
        "5"
    );

    let repeated = inbox
        .commit_page(&page("repo-a", "alpha", "10", "2026-01-01T00:00:04Z"))
        .expect("same cursor is idempotent");
    assert_eq!(repeated.cursor, "10");
    let backwards = inbox.commit_page(&page("repo-a", "alpha", "9", "2026-01-01T00:00:05Z"));
    assert!(backwards.is_err());
    assert_eq!(compare_cursor_values("9", "10"), std::cmp::Ordering::Less);
}

#[test]
fn acknowledgement_and_archive_are_idempotent_local_transitions() {
    let mut inbox = state();
    register(&mut inbox, "repo-a");
    let mut fetched = page("repo-a", "inbound", "1", "2026-01-01T00:00:01Z");
    fetched.add_item(PageItem::new(
        first("repo-a", "item-a", "body", "2026-01-01T00:00:01Z"),
        current(
            "repo-a",
            "item-a",
            Some("body"),
            false,
            "2026-01-01T00:00:01Z",
        ),
    ));
    inbox.commit_page(&fetched).expect("store item");

    for timestamp in ["2026-01-01T00:00:02Z", "2026-01-01T00:00:03Z"] {
        let acknowledged = inbox
            .acknowledge("repo-a", &["item-a"], timestamp)
            .expect("local acknowledgement");
        assert_eq!(acknowledged[0].acknowledged_at, "2026-01-01T00:00:02Z");
    }
    for timestamp in ["2026-01-01T00:00:04Z", "2026-01-01T00:00:05Z"] {
        let archived = inbox
            .archive("repo-a", &["item-a"], timestamp)
            .expect("local archive");
        assert_eq!(archived[0].archived_at, "2026-01-01T00:00:04Z");
    }

    assert_eq!(
        inbox
            .item("repo-a", "item-a")
            .expect("read first")
            .expect("item")
            .first_content,
        "body"
    );
    assert!(
        inbox
            .current("repo-a", "item-a")
            .expect("read current")
            .expect("current")
            .current_content
            .as_deref()
            .is_some()
    );
}

#[test]
fn every_inbound_read_and_mutation_is_repository_scoped() {
    let mut inbox = state();
    register(&mut inbox, "repo-a");
    register(&mut inbox, "repo-b");

    let mut page_a = page("repo-a", "inbound", "10", "2026-01-01T00:00:01Z");
    page_a.add_item(PageItem::new(
        first("repo-a", "shared-item", "a", "2026-01-01T00:00:01Z"),
        current(
            "repo-a",
            "shared-item",
            Some("a"),
            false,
            "2026-01-01T00:00:01Z",
        ),
    ));
    inbox.commit_page(&page_a).expect("store repo a item");
    let mut page_b = page("repo-b", "inbound", "20", "2026-01-01T00:00:02Z");
    page_b.add_item(PageItem::new(
        first("repo-b", "shared-item", "b", "2026-01-01T00:00:02Z"),
        current(
            "repo-b",
            "shared-item",
            Some("b"),
            false,
            "2026-01-01T00:00:02Z",
        ),
    ));
    inbox.commit_page(&page_b).expect("store repo b item");

    assert_eq!(
        inbox
            .item("repo-a", "shared-item")
            .expect("read a item")
            .expect("a item")
            .first_content,
        "a"
    );
    assert_eq!(
        inbox
            .item("repo-b", "shared-item")
            .expect("read b item")
            .expect("b item")
            .first_content,
        "b"
    );
    assert!(
        inbox
            .item("repo-a", "missing-in-b")
            .expect("missing item")
            .is_none()
    );
    assert!(
        inbox
            .current("repo-b", "shared-item")
            .expect("read b current")
            .expect("b current")
            .current_content
            .as_deref()
            == Some("b")
    );
    assert!(
        inbox
            .cursor("repo-b", "inbound")
            .expect("read b cursor")
            .expect("b cursor")
            .cursor
            == "20"
    );

    let mut mixed_scope_page = page("repo-a", "inbound", "30", "2026-01-01T00:00:03Z");
    mixed_scope_page.add_item(PageItem::new(
        first(
            "repo-b",
            "mixed-item",
            "wrong scope",
            "2026-01-01T00:00:03Z",
        ),
        current(
            "repo-b",
            "mixed-item",
            Some("wrong scope"),
            false,
            "2026-01-01T00:00:03Z",
        ),
    ));
    assert!(matches!(
        inbox.commit_page(&mixed_scope_page),
        Err(InboxStateError::InvalidPage { .. })
    ));
    assert!(
        inbox
            .cursor("repo-a", "inbound")
            .expect("read cursor after rejected page")
            .expect("cursor")
            .cursor
            == "10"
    );

    let wrong_ack = inbox.acknowledge("repo-b", &["only-in-a"], "2026-01-01T00:00:03Z");
    assert!(matches!(
        wrong_ack,
        Err(InboxStateError::State(StateError::NotFound { .. }))
    ));
    assert!(
        inbox
            .acknowledgement("repo-a", "shared-item")
            .expect("read a acknowledgement")
            .is_none()
    );

    let wrong_archive = inbox.archive("repo-b", &["only-in-a"], "2026-01-01T00:00:04Z");
    assert!(matches!(
        wrong_archive,
        Err(InboxStateError::State(StateError::NotFound { .. }))
    ));

    inbox
        .state_mut()
        .create_draft(&DraftInput::new(
            "repo-a",
            "reply-a",
            "inbound_reply",
            "inbound",
            "2026-01-01T00:00:05Z",
        ))
        .expect("create a reply draft");
    let wrong_link = inbox.link_reply(&ReplyLinkInput::new(
        "repo-b",
        "shared-item",
        "reply-a",
        "2026-01-01T00:00:06Z",
    ));
    assert!(matches!(
        wrong_link,
        Err(InboxStateError::State(StateError::NotFound { .. }))
    ));
    assert!(
        inbox
            .reply_link("repo-b", "shared-item")
            .expect("read wrong-scope link")
            .is_none()
    );

    let correct_link = inbox
        .link_reply(&ReplyLinkInput::new(
            "repo-a",
            "shared-item",
            "reply-a",
            "2026-01-01T00:00:07Z",
        ))
        .expect("link same-scope reply");
    assert_eq!(correct_link.repository_id, "repo-a");
    assert!(
        inbox
            .reply_link("repo-b", "shared-item")
            .expect("read cross link")
            .is_none()
    );
}

#[test]
fn a_reused_transition_id_with_different_evidence_rolls_back_the_page() {
    let mut inbox = state();
    register(&mut inbox, "repo-a");
    // Seed the item first so the transition can be recorded independently.
    let mut seed = page("repo-a", "inbound", "1", "2026-01-01T00:00:01Z");
    seed.add_item(PageItem::new(
        first("repo-a", "item-a", "first", "2026-01-01T00:00:01Z"),
        current(
            "repo-a",
            "item-a",
            Some("first"),
            false,
            "2026-01-01T00:00:01Z",
        ),
    ));
    inbox.commit_page(&seed).expect("seed item");

    let first_transition = transition(
        "repo-a",
        "same-transition",
        "item-a",
        "edited",
        Some("one"),
        "2026-01-01T00:00:02Z",
    );
    let mut first_page = page("repo-a", "inbound", "2", "2026-01-01T00:00:02Z");
    first_page.add_transition(first_transition);
    inbox
        .commit_page(&first_page)
        .expect("store first transition");

    let conflicting = transition(
        "repo-a",
        "same-transition",
        "item-a",
        "edited",
        Some("different"),
        "2026-01-01T00:00:03Z",
    );
    let mut conflict_page = page("repo-a", "inbound", "3", "2026-01-01T00:00:03Z");
    conflict_page.add_transition(conflicting);
    let error = inbox
        .commit_page(&conflict_page)
        .expect_err("conflicting transition must fail");
    assert!(matches!(error, InboxStateError::TransitionConflict { .. }));
    assert_eq!(
        inbox
            .cursor("repo-a", "inbound")
            .expect("read cursor after conflict")
            .expect("cursor")
            .cursor,
        "2"
    );
}
