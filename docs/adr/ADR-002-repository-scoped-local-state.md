# ADR-002: Repository-scoped local SQLite state under OS user-data

- **Status:** Accepted
- **Date:** 2026-09-24
- **Decision owners:** repo-com maintainers (canonical v1 plan)

## Context

Drafts, approvals, delivery attempts, inbound snapshots, cursors, and audit
evidence must survive process restarts for recovery, duplicate prevention, and
attribution. The product must remain single-user and local, with no shared
service or synchronized state.

## Decision

Store all operational state in one user-level SQLite database in the OS
application-data location (resolved with `dirs`). Namespace every record by the
configured `repository_id`, create the database with user-only filesystem
permissions, enable foreign keys, WAL, and a bounded busy timeout, and apply
forward-only migrations. On corruption, unsupported schema, lock timeout, or
migration failure, report a typed error without deleting or recreating the
database. Never upload, synchronize, or serve the database over a network, and
never store operational state beneath the repository.

## Alternatives Considered

- **Repository-local state file** — rejected: operational data would live inside
  the repository and risk accidental commit or sharing.
- **Shared multi-user service** — rejected: out of scope and conflicts with the
  single-user, local-first model.
- **Encrypted-at-rest store or OS keychain in v1** — rejected for v1: adds
  recovery complexity; user-only permissions are the v1 boundary with disclosed
  residual risk (see ADR-009).

## Consequences

- Benefits: durable, queryable, transactional state with repository isolation
  and no remote trust boundary.
- Costs and risks: local account access, backups, and filesystem snapshots can
  still read retained content; there is no cross-machine sync; migration
  mistakes are non-destructive but require forward fixes.

## Implementation References

- [repository-configuration-and-state.md](../features/repository-configuration-and-state.md)
  `STATE-FR-01` through `STATE-FR-03`, `STATE-CON-01`, `STATE-CON-02`
- [privacy-and-lifecycle-operations.md](../features/privacy-and-lifecycle-operations.md)
  `PRIV-CON-01`
- Planned outputs: `crates/repo-com-state/`, `migrations/0001_initial.sql`
- Owning tasks: `REPO-STATE-1`, `REL-INFRA-1`
