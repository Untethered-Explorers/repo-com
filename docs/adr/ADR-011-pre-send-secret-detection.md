# ADR-011: Pre-send secret detection with audited TTY override

- **Status:** Accepted
- **Date:** 2026-09-24
- **Decision owners:** repo-com maintainers (canonical v1 plan)

## Context

Agent-originated text can accidentally include credentials. A purely
preventive blocker risks false positives that halt legitimate work, while no
checking risks leaking secrets to a channel. Perfect detection is not
achievable, so the control must be honestly scoped.

## Decision

Scan the final rendered text and metadata for high-confidence Discord bot
tokens, authorization values, private-key markers, credential-bearing URLs, and
common secret assignments before a send can become eligible. A finding blocks
send by default. Only an interactive TTY operator may override the **exact
reviewed revision** after seeing the full preview; the override is audited with a
redacted reason code and never records the matched value, and non-TTY override is
forbidden. Treat the scanner as defense in depth, never as complete data-loss
prevention, and never persist findings or the matched values.

## Alternatives Considered

- **No scanning** — rejected: misses an obvious, high-impact accident class.
- **Hard block with no override** — rejected: false positives would block
  legitimate sends with no safe escape.
- **External secret-scanning service** — rejected for v1: sends content to a
  third party and adds a network dependency.

## Consequences

- Benefits: a cheap, deterministic safety net with an explicit audited escape
  hatch.
- Costs and risks: false negatives and false positives remain possible; the
  product must not claim complete DLP, and every override must be visible in the
  audit trail.

## Implementation References

- [draft-and-approval-workflow.md](../features/draft-and-approval-workflow.md)
  `DRAFT-FR-05`, `SAFETY-CON-01`
- [PRD §10 Security and Privacy](../PRD.md#10-security-and-privacy) `RC-SEC-09`
- Planned outputs: `crates/repo-com-draft-safety/`, `crates/repo-com-approval/`
- Owning tasks: `DRAFT-SECRET-1`, `DRAFT-APPROVAL-1`
