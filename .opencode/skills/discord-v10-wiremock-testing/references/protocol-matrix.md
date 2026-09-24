# Discord Protocol Matrix

> Load when: defining WireMock routes, request assertions, rate-limit fixtures, error classes, or bounded-read cases.

## Surface Matrix

| Surface | Allowed requests | Required assertions | Forbidden behavior |
|---|---|---|---|
| Setup | Selected read-only v10 identity, membership, channel, and permission reads | dedicated bot identity; required channel visibility and permissions | application, role, channel, or permission mutation |
| Message | Exactly one `POST /api/v10/channels/{channel_id}/messages` | exact final content, deterministic nonce, allowlisted mentions, optional validated reply reference | retry loop, attachments, embeds, raw workflow channel IDs |
| Inbound | Configured-channel reads with deterministic pagination and point checks | exactly one cursor/time boundary; 10 pages; 1,000 raw messages; 100 point checks | Gateway, daemon, arbitrary backfill, mutation, attachment bytes |
| Delivery | Message create called after a committed local claim | one request under concurrency; recorded state transition | network call before claim or unsafe second POST |
| Recovery | Read-only configured-destination reads | exact bot author, nonce, and final-content match | edit, delete, reaction, or automatic resend |
| E2E | Entire journey through the local server | request counts, token-free fixtures, no external network | real token, workspace, internet, or bypass path |

## Status and Transport Matrix

| Condition | Classification | Retry implication |
|---|---|---|
| 400 | terminal validation failure | none |
| 401 | terminal authentication failure | none |
| 403 | terminal permission failure | none |
| 404 | terminal not-found failure | none |
| 409 | terminal conflict | none |
| 429 | retry-wait with server-directed delay | bounded delivery retry may consider it |
| 5xx before proven dispatch | failed or retryable according to the task | bounded only |
| 5xx after possible dispatch | ambiguous or unknown | never automatic |
| Connect failure before dispatch | proven not sent | bounded retry may consider it |
| Timeout or reset after dispatch | ambiguous or unknown | never automatic |

Do not use a generic `failed` state for every non-2xx response. The adapter and delivery layers must preserve enough dispatch information to distinguish safe retry from duplicate risk.

## Dynamic Rate-Limit Cases

Cover both:

- route-scoped and global headers;
- user-scoped and shared 429 responses.

Use `Retry-After` or `retry_after`; do not hard-code a bucket reset timestamp. Cap a Discord-directed wait at 30 seconds per transport attempt and use an injected clock/sleeper in tests.

## Inbound Boundary Cases

Accept exactly one:

- explicit last-event-ID cursor;
- RFC 3339 boundary.

Reject both and neither. Reject a disabled alias, raw unconfigured channel ID, and more than 10 pages or 1,000 raw messages. Perform no more than 100 point checks after storing a page.

## Recovery Matching Cases

For outbound recovery, require all four predicates:

1. configured destination;
2. configured bot author;
3. exact deterministic nonce;
4. exact intended final content.

Test no match, one exact match, multiple matches, edited content, and deleted or tombstoned evidence. Conflicts remain blocked; do not silently convert edited, deleted, incomplete, or unreadable evidence into absence.

These are separate from the `reconcile-unknown-discord-delivery` skill, which owns the state decision and conservative five-minute plus three-successful-read absence boundary.
