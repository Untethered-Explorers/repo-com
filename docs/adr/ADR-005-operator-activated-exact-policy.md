# ADR-005: Operator-activated exact-tuple auto-send policy

- **Status:** Accepted
- **Date:** 2026-09-24
- **Decision owners:** repo-com maintainers (canonical v1 plan)

## Context

Some pre-approved notification classes should send without per-message human
approval, but a skill must never be able to widen its own permissions. Broad
matching rules would let a skill send content the operator never authorized.

## Decision

Permit automatic sending only when one committed policy entry **exactly** equals
the draft's event type, destination alias, and severity. A wildcard, prefix, or
broader severity match is ineligible. Activate a policy only through an
interactive TTY operator confirmation that records the canonical configuration
hash and the policy-tuple hash in user state. Any relevant config or tuple change
marks the activation stale automatically, and non-TTY callers may inspect status
or deactivate but may never activate.

## Alternatives Considered

- **Wildcard, prefix, or severity-threshold matching** — rejected: silently
  expands authority beyond the reviewed intent.
- **Environment-variable or config-file activation** — rejected: a skill or
  edited config could enable sending without an operator action.
- **Always require per-message approval** — retained as the default for all
  non-policy messages.

## Consequences

- Benefits: a skill cannot widen its own send authority; authority is bound to
  an exact, hashed tuple that the operator saw.
- Costs and risks: operators must reactivate policy after any relevant config
  edit; status output must clearly signal stale activations.

## Implementation References

- [repository-configuration-and-state.md](../features/repository-configuration-and-state.md)
  `POLICY-FR-01` through `POLICY-FR-03`, `POLICY-CON-01`
- [PRD §10 Security and Privacy](../PRD.md#10-security-and-privacy) `RC-SEC-04`
- Planned outputs: `crates/repo-com-policy/`
- Owning task: `REPO-POLICY-1`
