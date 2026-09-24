# ADR-005: Operator-activated exact-tuple auto-send policy

- **Status:** Accepted
- **Date:** 2026-09-24
- **Decision owners:** repo-com maintainers

## Context

A repository may want a narrow class of notifications to proceed without a new
human approval for every message. Broad matching would let a skill widen its
own authority, and an edited configuration must not silently inherit old
permission.

## Decision

Match only an exact tuple of event type, destination alias, and severity.
Reject wildcards, prefixes, broader severity labels, duplicate configuration
entries, and ambiguous active records. Bind an activation to the SHA-256 hash
of the complete normalized configuration and the hash of the exact tuple.
Changing either hash makes the activation stale.

Activation requires an explicit TTY confirmation supplied by the caller.
Read-only status and permission-reducing deactivation are available without a
TTY. A policy decision is only one gate; the future send path must separately
revalidate approval, destination, revision, safety, and delivery state.

The current policy crate implements exact matching, canonical hashes, previews,
TTY-confirmed activation, status classification, stale detection, ambiguity
denial, idempotent replay, and deactivation. It does not send messages.

## Alternatives Considered

- **Wildcard, prefix, or severity-threshold matching** — rejected because it
  expands authority beyond the reviewed tuple.
- **Environment-variable or config-file activation** — rejected because a
  skill or edited file could create authority without an operator action.
- **Always require per-message approval** — retained as the future default for
  messages without a current exact activation.

## Consequences

- Benefits: a skill cannot widen policy authority; current eligibility is tied
  to hashes and an explicit operator action.
- Costs and risks: an operator must reactivate after relevant configuration
  changes, and callers must display stale or ambiguous status clearly.
- Policy eligibility must not be described as proof that a message was sent or
  that final approval and delivery checks passed.

## Implementation References

- [`crates/repo-com-policy/src/hash.rs`](../../crates/repo-com-policy/src/hash.rs)
- [`crates/repo-com-policy/src/evaluate.rs`](../../crates/repo-com-policy/src/evaluate.rs)
- [`crates/repo-com-policy/src/activation.rs`](../../crates/repo-com-policy/src/activation.rs)
- [`crates/repo-com-policy/tests/policy_contract.rs`](../../crates/repo-com-policy/tests/policy_contract.rs)
- [`crates/repo-com-state/src/store.rs`](../../crates/repo-com-state/src/store.rs)
  (policy activation persistence)
- [Library Consumer Guide](../user-guide.md)
- Future delivery gates: [`discord-delivery-and-reconciliation.md`](../features/discord-delivery-and-reconciliation.md)
