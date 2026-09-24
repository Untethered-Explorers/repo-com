# ADR-006: Dedicated Discord bot, REST v10, read-only setup

- **Status:** Accepted
- **Date:** 2026-09-24
- **Decision owners:** repo-com maintainers (canonical v1 plan)

## Context

`repo-com` sends and reads Discord messages on the operator's behalf. Using a
user token or self-bot impersonates the operator, violates least privilege, and
risks account action. Discord's REST API changes and rate-limits dynamically, so
hard-coded behavior is fragile.

## Decision

Authenticate only as a dedicated Discord **bot**, reading the token solely from
the `REPO_COM_DISCORD_TOKEN` environment variable and sending it only in the Bot
authorization scheme. Reject user tokens, Bearer user authentication, self-bots,
and unpinned API versions. Pin every request to `/api/v10`. Provide a guided,
**read-only** setup check that validates bot identity, workspace membership,
channel visibility, `VIEW_CHANNEL`, `SEND_MESSAGES`, `READ_MESSAGE_HISTORY`, and
resolved mention access, and never creates applications or mutates permissions.
Read rate limits from response headers and `Retry-After` rather than hard-coding
buckets. Redact the token and response bodies from errors and diagnostics.

## Alternatives Considered

- **User token or self-bot** — rejected: impersonation and policy violation.
- **Automatic application/permission provisioning** — rejected: setup must not
  mutate the operator's server.
- **Gateway websocket client** — rejected for v1: a daemon-like connection is out
  of scope; on-demand REST is sufficient.

## Consequences

- Benefits: least privilege, clear attribution to a bot identity, and resilience
  to dynamic rate limits.
- Costs and risks: an operator/administrator must create the bot and grant
  permissions manually; missing permissions surface as typed remediation rather
  than automatic fixes.

## Implementation References

- [discord-delivery-and-reconciliation.md](../features/discord-delivery-and-reconciliation.md)
  `DISC-FR-01`, `DISC-FR-02`, `DISC-CON-01`, `DISC-CON-02`
- [PRD §10 Security and Privacy](../PRD.md#10-security-and-privacy) `RC-SEC-01`,
  `RC-SEC-02`; [PRD §7.3](../PRD.md#73-key-interfaces)
- Planned outputs: `crates/repo-com-discord-client/`
- Owning task: `DISC-CLIENT-1`
