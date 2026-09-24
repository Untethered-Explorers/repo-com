# Topic and Claim Matrix

> Load when: assigning documentation ownership, checking required topics, validating residual risk, or detecting prohibited positive claims.

## Topic Ownership

| Topic | Owning document | Authoritative source |
|---|---|---|
| Schema version 1, repository identity, aliases, exact policies, retention, rejected fields | `docs/configuration.md` | configuration model, resolver, validator, and `REPO-CFG-1` contract |
| Dedicated bot creation, least-privilege grants, channel and mention checks, token environment variable, rotation | `docs/discord-setup.md` | Discord setup adapter, current official Discord guidance, `DISC-CLIENT-1` contract |
| Installation, commands, flags, protocol, preview, approval, policy, outcomes, recovery, fetch and reply, audit, retention, purge, accessibility | `docs/operator-guide.md` | actual handler, renderer, service, and E2E contracts |
| Trust boundaries, bot-only auth, untrusted input, exact authorization, duplicate prevention, redaction, local state, recovery | `docs/security-model.md` | security and delivery contracts plus implemented behavior |
| Threat surfaces, trust assumptions, abuse cases, unsupported protections, residual risk | `docs/threat-model.md` | security model, privacy and lifecycle contracts, observed limitations |

Every required topic must have one owning document. Other documents may link to it, but must not contradict it.

## Required Residual-Risk Disclosures

State affirmatively in the security and threat documentation:

- no encryption at rest;
- user-only filesystem permissions, with documented platform behavior;
- no telemetry;
- exposure through local-account compromise;
- exposure through backups;
- exposure through filesystem snapshots.

A validator must distinguish `no encryption at rest` from a positive encryption guarantee.

## Alignment Checks

Verify:

- actual command tree and flags;
- no default repository, destination, revision, cursor, boundary, or inbound item;
- protocol version 1 and exactly one machine object;
- stable process categories;
- TTY and non-TTY behavior;
- terminal labels, state names, 80-column and no-color behavior;
- local versus last-fetched remote state;
- unknown-delivery reconciliation and separate new-attempt authorization;
- retention and purge planning versus execution;
- supported Discord REST v10 and dedicated bot authentication.

Do not document a flag, default, state, or installer name that is not implemented or contractually owned.

## Prohibited Positive Claims

Reject claims that repo-com:

- proves a teammate read, viewed, or replied to a message;
- provides response analytics or read receipts;
- allows arbitrary destinations or user tokens;
- is compatible with live Discord based only on Wiremock;
- has human approval, release sign-off, or compliance certification;
- encrypts state at rest or collects telemetry;
- knows current remote truth beyond a timestamped fetch;
- safely resends an unknown delivery automatically;
- treats a stored inbound snapshot as current content;
- completed a command, purge, approval, or send that was not observed.

Negative statements that deny these claims are required where relevant and must not be rejected by a global keyword ban.

## Secret and Example Checks

Use environment-variable names and placeholders, never realistic values. Reject bot tokens, private keys, authorization headers, live workspace IDs when sensitive, and real team-message content. External examples must be synthetic and clearly non-production.

## Evidence Boundary

`documentation_contract` proves deterministic topic, alignment, polarity, and secret-pattern checks. It does not prove human comprehension, security approval, live Discord behavior, or final release sign-off.
