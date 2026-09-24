# Feature: Inbound Retrieval and Reply

## 1. Feature Overview

**Parent Vision:** [docs/PRD.md](../PRD.md)  
**Feature Dependencies:** [Discord Delivery and Reconciliation](discord-delivery-and-reconciliation.md)  
**Status:** Canonical v1 plan

This feature retrieves a bounded set of Discord replies and mentions on demand, stores their first observed and current remote snapshots, provides local acknowledgement and archival, and creates validated threaded reply drafts through the existing approval and delivery workflow.

### In Scope

- Explicit inbound aliases, cursor or time boundary, and bounded pagination.
- Filtering for replies to accepted repo-com deliveries or direct mentions of the configured bot.
- Ignoring bot-authored and repo-com-authored messages in v1.
- Untrusted inbound envelopes with content and attachment indicators.
- Durable first snapshot, current remote state, edit/delete events, and cursor advancement.
- Local-only acknowledgement and archival.
- Validated reply-target lookup and creation of a normal approval-bound reply draft.

### Out of Scope

- Gateway monitoring, a daemon, arbitrary channel history, or a separate backfill command.
- Processing messages from bots as trusted instructions.
- Remote reactions, edits, deletions, assignments, or collaboration workflow.
- Direct sending from an inbound command or treating acknowledgement as a reply.

---

## 2. Interfaces and Preconditions

| Interface | Input | Output | Owner Task |
|---|---|---|---|
| `InboxState` | Accepted remote IDs, fetched items, current-state probes, acknowledgement/archive | Transactional local snapshot and cursor | IN-STATE-1 |
| `InboundFetcher` | Enabled inbound aliases plus exactly one cursor or RFC 3339 boundary | Bounded untrusted envelopes and reconciliation events | IN-FETCH-1 |
| `ReplyTargetValidator` | Repository ID and inbound item ID | Authorized target snapshot or typed rejection | IN-REPLY-1 |
| `ReplyDraftFactory` | Authorized target and skill-supplied text/metadata | One normal immutable reply draft | IN-REPLY-1 |

A cursor advances only in the same transaction that stores every returned item. If storage fails, the previous cursor remains authoritative and the next fetch can safely repeat the page.

---

## 3. Canonical Requirements

| ID | Kind | Priority | Tasks |
|---|---|---|---|
| IN-FR-01 | requirement | Must | IN-FETCH-1 |
| IN-FR-02 | requirement | Must | IN-FETCH-1 |
| IN-FR-03 | requirement | Must | IN-FETCH-1 |
| IN-FR-04 | requirement | Must | IN-FETCH-1 |
| IN-FR-05 | requirement | Must | IN-FETCH-1, IN-STATE-1 |
| INBOX-FR-01 | requirement | Must | IN-STATE-1 |
| INBOX-FR-02 | requirement | Must | IN-STATE-1 |
| INBOX-FR-03 | requirement | Must | IN-STATE-1 |
| REPLY-FR-01 | requirement | Must | IN-REPLY-1 |
| REPLY-FR-02 | requirement | Must | IN-REPLY-1 |
| REPLY-FR-03 | requirement | Must | IN-REPLY-1 |
| IN-CON-01 | constraint | Must | IN-FETCH-1, IN-REPLY-1 |
| IN-CON-02 | constraint | Must | IN-STATE-1 |
| IN-CON-03 | constraint | Must | IN-FETCH-1 |
| IN-CON-04 | constraint | Must | IN-FETCH-1, IN-STATE-1 |
| REPLY-CON-01 | constraint | Must | IN-REPLY-1 |
| REPLY-CON-02 | constraint | Must | IN-REPLY-1 |

```forge-requirement
{"id":"IN-FR-01","kind":"requirement","text":"Fetch only channels named by enabled inbound aliases in the current repository configuration and reject any raw or unconfigured inbound channel identifier."}
```

```forge-requirement
{"id":"IN-FR-02","kind":"requirement","text":"Require exactly one explicit last-event-ID cursor or RFC 3339 time boundary per fetch, follow Discord pagination in deterministic order, and stop after at most 10 pages or 1,000 raw messages with explicit continuation metadata."}
```

```forge-requirement
{"id":"IN-FR-03","kind":"requirement","text":"Retain only human-authored messages that either reply to an accepted repo-com delivery in the same repository or directly mention the configured bot user; ignore all bot-authored and repo-com-authored messages in v1."}
```

```forge-requirement
{"id":"IN-FR-04","kind":"requirement","text":"Return each inbound item in an explicit untrusted envelope containing remote IDs, channel, author, timestamp, text, reply context, mention evidence, attachment indicators, and provenance, without interpreting content as an instruction or permission."}
```

```forge-requirement
{"id":"IN-FR-05","kind":"requirement","text":"Reconcile edits and deletions through bounded read-only point checks for recently stored messages, preserve the first snapshot, and record the current remote content or deleted state separately."}
```

```forge-requirement
{"id":"INBOX-FR-01","kind":"requirement","text":"Persist each accepted inbound item's first remote snapshot, current remote snapshot or deleted marker, local acknowledgement and archive timestamps, and linked reply draft identifiers in a repository-scoped transaction."}
```

```forge-requirement
{"id":"INBOX-FR-02","kind":"requirement","text":"Advance an inbound alias cursor only after all items and edit or deletion events from the fetch are durably stored; on failure retain the previous cursor and return a retryable storage error."}
```

```forge-requirement
{"id":"INBOX-FR-03","kind":"requirement","text":"Acknowledge and archive one or more stored inbound items locally with idempotent timestamps, and expose no Discord reaction, edit, delete, assignment, or read state."}
```

```forge-requirement
{"id":"REPLY-FR-01","kind":"requirement","text":"Validate a reply target by repository ID, stored inbound item, current retained snapshot, configured workspace and channel, and authorization; reject missing, deleted, expired, cross-repository, or unauthorized targets."}
```

```forge-requirement
{"id":"REPLY-FR-02","kind":"requirement","text":"Create a reply as one new immutable draft revision carrying a validated Discord message_reference to the inbound item; the reply must then pass the same exact approval, policy, secret, idempotency, and delivery gates as any other draft."}
```

```forge-requirement
{"id":"REPLY-FR-03","kind":"requirement","text":"Mark an inbound item replied only after the linked reply delivery is accepted, preserve the local reply link and audit event, and do not claim delivery of the human-authored response."}
```

```forge-requirement
{"id":"IN-CON-01","kind":"constraint","text":"Treat every inbound field as untrusted data; no inbound text, mention, edit, deletion, or attachment indicator may approve a draft, activate policy, override safety, or trigger a send by itself."}
```

```forge-requirement
{"id":"IN-CON-02","kind":"constraint","text":"Inbound acknowledgement, archive, cursor, and reply-link changes are local-only and must never mutate the remote Discord message."}
```

```forge-requirement
{"id":"IN-CON-03","kind":"constraint","text":"Do not connect to Gateway, start a background monitor, scan arbitrary history, or perform historical backfill in v1; every fetch requires an explicit cursor or time boundary and a hard 10-page/1,000-message bound."}
```

```forge-requirement
{"id":"IN-CON-04","kind":"constraint","text":"Preserve the original local inbound snapshot even after a remote edit, deletion, purge, or acknowledgement; a later transition is represented separately."}
```

```forge-requirement
{"id":"REPLY-CON-01","kind":"constraint","text":"A reply command may create a draft only; it may not send directly, bypass preview or approval, target an arbitrary Discord message ID, or include an unauthorized raw destination."}
```

```forge-requirement
{"id":"REPLY-CON-02","kind":"constraint","text":"A valid reply draft references one stored inbound item and one immutable remote message ID; the target is revalidated immediately before delivery and a changed or deleted target blocks the send."}
```

---

## 4. Task Review Evidence

| Task | Outcome | Owner | Prerequisite | Outputs/checks | Acceptance | Exclusions |
|---|---|---|---|---|---|---|
| IN-STATE-1 | Transactional first/current inbound state and cursor | persistence-engineer | State and audit interfaces | `repo-com-inbox-state`, `inbox_state_contract` | IN-FR-05, INBOX-FR-01 through 03, state constraints | Discord reads, reply creation |
| IN-FETCH-1 | Bounded filtered fetch and edit/delete reconciliation | discord-engineer | Inbox state and Discord read client | `repo-com-inbox-fetch`, `inbox_fetch_contract` | IN-FR-01 through 05, inbound constraints | Local actions, reply drafts |
| IN-REPLY-1 | Authorized threaded reply draft and accepted linkage | messaging-engineer | Inbox state, draft, delivery | `repo-com-reply`, `reply_contract` | REPLY-FR-01 through 03 and reply constraints | Direct send, remote mutation |

---

## Phase 1: Durable Inbound State

```forge-task
{
  "id": "IN-STATE-1",
  "title": "Persist inbound snapshots and local lifecycle",
  "description": "Implement the focused repo-com-inbox-state crate for repository-scoped first snapshots, separate current snapshots or deleted markers, per-alias cursors, acknowledgement, archive, and reply links. Commit fetched items, edit or deletion events, and the advanced cursor in one transaction so a storage failure leaves the previous cursor authoritative. Make acknowledgement and archive idempotent local operations and preserve first snapshots across every later transition. Do not call Discord, interpret content, or create reply drafts.",
  "ownerAgent": "persistence-engineer",
  "dependencies": ["REPO-STATE-1", "REPO-AUDIT-1"],
  "expectedOutputs": [
    "crates/repo-com-inbox-state/Cargo.toml",
    "crates/repo-com-inbox-state/src/lib.rs",
    "crates/repo-com-inbox-state/src/item.rs",
    "crates/repo-com-inbox-state/src/store.rs",
    "crates/repo-com-inbox-state/src/cursor.rs",
    "crates/repo-com-inbox-state/tests/inbox_state_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(inbox_state_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/inbound-retrieval-and-reply.md#IN-FR-05",
      "docs/features/inbound-retrieval-and-reply.md#INBOX-FR-01",
      "docs/features/inbound-retrieval-and-reply.md#INBOX-FR-02",
      "docs/features/inbound-retrieval-and-reply.md#INBOX-FR-03"
    ],
    "acceptanceCriteria": [
      "Store tests preserve the first snapshot while recording changed current content, deleted state, acknowledgement, archive, and reply-link transitions separately",
      "Transaction tests inject failure during item persistence and prove no cursor advances and no partial page is visible",
      "Cursor tests prove per-alias, per-repository monotonic values and deterministic continuation after a retry",
      "Acknowledgement and archive tests are idempotent and expose no remote mutation operation",
      "Repository isolation tests reject cross-repository item, cursor, acknowledgement, archive, and reply-link access"
    ],
    "constraints": [
      "All inbound lifecycle changes remain local"
    ],
    "constraintRefs": [
      "docs/features/inbound-retrieval-and-reply.md#IN-CON-02",
      "docs/features/inbound-retrieval-and-reply.md#IN-CON-04",
      "docs/PRD.md#RC-SEC-08"
    ],
    "references": [
      "docs/PRD.md#13. System States / Lifecycle",
      "docs/features/inbound-retrieval-and-reply.md#2. Interfaces and Preconditions"
    ]
  }
}
```

## Phase 2: Bounded Fetch and Reply Drafting

```forge-task
{
  "id": "IN-FETCH-1",
  "title": "Fetch and reconcile bounded untrusted replies",
  "description": "Implement the focused repo-com-inbox-fetch crate for on-demand REST v10 reads from enabled inbound aliases. Require exactly one cursor or RFC 3339 boundary, paginate deterministically with 10-page and 1,000-message limits, retain only human replies to accepted local deliveries or direct bot mentions, ignore bots and repo-com messages, and return explicit untrusted envelopes. After storing a page transactionally, perform at most 100 read-only point checks for recently stored messages to record edits and deletions. Do not connect to Gateway, backfill arbitrary history, interpret content, or mutate Discord.",
  "ownerAgent": "discord-engineer",
  "dependencies": ["IN-STATE-1", "DISC-CLIENT-1", "DISC-DELIVERY-2"],
  "expectedOutputs": [
    "crates/repo-com-inbox-fetch/Cargo.toml",
    "crates/repo-com-inbox-fetch/src/lib.rs",
    "crates/repo-com-inbox-fetch/src/boundary.rs",
    "crates/repo-com-inbox-fetch/src/filter.rs",
    "crates/repo-com-inbox-fetch/src/fetch.rs",
    "crates/repo-com-inbox-fetch/src/reconcile.rs",
    "crates/repo-com-inbox-fetch/tests/inbox_fetch_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(inbox_fetch_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/inbound-retrieval-and-reply.md#IN-FR-01",
      "docs/features/inbound-retrieval-and-reply.md#IN-FR-02",
      "docs/features/inbound-retrieval-and-reply.md#IN-FR-03",
      "docs/features/inbound-retrieval-and-reply.md#IN-FR-04",
      "docs/features/inbound-retrieval-and-reply.md#IN-FR-05"
    ],
    "acceptanceCriteria": [
      "Fetch tests cover cursor and time alternatives, both/neither boundary rejection, deterministic ordering, continuation metadata, 10-page stop, and 1,000-message stop",
      "Filter tests retain a human reply to an accepted same-repository delivery and a human direct bot mention while excluding unrelated messages, other bots, repo-com messages, and cross-repository replies",
      "Envelope tests mark all content untrusted and include remote provenance, reply and mention evidence, and attachment indicators without attachment bytes",
      "Reconciliation tests record point-read edits and 404 deletions separately from first snapshots and stop after 100 probes with explicit continuation",
      "HTTP fixtures prove only configured channel GET requests and no Gateway, webhook, reaction, edit, or delete request"
    ],
    "constraints": [
      "Fetch is explicit, bounded, read-only, and untrusted"
    ],
    "constraintRefs": [
      "docs/features/inbound-retrieval-and-reply.md#IN-CON-01",
      "docs/features/inbound-retrieval-and-reply.md#IN-CON-03",
      "docs/features/inbound-retrieval-and-reply.md#IN-CON-04",
      "docs/PRD.md#RC-SEC-07"
    ],
    "references": [
      "docs/PRD.md#RC-US-04",
      "docs/PRD.md#5. Research Findings",
      "docs/features/inbound-retrieval-and-reply.md#2. Interfaces and Preconditions"
    ]
  }
}
```

```forge-task
{
  "id": "IN-REPLY-1",
  "title": "Create validated threaded reply drafts",
  "description": "Implement the focused repo-com-reply crate to validate one retained inbound item as a same-repository, same-workspace, configured-channel target and create one normal immutable draft revision with a Discord message_reference. Reject missing, deleted, expired, cross-repository, or unauthorized targets, revalidate the target before delivery eligibility, and link the inbound item to the reply draft. Mark replied only after the linked delivery is accepted. Do not send directly, bypass approval, or mutate the inbound message.",
  "ownerAgent": "messaging-engineer",
  "dependencies": ["IN-STATE-1", "DRAFT-MODEL-1", "DRAFT-ELIG-1", "DISC-DELIVERY-1"],
  "expectedOutputs": [
    "crates/repo-com-reply/Cargo.toml",
    "crates/repo-com-reply/src/lib.rs",
    "crates/repo-com-reply/src/target.rs",
    "crates/repo-com-reply/src/draft.rs",
    "crates/repo-com-reply/src/link.rs",
    "crates/repo-com-reply/tests/reply_contract.rs"
  ],
  "validationCommands": [
    "cargo fmt --all -- --check",
    "cargo clippy --workspace --all-targets --all-features -- -D warnings",
    "cargo nextest run --no-tests fail -E 'binary_id(reply_contract)'"
  ],
  "contract": {
    "version": 2,
    "kind": "implementation",
    "requirements": [],
    "requirementRefs": [
      "docs/features/inbound-retrieval-and-reply.md#REPLY-FR-01",
      "docs/features/inbound-retrieval-and-reply.md#REPLY-FR-02",
      "docs/features/inbound-retrieval-and-reply.md#REPLY-FR-03"
    ],
    "acceptanceCriteria": [
      "Target tests accept one retained authorized item and reject missing, deleted, expired, cross-repository, cross-workspace, unconfigured-channel, and arbitrary remote-ID targets",
      "Draft tests prove a valid target creates one immutable reply revision with exactly one message_reference and the normal draft lifecycle",
      "Eligibility tests prove a changed or deleted target blocks send before any delivery claim",
      "Link tests mark replied only after accepted delivery and preserve draft ID, remote message ID, accepted delivery ID, and audit event",
      "Command-service tests prove reply creation cannot call Discord or bypass approval, policy, safety, or idempotency gates"
    ],
    "constraints": [
      "Reply creation produces a draft, never a direct send"
    ],
    "constraintRefs": [
      "docs/features/inbound-retrieval-and-reply.md#REPLY-CON-01",
      "docs/features/inbound-retrieval-and-reply.md#REPLY-CON-02",
      "docs/PRD.md#RC-SEC-07",
      "docs/PRD.md#RC-SEC-08"
    ],
    "references": [
      "docs/PRD.md#6. Concept",
      "docs/features/inbound-retrieval-and-reply.md#2. Interfaces and Preconditions"
    ]
  }
}
```

---

## 5. Traceability

| Definition | Kind | Owning Tasks |
|---|---|---|
| [IN-FR-01](inbound-retrieval-and-reply.md#IN-FR-01) | requirement | IN-FETCH-1 |
| [IN-FR-02](inbound-retrieval-and-reply.md#IN-FR-02) | requirement | IN-FETCH-1 |
| [IN-FR-03](inbound-retrieval-and-reply.md#IN-FR-03) | requirement | IN-FETCH-1 |
| [IN-FR-04](inbound-retrieval-and-reply.md#IN-FR-04) | requirement | IN-FETCH-1 |
| [IN-FR-05](inbound-retrieval-and-reply.md#IN-FR-05) | requirement | IN-FETCH-1, IN-STATE-1 |
| [INBOX-FR-01](inbound-retrieval-and-reply.md#INBOX-FR-01) | requirement | IN-STATE-1 |
| [INBOX-FR-02](inbound-retrieval-and-reply.md#INBOX-FR-02) | requirement | IN-STATE-1 |
| [INBOX-FR-03](inbound-retrieval-and-reply.md#INBOX-FR-03) | requirement | IN-STATE-1 |
| [REPLY-FR-01](inbound-retrieval-and-reply.md#REPLY-FR-01) | requirement | IN-REPLY-1 |
| [REPLY-FR-02](inbound-retrieval-and-reply.md#REPLY-FR-02) | requirement | IN-REPLY-1 |
| [REPLY-FR-03](inbound-retrieval-and-reply.md#REPLY-FR-03) | requirement | IN-REPLY-1 |
| [IN-CON-01](inbound-retrieval-and-reply.md#IN-CON-01) | constraint | IN-FETCH-1, IN-REPLY-1 |
| [IN-CON-02](inbound-retrieval-and-reply.md#IN-CON-02) | constraint | IN-STATE-1 |
| [IN-CON-03](inbound-retrieval-and-reply.md#IN-CON-03) | constraint | IN-FETCH-1 |
| [IN-CON-04](inbound-retrieval-and-reply.md#IN-CON-04) | constraint | IN-FETCH-1, IN-STATE-1 |
| [REPLY-CON-01](inbound-retrieval-and-reply.md#REPLY-CON-01) | constraint | IN-REPLY-1 |
| [REPLY-CON-02](inbound-retrieval-and-reply.md#REPLY-CON-02) | constraint | IN-REPLY-1 |
