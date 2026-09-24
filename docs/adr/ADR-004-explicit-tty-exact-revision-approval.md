# ADR-004: Explicit TTY mode with exact-revision approval and fail-closed non-TTY

- **Status:** Accepted
- **Date:** 2026-09-24
- **Decision owners:** repo-com maintainers

## Context

Agent-originated content must not gain authority merely because it runs in a
non-interactive shell. Interactive decisions need an explicit boundary that is
testable and cannot be confused with an ambient terminal probe. Exact revision
approval is part of the future draft workflow.

## Decision

Represent TTY state explicitly as `TtyMode::Tty` or `TtyMode::NonTty` in the
foundation. Domain code must not inspect stdin or stdout itself. A non-TTY
prompt request fails with `operator-action-required`, and a non-TTY caller
cannot create approval, activate policy, or override a safety finding.

For the future draft workflow, human approval binds to one immutable revision
containing its text, metadata, destination, repository scope, and expiry. Any
relevant change invalidates the approval. The current policy registry already
uses this fail-closed boundary for authority-creating activation and accepts
only a caller-supplied TTY confirmation.

## Alternatives Considered

- **Implicit `isatty` probing in domain code** — rejected because ambient
  terminal state is difficult to test and control.
- **Non-TTY approval via a flag** — rejected because it enables automation to
  self-approve.
- **Always require per-message approval** — retained as the future default for
  messages not covered by an active exact policy.

## Consequences

- Benefits: automation cannot accidentally become an approval authority, and
  the current activation boundary is deterministic and testable.
- Costs and risks: the future operator workflow requires an interactive caller;
  exact approval and terminal presentation are not implemented yet.
- A library consumer must collect operator confirmation and pass the explicit
  mode; the policy crate does not prompt or probe a terminal.

## Implementation References

- [`crates/repo-com-foundation/src/args.rs`](../../crates/repo-com-foundation/src/args.rs)
  (`TtyMode` and `require_prompt_allowed`)
- [`crates/repo-com-foundation/tests/foundation_contract.rs`](../../crates/repo-com-foundation/tests/foundation_contract.rs)
- [`crates/repo-com-policy/src/activation.rs`](../../crates/repo-com-policy/src/activation.rs)
  (`OperatorConfirmation` and `PolicyRegistry::activate`)
- [`crates/repo-com-policy/tests/policy_contract.rs`](../../crates/repo-com-policy/tests/policy_contract.rs)
- Planned exact approval and terminal flow:
  [`draft-and-approval-workflow.md`](../features/draft-and-approval-workflow.md)
  and [`release-readiness.md`](../features/release-readiness.md)
