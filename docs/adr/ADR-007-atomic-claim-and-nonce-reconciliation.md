# ADR-007: Atomic duplicate-safe claim and nonce reconciliation

- **Status:** Accepted
- **Date:** 2026-09-24
- **Decision owners:** repo-com maintainers
- **Implementation status:** Partially implemented; state persistence and atomic claim primitives exist, but Discord delivery and reconciliation do not.

## Context

A crash, retry, or concurrent invocation must never create a second remote
message for the same draft revision. Discord requests and events are not
strongly consistent, so the client can lose a response after Discord accepted a
message. Transport-level idempotency keys are unavailable for this flow.

## Decision

Claim a send in one local SQLite transaction that reloads current config and
repository hashes, revision hash and expiry, resolved destination and mention
allowlist, exact approval or activated policy, and secret-scan decision, then
creates one uniquely constrained attempt and its audit event and commits
**before** any network I/O. Concurrent or repeated callers receive the recorded
outcome instead of a second POST. Render a deterministic delivery nonce in the
message footer. Classify a post-dispatch timeout, reset, or 5xx as `unknown`;
reconcile by reading only the configured destination and matching the bot
author, nonce, and exact content. Retry at most three total transport attempts,
only for proven pre-dispatch failures or HTTP 429, with the server-directed
delay capped at 30 seconds. Never automatically resend an unknown or unresolved
outcome.

## Alternatives Considered

- **Best-effort send with client-generated idempotency key** — Discord does not
  provide exactly-once semantics for this path; rejected as insufficient.
- **Unlimited retries on any error** — rejected: risks duplicates and hammers
  the API.
- **No reconciliation, resend on operator request alone** — rejected: could
  duplicate a message Discord actually accepted.

## Consequences

- Benefits: crash- and concurrency-safe delivery with a conservative, auditable
  recovery path.
- Costs and risks: an ambiguous outcome can remain blocked until reconciliation
  finds matching evidence; absence requires a five-minute window and three
  successful reads before `reconciled_absent`.

## Implementation References

- [discord-delivery-and-reconciliation.md](../features/discord-delivery-and-reconciliation.md)
  `DEL-FR-01` through `DEL-FR-07`, `DEL-CON-01` through `DEL-CON-04`
- [PRD §13 System States / Lifecycle](../PRD.md) (section 13);
  `RC-NFR-02`, `RC-NFR-05`
- Planned outputs: `crates/repo-com-delivery/`,
  `crates/repo-com-delivery-retry/`
- Owning tasks: `DISC-DELIVERY-1`, `DISC-DELIVERY-2`
