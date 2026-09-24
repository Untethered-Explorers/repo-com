# ADR-008: Untrusted inbound data with local-only lifecycle

- **Status:** Accepted
- **Date:** 2026-09-24
- **Decision owners:** repo-com maintainers
- **Implementation status:** Partially implemented; inbound snapshot and lifecycle persistence exist, but Discord fetch and reply transport do not.

## Context

Replies and mentions arrive from Discord, which is outside the trust boundary.
If inbound text could influence permissions or trigger sends, a teammate (or an
attacker with channel access) could indirectly control the agent. The product
also must not imply read receipts or mutate remote history.

## Decision

Treat every inbound message, mention, edit, and deletion as **untrusted data**
that cannot approve a draft, activate policy, override safety, or trigger a
send. Fetch only enabled inbound aliases from an explicit last-event-ID cursor
or RFC 3339 boundary, bounded to 10 pages / 1,000 raw messages, and retain only
human messages that reply to an accepted local delivery or directly mention the
configured bot; ignore other bots and `repo-com`'s own messages. Store the first
remote snapshot and the current snapshot or deleted marker separately, and
advance a cursor only in the same transaction that stores the page. Make
acknowledgement and archival local-only, and create replies as normal immutable
drafts carrying a validated `message_reference` that pass the same approval,
policy, safety, and delivery gates. Mark an item replied only after the linked
delivery is accepted.

## Alternatives Considered

- **Gateway monitor / daemon** — rejected for v1: out of scope and increases the
  attack surface.
- **Treat inbound as instructions** — rejected: violates the trust boundary.
- **Remote reactions or edits for acknowledgement** — rejected: inbound
  lifecycle must not mutate Discord.

## Consequences

- Benefits: inbound cannot escalate authority; correlation and audit remain
  local and durable; no background service is required.
- Costs and risks: fetching requires an explicit cursor or time boundary; edited
  or deleted remote messages leave the original local snapshot intact by design.

## Implementation References

- [inbound-retrieval-and-reply.md](../features/inbound-retrieval-and-reply.md)
  `IN-FR-01` through `IN-FR-05`, `INBOX-FR-01` through `INBOX-FR-03`,
  `REPLY-FR-01` through `REPLY-FR-03`, `IN-CON-01` through `IN-CON-04`
- [PRD §10 Security and Privacy](../PRD.md#10-security-and-privacy) `RC-SEC-07`,
  `RC-SEC-08`
- Planned outputs: `crates/repo-com-inbox-state/`,
  `crates/repo-com-inbox-fetch/`, `crates/repo-com-reply/`
- Owning tasks: `IN-STATE-1`, `IN-FETCH-1`, `IN-REPLY-1`
