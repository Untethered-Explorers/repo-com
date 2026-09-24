---
name: separate-automated-and-human-evidence
description: "Keep repo-com generated tests, performance samples, documentation, CI, and release artifacts distinct from human UX, security, live Discord, and final sign-off records; use when classifying evidence, preparing human-review prerequisites, or preventing fabricated approvals and claims."
---

# Separate Automated and Human Evidence

Classify evidence by owner and claim scope before reporting completion. This skill protects the four repo-com human-review boundaries and the implementation evidence that precedes them; it does not let an agent create scores, attestations, live observations, or sign-off.

Load the [evidence and review matrix](./references/evidence-and-review-matrix.md) before collecting artifacts, populating a review file, or making a completion claim.

## Process

### Step 1: Classify the task contract

Read the exact task kind and file ownership:

- `implementation` may produce only its declared code, tests, fixtures, workflows, documents, and machine-readable evidence;
- `human-review` has no model owner, no expected implementation outputs, and no autonomous validation commands.

If the task is human-review, then stop implementation work and identify the designated review owner and prerequisites. Do not create a placeholder score or generic approval to unblock execution.

### Step 2: Bind automated evidence to the exact task

Collect only task-owned outputs and run the task's exact validation commands. Record actual command, environment, scope, exit status, selected test count, and output identity. Do not broaden evidence into another task's claim.

Qualify performance samples with environment, 100 warm no-network samples per command class, p50, p95, maximum, threshold, and exclusions. Qualify E2E with token-free mocked boundaries, request counts, concurrency, duplicate prevention, and no-external-network evidence.

### Step 3: Scan every evidence surface

Inspect source, tests, fixtures, logs, snapshots, workflows, generated metadata, performance samples, documentation, review files, checksums, license reports, and SBOMs for bot tokens, authorization values, private keys, and real team content.

If a secret or private content is found, then block the evidence handoff and follow the owning privacy or security procedure. Never publish the offending value in a report.

### Step 4: Enforce the human-review sequence

Wait for the exact task dependencies before each review:

- terminal UX follows UI, app, performance, documentation, and E2E evidence;
- security and privacy follow CI, packaging, documentation, and E2E evidence;
- live Discord follows UX, security, CI, packaging, and documentation evidence;
- final sign-off follows all reviews and hashed evidence.

If a prerequisite is missing, then keep the review pending. Missing evidence is not implicit approval.

### Step 5: Let only the designated human populate the record

Use the exact review file and schema for the task. Agents may prepare source and automated evidence paths, but may not write human scores, reviewer identity, live observations, pass/fail decisions, conditions, or sign-off.

Validate the record for the task-specific fields and explicit decision. A generic engine attestation or file existence check is not enough for repo-com's four human schemas.

### Step 6: Report the narrowest defensible claim

State exactly what was observed and what was not. Automated results may establish mocked protocol, deterministic rendering, performance samples, documentation checks, policy, or artifact properties. They may not establish human UX, security approval, live Discord compatibility, or final release approval.

For final sign-off, require named reviewers, evidence hashes, explicit decision, conditions, and date. If critical evidence or a reviewer is absent, then the defensible decision is pending or reject, never inferred approval.

## Gotchas

- **Generic attestation treated as the human schema.** A narrow `approved` envelope does not contain repo-com's scores, observations, defects, conditions, or task-specific decision fields.
- **Local policy treated as platform evidence.** A workflow or release-plan validator is not an actual three-platform run.
- **Wiremock treated as live acceptance.** Mocked requests do not prove Discord compatibility or teammate behavior.
- **Snapshot treated as human accessibility review.** Deterministic output proves renderer behavior, not human comprehension.
- **Missing reviewer treated as consent.** Preserve pending or reject; never invent identity or sign-off.
- **Evidence file created by an implementation agent.** Human-review files have exclusive human ownership.
- **Secret retained in a failure report.** Redact and block the handoff without copying the value into evidence.

## Validation

Self-check the evidence boundary:

- [ ] Every artifact is tagged automated, human UX, human security, live Discord, or final sign-off.
- [ ] Automated evidence names the exact task, command, target, environment, and observed result.
- [ ] Performance, mocked E2E, CI, policy, and artifact claims use their narrow scope and contain no human inference.
- [ ] All evidence surfaces are secret-free and contain no real team content.
- [ ] Each human review has its exact prerequisites and designated human-owned file.
- [ ] Human files contain the task-specific fields and explicit decision; no agent populated them.
- [ ] Final sign-off names reviewers, hashes, conditions, and date or remains pending/rejected.

Run only the exact task-declared automated checks, commonly the relevant contract binary, `performance_contract`, `e2e_workflow`, `ci_policy_contract`, or `release_policy_contract`. If a human task has no autonomous command, then do not invent a passing check.
