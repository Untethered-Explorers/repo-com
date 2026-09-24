# ADR-004: Explicit TTY mode with exact-revision approval and fail-closed non-TTY

- **Status:** Accepted
- **Date:** 2026-09-24
- **Decision owners:** repo-com maintainers (canonical v1 plan)

## Context

Agent-originated content must not be sent without human review by default, but
repository skills run in non-interactive shells where prompting would hang or be
unsafe. Approval must bind to the exact reviewed content and must not become a
reusable bypass.

## Decision

Expose TTY state as an explicit enum with no hidden terminal probing inside
domain code. Interactive approval, secret-scan override, and policy activation
are operator-only TTY actions. Human approval is bound to one immutable draft
revision hash containing text, metadata, destination alias, resolved
destination, repository ID, and expiry; approval expires at the earlier of draft
expiry or 15 minutes and is invalidated by any relevant change. A non-TTY
invocation may send only when an existing exact approval or activated policy
already satisfies eligibility; it may not create approval, activate policy, or
override a safety finding, and instead returns an operator-action-required
outcome.

## Alternatives Considered

- **Implicit `isatty` probing in domain logic** — rejected: nondeterministic and
  hard to test.
- **Non-TTY approval via flag** — rejected: weakens exact-revision consent and
  enables automation to self-approve.
- **No approval, policy always required** — rejected: too restrictive for
  interactive operator use.

## Consequences

- Benefits: consent is explicit, reviewable, and time-bounded; automation fails
  closed rather than hanging or bypassing review.
- Costs and risks: operators must run approval interactively; the 15-minute
  window adds friction that the human UX review must validate.

## Implementation References

- [cli-foundation.md](../features/cli-foundation.md) `FOUND-FR-04`
- [draft-and-approval-workflow.md](../features/draft-and-approval-workflow.md)
  `APPROVAL-FR-01` through `APPROVAL-FR-03`, `ELIG-FR-03`, `APPROVAL-CON-01`
- [repository-configuration-and-state.md](../features/repository-configuration-and-state.md)
  `POLICY-CON-01`
- Planned outputs: `crates/repo-com-approval/`,
  `crates/repo-com-send-eligibility/`, `crates/repo-com-terminal-outbound/`
- Owning tasks: `PLAT-1`, `DRAFT-APPROVAL-1`, `DRAFT-ELIG-1`, `REL-UI-OUT-1`
