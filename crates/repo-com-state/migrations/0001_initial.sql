-- repo-com state schema version 1.
--
-- Every operational table carries repository_id as part of its key.  The
-- composite foreign keys are intentional: an object identifier is only
-- meaningful inside its configured repository.

CREATE TABLE schema_metadata (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    schema_version INTEGER NOT NULL CHECK (schema_version = 1),
    created_at TEXT NOT NULL CHECK (length(created_at) > 0)
);

CREATE TABLE repositories (
    repository_id TEXT PRIMARY KEY NOT NULL CHECK (length(repository_id) > 0),
    workspace_id TEXT NOT NULL CHECK (length(workspace_id) > 0),
    config_hash TEXT NOT NULL CHECK (length(config_hash) > 0),
    created_at TEXT NOT NULL CHECK (length(created_at) > 0),
    updated_at TEXT NOT NULL CHECK (length(updated_at) > 0)
);

CREATE TABLE drafts (
    repository_id TEXT NOT NULL,
    draft_id TEXT NOT NULL CHECK (length(draft_id) > 0),
    event_type TEXT NOT NULL CHECK (length(event_type) > 0),
    destination_alias TEXT NOT NULL CHECK (length(destination_alias) > 0),
    status TEXT NOT NULL CHECK (status IN (
        'draft', 'approved', 'policy_eligible', 'blocked', 'expired', 'sent'
    )),
    current_revision INTEGER NOT NULL DEFAULT 0 CHECK (current_revision >= 0),
    expiry_at TEXT,
    reply_to_inbound_item_id TEXT,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL CHECK (length(created_at) > 0),
    updated_at TEXT NOT NULL CHECK (length(updated_at) > 0),
    PRIMARY KEY (repository_id, draft_id),
    FOREIGN KEY (repository_id) REFERENCES repositories(repository_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE draft_revisions (
    repository_id TEXT NOT NULL,
    draft_id TEXT NOT NULL CHECK (length(draft_id) > 0),
    revision INTEGER NOT NULL CHECK (revision > 0),
    content_hash TEXT NOT NULL CHECK (length(content_hash) > 0),
    body TEXT NOT NULL,
    metadata_json TEXT NOT NULL DEFAULT '{}',
    destination_alias TEXT NOT NULL CHECK (length(destination_alias) > 0),
    resolved_destination TEXT NOT NULL CHECK (length(resolved_destination) > 0),
    expiry_at TEXT,
    lifecycle_state TEXT NOT NULL CHECK (lifecycle_state IN (
        'draft', 'approved', 'policy_eligible', 'blocked', 'expired', 'sent'
    )),
    reply_to_inbound_item_id TEXT,
    created_at TEXT NOT NULL CHECK (length(created_at) > 0),
    PRIMARY KEY (repository_id, draft_id, revision),
    FOREIGN KEY (repository_id, draft_id) REFERENCES drafts(repository_id, draft_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE approvals (
    repository_id TEXT NOT NULL,
    approval_id TEXT NOT NULL CHECK (length(approval_id) > 0),
    draft_id TEXT NOT NULL CHECK (length(draft_id) > 0),
    revision INTEGER NOT NULL CHECK (revision > 0),
    approval_state TEXT NOT NULL CHECK (approval_state IN ('approved', 'revoked')),
    actor_kind TEXT NOT NULL CHECK (length(actor_kind) > 0),
    operator_reference TEXT,
    approved_at TEXT NOT NULL CHECK (length(approved_at) > 0),
    revoked_at TEXT,
    PRIMARY KEY (repository_id, approval_id),
    UNIQUE (repository_id, draft_id, revision),
    FOREIGN KEY (repository_id, draft_id, revision)
        REFERENCES draft_revisions(repository_id, draft_id, revision)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE policy_activations (
    repository_id TEXT NOT NULL,
    activation_id TEXT NOT NULL CHECK (length(activation_id) > 0),
    config_hash TEXT NOT NULL CHECK (length(config_hash) > 0),
    policy_tuple_hash TEXT NOT NULL CHECK (length(policy_tuple_hash) > 0),
    event_type TEXT NOT NULL CHECK (length(event_type) > 0),
    destination_alias TEXT NOT NULL CHECK (length(destination_alias) > 0),
    severity TEXT NOT NULL CHECK (length(severity) > 0),
    activated_at TEXT NOT NULL CHECK (length(activated_at) > 0),
    deactivated_at TEXT,
    active INTEGER NOT NULL DEFAULT 1 CHECK (active IN (0, 1)),
    PRIMARY KEY (repository_id, activation_id),
    FOREIGN KEY (repository_id) REFERENCES repositories(repository_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE delivery_attempts (
    repository_id TEXT NOT NULL,
    attempt_id TEXT NOT NULL CHECK (length(attempt_id) > 0),
    draft_id TEXT NOT NULL CHECK (length(draft_id) > 0),
    revision INTEGER NOT NULL CHECK (revision > 0),
    attempt_number INTEGER NOT NULL CHECK (attempt_number > 0),
    claim_nonce TEXT NOT NULL CHECK (length(claim_nonce) > 0),
    state TEXT NOT NULL CHECK (state IN (
        'unclaimed', 'claimed', 'accepted', 'failed', 'unknown'
    )),
    claimed_at TEXT NOT NULL CHECK (length(claimed_at) > 0),
    completed_at TEXT,
    remote_message_id TEXT,
    failure_code TEXT,
    PRIMARY KEY (repository_id, attempt_id),
    UNIQUE (repository_id, draft_id, revision, attempt_number),
    UNIQUE (repository_id, claim_nonce),
    FOREIGN KEY (repository_id, draft_id, revision)
        REFERENCES draft_revisions(repository_id, draft_id, revision)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE inbound_cursors (
    repository_id TEXT NOT NULL,
    alias TEXT NOT NULL CHECK (length(alias) > 0),
    cursor TEXT NOT NULL CHECK (length(cursor) > 0),
    updated_at TEXT NOT NULL CHECK (length(updated_at) > 0),
    PRIMARY KEY (repository_id, alias),
    FOREIGN KEY (repository_id) REFERENCES repositories(repository_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE inbound_items (
    repository_id TEXT NOT NULL,
    item_id TEXT NOT NULL CHECK (length(item_id) > 0),
    channel_id TEXT NOT NULL CHECK (length(channel_id) > 0),
    author_id TEXT NOT NULL CHECK (length(author_id) > 0),
    first_content TEXT NOT NULL,
    first_attachments_json TEXT NOT NULL DEFAULT '[]',
    first_observed_at TEXT NOT NULL CHECK (length(first_observed_at) > 0),
    created_at TEXT NOT NULL CHECK (length(created_at) > 0),
    PRIMARY KEY (repository_id, item_id),
    FOREIGN KEY (repository_id) REFERENCES repositories(repository_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE inbound_current_snapshots (
    repository_id TEXT NOT NULL,
    item_id TEXT NOT NULL CHECK (length(item_id) > 0),
    current_content TEXT,
    current_attachments_json TEXT NOT NULL DEFAULT '[]',
    deleted INTEGER NOT NULL DEFAULT 0 CHECK (
        deleted IN (0, 1)
        AND (deleted = 0 OR current_content IS NULL)
    ),
    observed_at TEXT NOT NULL CHECK (length(observed_at) > 0),
    PRIMARY KEY (repository_id, item_id),
    FOREIGN KEY (repository_id, item_id) REFERENCES inbound_items(repository_id, item_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE inbound_item_transitions (
    repository_id TEXT NOT NULL,
    transition_id TEXT NOT NULL CHECK (length(transition_id) > 0),
    item_id TEXT NOT NULL CHECK (length(item_id) > 0),
    transition_type TEXT NOT NULL CHECK (transition_type IN (
        'created', 'edited', 'deleted', 'acknowledged', 'archived', 'reply_linked'
    )),
    content TEXT,
    occurred_at TEXT NOT NULL CHECK (length(occurred_at) > 0),
    metadata_json TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (repository_id, transition_id),
    FOREIGN KEY (repository_id, item_id) REFERENCES inbound_items(repository_id, item_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE inbound_acknowledgements (
    repository_id TEXT NOT NULL,
    item_id TEXT NOT NULL CHECK (length(item_id) > 0),
    acknowledged_at TEXT NOT NULL CHECK (length(acknowledged_at) > 0),
    PRIMARY KEY (repository_id, item_id),
    FOREIGN KEY (repository_id, item_id) REFERENCES inbound_items(repository_id, item_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE inbound_archives (
    repository_id TEXT NOT NULL,
    item_id TEXT NOT NULL CHECK (length(item_id) > 0),
    archived_at TEXT NOT NULL CHECK (length(archived_at) > 0),
    PRIMARY KEY (repository_id, item_id),
    FOREIGN KEY (repository_id, item_id) REFERENCES inbound_items(repository_id, item_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE inbound_reply_links (
    repository_id TEXT NOT NULL,
    item_id TEXT NOT NULL CHECK (length(item_id) > 0),
    reply_draft_id TEXT NOT NULL CHECK (length(reply_draft_id) > 0),
    linked_at TEXT NOT NULL CHECK (length(linked_at) > 0),
    PRIMARY KEY (repository_id, item_id),
    UNIQUE (repository_id, reply_draft_id),
    FOREIGN KEY (repository_id, item_id) REFERENCES inbound_items(repository_id, item_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT,
    FOREIGN KEY (repository_id, reply_draft_id) REFERENCES drafts(repository_id, draft_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE TABLE audit_events (
    audit_id INTEGER PRIMARY KEY AUTOINCREMENT,
    repository_id TEXT NOT NULL CHECK (length(repository_id) > 0),
    event_id TEXT NOT NULL CHECK (length(event_id) > 0),
    object_type TEXT NOT NULL CHECK (length(object_type) > 0),
    object_id TEXT NOT NULL CHECK (length(object_id) > 0),
    transition TEXT NOT NULL CHECK (length(transition) > 0),
    occurred_at TEXT NOT NULL CHECK (length(occurred_at) > 0),
    actor_kind TEXT NOT NULL CHECK (length(actor_kind) > 0),
    outcome TEXT NOT NULL CHECK (length(outcome) > 0),
    metadata_json TEXT NOT NULL DEFAULT '{}',
    UNIQUE (repository_id, event_id),
    FOREIGN KEY (repository_id) REFERENCES repositories(repository_id)
        ON UPDATE RESTRICT ON DELETE RESTRICT
);

CREATE INDEX drafts_by_repository ON drafts(repository_id, updated_at, draft_id);
CREATE INDEX draft_revisions_by_repository ON draft_revisions(repository_id, draft_id, revision);
CREATE INDEX approvals_by_draft ON approvals(repository_id, draft_id, revision);
CREATE INDEX policy_activations_by_tuple ON policy_activations(
    repository_id, config_hash, policy_tuple_hash, active
);
CREATE INDEX delivery_attempts_by_revision ON delivery_attempts(
    repository_id, draft_id, revision, attempt_number
);
CREATE INDEX inbound_items_by_channel ON inbound_items(repository_id, channel_id, first_observed_at);
CREATE INDEX inbound_transitions_by_item ON inbound_item_transitions(repository_id, item_id, occurred_at);
CREATE INDEX audit_events_by_object ON audit_events(repository_id, object_type, object_id, occurred_at);

-- A revision is evidence, not a mutable current row.  Corrections are new
-- revisions; the trigger also protects the invariant when raw SQL is used.
CREATE TRIGGER draft_revisions_immutable_update
BEFORE UPDATE ON draft_revisions
BEGIN
    SELECT RAISE(ABORT, 'draft revisions are immutable');
END;

CREATE TRIGGER draft_revisions_immutable_delete
BEFORE DELETE ON draft_revisions
BEGIN
    SELECT RAISE(ABORT, 'draft revisions are immutable');
END;

-- The first remote observation is similarly immutable.  Current content and
-- deletion are represented by the separate current/transition tables.
CREATE TRIGGER inbound_items_first_snapshot_immutable_update
BEFORE UPDATE ON inbound_items
BEGIN
    SELECT RAISE(ABORT, 'inbound first snapshots are immutable');
END;

CREATE TRIGGER inbound_items_first_snapshot_immutable_delete
BEFORE DELETE ON inbound_items
BEGIN
    SELECT RAISE(ABORT, 'inbound first snapshots are immutable');
END;

-- Audit evidence is append-only.  There is intentionally no update/delete API
-- and the database rejects those operations even for local SQL callers.
CREATE TRIGGER audit_events_append_only_update
BEFORE UPDATE ON audit_events
BEGIN
    SELECT RAISE(ABORT, 'audit events are append-only');
END;

CREATE TRIGGER audit_events_append_only_delete
BEFORE DELETE ON audit_events
BEGIN
    SELECT RAISE(ABORT, 'audit events are append-only');
END;

INSERT INTO schema_metadata(singleton, schema_version, created_at)
VALUES (1, 1, '1970-01-01T00:00:00Z');
