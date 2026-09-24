# Transaction Map

> Load when: selecting the state branch, transaction unit, rollback rule, or contract-test selector.

## Branch Matrix

| Branch | Atomic unit | Required uniqueness or preservation | Rollback result | Test target |
|---|---|---|---|---|
| Base schema | version-1 migration and store initialization | apply once; safe reopen; foreign keys, WAL, bounded busy timeout | typed failure; preserve corrupt or unsupported database | `state_contract` |
| Audit-coupled mutation | current-state revalidation, mutation, redacted audit append | audit append-only; no update or delete path | all local writes and audit event roll back | owning branch target |
| Delivery claim | authorization revalidation, unique attempt, audit event | one claim for the exact revision; terminal states cannot be reclaimed | no claim, attempt, or audit event after local failure | `delivery_contract` |
| Inbound page | all stored items, edit/delete events, cursor advancement | first snapshot immutable; per-repository cursor | previous cursor remains; no partial page visible | `inbox_state_contract` |
| Retention | cutoff validation and complete sweep | preserve IDs, hashes, timestamps, links, lifecycle state, audit | attempted state mutation is blocked | `retention_contract` |
| Purge | exact-scope deletion and count-only audit | deterministic plan hash and exact category/row counts | rollback and mandatory replanning | `purge_contract` |

## Common Transaction Envelope

1. Resolve the configured repository before mutation.
2. Begin the narrowest transaction that covers the complete atomic unit.
3. Revalidate every task-specific current-state precondition.
4. Perform repository-scoped state writes.
5. Append the matching redacted audit event when the contract requires it.
6. Commit once.
7. On any local failure, roll back and return a typed non-destructive result.

Do not include Discord or other network I/O in a SQLite transaction. A delivery claim commits before the separately owned message operation starts.

## Repository Isolation Checks

For every state branch, test at least:

- the same object identifier in two repositories;
- a foreign cursor, attempt, purge plan, approval, or audit record;
- a missing repository alias or ambiguous repository resolution;
- a row count and audit count that remain identical after rollback.

## Integrity and Migration Checks

- The base database lives in the user-level application-data location, never beneath the repository.
- The version-1 migration is forward-only and applied exactly once.
- Corrupt, unsupported, locked, and migration-failure cases preserve the original database.
- The task contract does not define a numeric busy timeout, isolation level, lock backoff, or purge batch size. Choose an implementation value explicitly, test it, and do not present it as a product requirement.

## Human-Review Tasks

The human-review records for release UX, security, live Discord, and sign-off are outside this transaction model. No agent may create scores, attestations, or sign-off evidence.
