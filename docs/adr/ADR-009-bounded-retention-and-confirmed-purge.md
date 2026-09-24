# ADR-009: Bounded retention, confirmed purge, unencrypted local state

- **Status:** Accepted
- **Date:** 2026-09-24
- **Decision owners:** repo-com maintainers
- **Implementation status:** Partially implemented; retention settings are validated and stored, but retention sweeps and confirmed purge are not implemented.

## Context

Local state holds team communication content and delivery metadata. Retaining it
indefinitely increases privacy exposure, while deleting it carelessly could
destroy audit evidence or silently mutate Discord. v1 does not include
encryption at rest or a keychain.

## Decision

Retain draft and inbound message **content for 30 days** and non-content
delivery/audit **metadata for one year** by default, with validated per-repository
overrides (content 1–365 days; metadata 30–3,650 days; metadata at least content
retention). Run an opportunistic retention sweep before each state-mutating
invocation and on explicit request; a failed sweep blocks the new mutation with a
typed storage-integrity error. At content expiry, remove or irreversibly replace
text with a content-expired marker while preserving non-content IDs, hashes,
timestamps, lifecycle state, and audit evidence. Provide a **non-mutating** purge
plan with exact repository-scoped counts and a deterministic plan hash, and
execute only after an interactive TTY confirmation bound to the repository,
scope, cutoff, plan hash, and config hash. Purge and retention are local-only and
never mutate Discord. Send no telemetry. Keep state protected by user-only
filesystem permissions and document that this does not protect against local
account compromise, backups, or filesystem snapshots.

## Alternatives Considered

- **Encryption at rest / OS keychain in v1** — deferred to v2: adds recovery
  complexity; revisit after threat-model review.
- **No automatic retention** — rejected: increases unnecessary privacy exposure.
- **Remote deletion of sent messages** — rejected: outbound messages are
  immutable (see ADR-007).

## Consequences

- Benefits: bounded default exposure, explicit operator-confirmed destruction,
  and honest disclosure of residual risk.
- Costs and risks: users must understand that unencrypted local state is
  readable by anyone with account access; failed sweeps block mutations by
  design.

## Implementation References

- [privacy-and-lifecycle-operations.md](../features/privacy-and-lifecycle-operations.md)
  `RET-FR-01` through `RET-FR-05`, `RET-CON-01` through `RET-CON-03`,
  `PRIV-CON-01`, `PRIV-CON-02`
- [release-readiness.md](../features/release-readiness.md) `REL-CON-06`
- Planned outputs: `crates/repo-com-retention/`, `crates/repo-com-purge/`,
  `crates/repo-com-lifecycle/`
- Owning tasks: `PRIV-RET-1`, `PRIV-RET-2`, `PRIV-LIFE-1`
