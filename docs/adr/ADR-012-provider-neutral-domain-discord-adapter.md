# ADR-012: Provider-neutral domain with a Discord-only v1 adapter

- **Status:** Accepted
- **Date:** 2026-09-24
- **Decision owners:** repo-com maintainers (canonical v1 plan)

## Context

Email and other notification providers are plausible future transports. Coupling
the draft, delivery, and inbox domain model to Discord would make a later
provider a rewrite. Shipping several providers in v1 would dilute focus and
expand the safety surface.

## Decision

Keep the draft, delivery, and inbox domain interfaces provider-neutral, and ship
only a Discord adapter in v1. Discord-specific concepts (channel IDs,
`allowed_mentions`, `message_reference`, nonce footers) live in the adapter and
content-rendering layers, not in the domain state machine. Design a second
provider only after the Discord workflow validates the shared model.

## Alternatives Considered

- **Discord-coupled domain model** — rejected: blocks future providers and mixes
  transport concerns into safety logic.
- **Multiple providers in v1** — rejected: premature; each provider has distinct
  authentication, threading, and inbound semantics.
- **Email first** — rejected: Discord offers lower-friction delivery for the
  primary success signal (a teammate replying).

## Consequences

- Benefits: focused v1, a cleaner safety boundary, and a path to v2 providers.
- Costs and risks: provider-neutral abstractions may need revision when the first
  alternate adapter is built; the abstraction must not leak Discord assumptions.

## Implementation References

- [PRD §7.3 Key Interfaces](../PRD.md#73-key-interfaces) `RC-FR-04`;
  [§19 Future Considerations](../PRD.md#19-future-considerations)
- [IDEA.md](../IDEA.md) message boundaries and v1 scope
- Planned outputs: provider-neutral crates such as
  `repo-com-draft-model/`, `repo-com-delivery/`, `repo-com-inbox-state/`, with
  Discord adapters in `repo-com-discord-*` crates
