# ADR-002: Repository-scoped local SQLite state under OS user-data

- **Status:** Accepted
- **Date:** 2026-09-24
- **Decision owners:** repo-com maintainers

## Context

Drafts, approvals, policy activations, delivery attempts, inbound snapshots,
and audit evidence need durable local state. The product is single-user and
local-first, so operational data must not be silently placed in the repository
or synchronized to a shared service.

## Decision

Store operational state in one SQLite database under the OS user-data root at
`repo-com/state.sqlite3`. Namespace every record by the configured
`repository_id`. Enable foreign keys, WAL, and a bounded busy timeout, apply
forward-only migrations, and protect the database with the platform's user-only
file boundary. The default path is outside the repository; explicit paths are
caller-controlled and must not be used for operational repository-local state.
On corruption, unsupported schema, lock timeout, or migration failure, return a
typed error without deleting or recreating the database.

The current state crate implements the path, migration, transaction, repository
scope, and immutable-evidence boundaries. It also exposes persistence APIs for
planned draft, delivery, inbound, and audit workflows; those persistence
primitives do not implement remote transport or higher-level lifecycle policy.

## Alternatives Considered

- **Repository-local state file** — rejected because operational data could be
  accidentally committed or shared with source history.
- **Shared multi-user service** — rejected as inconsistent with the local,
  single-user trust boundary.
- **Encrypted-at-rest store or OS keychain in v1** — deferred; the current
  implementation discloses that SQLite state is unencrypted and relies on file
  protections.

## Consequences

- Benefits: durable, queryable, repository-isolated state with a single local
  database and no remote trust boundary.
- Costs and risks: local account access, backups, and filesystem snapshots can
  read retained content; there is no cross-machine synchronization; migration
  mistakes require forward fixes.
- The embedding application must coordinate safe backups around SQLite WAL
  sidecars. No backup or repair command is provided by the current workspace.

## Implementation References

- [`crates/repo-com-state/src/paths.rs`](../../crates/repo-com-state/src/paths.rs)
- [`crates/repo-com-state/src/migrations.rs`](../../crates/repo-com-state/src/migrations.rs)
- [`crates/repo-com-state/src/store.rs`](../../crates/repo-com-state/src/store.rs)
- [`crates/repo-com-state/migrations/0001_initial.sql`](../../crates/repo-com-state/migrations/0001_initial.sql)
- [`crates/repo-com-state/tests/state_contract.rs`](../../crates/repo-com-state/tests/state_contract.rs)
- [Administrator Guide](../admin-guide.md)
- Planned retention and purge work: [`privacy-and-lifecycle-operations.md`](../features/privacy-and-lifecycle-operations.md)
