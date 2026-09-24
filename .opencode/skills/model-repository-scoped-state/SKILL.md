---
name: model-repository-scoped-state
description: "Implement or change repo-com SQLite state with repository isolation, explicit transaction boundaries, state-and-audit atomicity, rollback, uniqueness, and no-remote side effects; use for schema, delivery claims, inbound cursors, retention sweeps, or confirmed purge execution."
---

# Model Repository-Scoped State

Design every local mutation as one repository-bound transaction with a testable rollback boundary. Use this skill for base state, delivery claims, inbound cursor pages, retention, and purge; it does not define TTY confirmation, Discord retry, or remote reads.

Load the [transaction map](./references/transaction-map.md) when selecting the operation branch or defining its atomic unit.

## Process

### Step 1: Resolve the exact state operation

Read the owning task contract and classify the branch: base schema, delivery claim, inbound page, retention sweep, or purge execution. Record the repository identity, object identifiers, expected outputs, transaction requirements, audit coupling, and explicit no-remote rule.

If the request is read-only, then do not open a mutation transaction or create an audit transition.

### Step 2: Bind every row and query to one repository

Require the configured `repository_id` on each repository API and include it in every relevant uniqueness, lookup, update, cursor, and audit query. Treat an object ID without matching repository scope as unauthorized.

Use canonical repository resolution before opening the transaction. If aliases or repository identity are missing or ambiguous, then return the owning operation's typed validation error.

### Step 3: Define the atomic unit

Write down what must commit together before writing SQL:

- schema version and migration state;
- current precondition revalidation plus state mutation plus audit event;
- one complete inbound page, its lifecycle events, and cursor advancement;
- delivery authorization, uniqueness, attempt creation, and audit event;
- retention cutoff and all sweep changes;
- purge plan validation, exact-scope deletion, and count-only audit event.

If a required write cannot join the unit, then redesign the boundary rather than accepting partial state.

### Step 4: Revalidate inside the transaction

Reload the current repository, configuration, revision, destination, policy, approval, cutoff, or deterministic plan as required by the task. Recheck uniqueness and terminal-state rules before mutation. Do not trust a decision calculated before the transaction when current state can change.

For delivery, commit the claim and audit before any Discord request. If a later network result is unknown, then do not undo the claim into a retryable pre-send state.

### Step 5: Commit or roll back as a unit

Use one explicit transaction for each branch and commit only after all local writes and audit appends succeed. On any failure, roll back the entire unit and leave the previous state authoritative.

For retention, a failed pre-mutation sweep blocks the attempted new mutation. For purge, failed execution requires a new plan. For inbound storage, a failed page leaves the previous cursor and exposes no partial page.

### Step 6: Fail non-destructively

Enable foreign keys, WAL, and a bounded busy timeout according to the foundation contract, but do not treat those settings as substitutes for transactions or uniqueness constraints. On corruption, an unsupported schema, migration failure, or exhausted busy timeout, return a typed integrity result and preserve the database bytes.

Never delete, repair, recreate, or silently upgrade a corrupt or unsupported store.

## Gotchas

- **WAL mistaken for atomicity.** WAL and a busy timeout improve concurrency; they do not enforce a transaction boundary, repository filter, or unique claim.
- **Cross-repository lookup by object ID.** Every query must bind repository scope even when the row ID appears globally unique.
- **Cursor advanced without its page.** Commit stored items, edit/delete transitions, and cursor advancement together or retain the old cursor.
- **Delivery claim rolled back into permission.** SQLite cannot atomically include Discord. Claim first, POST later, and treat post-dispatch ambiguity as a blocker rather than an automatic retry.
- **Retention deleting evidence.** Content expiry may remove text while preserving revision identity, hashes, timestamps, links, lifecycle state, and audit records.
- **Purge plan treated as authority.** A plan is non-mutating evidence. Execution must match repository, scope, cutoff, configuration hash, and deterministic plan hash.

## Validation

Self-check the state boundary before handoff:

- [ ] Repository isolation is proven with same-ID and cross-repository negative cases.
- [ ] The declared transaction contains every state and audit write that must succeed or fail together.
- [ ] Failure injection proves rollback and leaves the prior cursor, claim, revision, or plan authoritative.
- [ ] Concurrency or uniqueness tests prove the intended single winner without lost committed state.
- [ ] Corrupt, unsupported, locked, and migration-failure cases return typed errors and preserve bytes.
- [ ] No test exposes a Discord mutation, remote deletion, telemetry path, or network dependency in a local-only operation.

Run the exact branch contract from the task map. For example:

```bash
cargo nextest run --no-tests fail -E 'binary_id(state_contract)'
cargo nextest run --no-tests fail -E 'binary_id(delivery_contract)'
cargo nextest run --no-tests fail -E 'binary_id(inbox_state_contract)'
cargo nextest run --no-tests fail -E 'binary_id(retention_contract)'
cargo nextest run --no-tests fail -E 'binary_id(purge_contract)'
```

If a rollback or isolation test fails, then fix the transaction boundary before broadening the selector. If the selected test reports zero tests, then fix the target or selector before accepting the result.
