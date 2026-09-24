---
name: validate-canonical-product-documentation
description: "Keep repo-com configuration, Discord setup, operator, security, threat-model, command, protocol, accessibility, and residual-risk documentation aligned with implemented contracts and free of prohibited claims; use when authoring or changing REL-DOC-1 outputs and documentation validators."
---

# Validate Canonical Product Documentation

Keep the five downstream product documents synchronized with implemented repo-com behavior and explicit residual risk. This skill covers `docs/configuration.md`, `docs/discord-setup.md`, `docs/operator-guide.md`, `docs/security-model.md`, and `docs/threat-model.md`; it does not edit the PRD, feature requirements, agent ownership, or human-review records.

Load the [topic and claim matrix](./references/topic-and-claim-matrix.md) before drafting prose, adding examples, or writing claim-polarity checks.

## Process

### Step 1: Read the documentation contract and implemented interfaces

Read the exact `REL-DOC-1` acceptance criteria and collect facts from the implemented command handlers, protocol, renderer crates, state and lifecycle services, configuration schema, Discord adapter, and release policy. Prefer observed interface and test behavior over planned prose or historical source documents.

If an implementation prerequisite is absent, then document the gap and stop short of claiming the product document is complete.

### Step 2: Build a topic ownership matrix

Assign every required topic to one owning document and record the authoritative source. Cover configuration schema and aliases, Discord setup and least privilege, command and protocol behavior, terminal accessibility, operator recovery, privacy retention and purge, audit, security boundaries, threat surfaces, and residual risks.

If a topic has no authoritative source or the documents disagree, then surface the mismatch before writing a definitive statement.

### Step 3: Refresh external facts with attribution

Consult current official Discord, Rust, operating-system permission, and security guidance when needed. Distinguish external guidance from repo-com implementation guarantees. Never turn a third-party recommendation, planning version, or current platform behavior into an unverified product promise.

### Step 4: Draft operator procedures separately from analysis

Write the operator guide as task-oriented commands, inputs, outputs, confirmations, outcomes, recovery, and accessibility guidance. Write security and threat documents around trust boundaries, actors, assets, abuse cases, mitigations, unsupported protections, and residual risk.

Use synthetic examples only. Never copy a real token, private key, authorization value, team-message content, or credential-bearing environment output.

### Step 5: Validate claim polarity and alignment

Check that command names, flags, defaults, identifiers, protocol version, exit categories, state labels, limits, TTY behavior, and accessibility behavior match implementation. Validate that security and threat documents explicitly disclose no encryption at rest, user-only permissions, no telemetry, and exposure through local-account compromise, backups, and filesystem snapshots.

Reject positive claims of read receipts, response analytics, arbitrary destinations, user-token support, live compatibility, human approval, release sign-off, compliance certification, encryption, telemetry, current remote truth, or automatic resend of unknown delivery.

Do not globally ban words such as `telemetry` or `encryption at rest`; required negative disclosures must be allowed. Validate the polarity and context of each claim.

### Step 6: Run the documentation contract and review evidence

Run the exact `documentation_contract` and inspect every finding. Correct failures, then report the actual result and scope. Keep automated validation separate from human UX, security, live Discord, and release sign-off records.

## Gotchas

- **Naive keyword ban rejects required disclosure.** Security docs must be able to say `no telemetry` and `no encryption at rest`.
- **Installation prose invents packaging behavior.** Until packaging policy is authoritative, document only the implemented installation contract and mark missing artifact names as unresolved.
- **Current-looking text is not implemented truth.** Every command, default, label, limit, and exit category must align with the actual code or contract.
- **External guidance becomes a guarantee.** Attribute current Discord, Rust, and OS facts and keep them separate from repo-com behavior.
- **Realistic secret fixtures committed.** Use synthetic or structurally invalid values and scan the final files.
- **Documentation validation becomes human approval.** Contract results prove topic and claim checks, not human sign-off.
- **PRD modified to fit prose.** The documentation stage consumes canonical sources; it does not rewrite them.

## Validation

Self-check each documentation change:

- [ ] Every required topic appears in its owning document with an authoritative source.
- [ ] Commands, flags, defaults, protocol, state names, limits, TTY, and accessibility claims match implementation.
- [ ] Security and threat documents contain explicit negative residual-risk disclosures.
- [ ] Unknown-delivery, read-receipt, privacy, live-service, and release claims have correct polarity and scope.
- [ ] Synthetic examples and files contain no token, private key, authorization value, or real team content.
- [ ] The exact `documentation_contract` passes and the report makes no human or live acceptance claim.

Run the task-declared checks, normally:

```bash
cargo nextest run --no-tests fail -E 'binary_id(documentation_contract)'
```

If the contract is unavailable or reports zero selected tests, then report the missing prerequisite instead of accepting keyword-only checks.
