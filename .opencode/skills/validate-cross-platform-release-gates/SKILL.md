---
name: validate-cross-platform-release-gates
description: "Validate repo-com Linux, macOS, and Windows build, patched SQLite, performance, E2E, advisory, license, secret, workflow, checksum, SBOM, and release-plan gates without publishing; use for CI policy, packaging policy, release artifacts, or cross-platform evidence."
---

# Validate Cross-Platform Release Gates

Separate machine-checkable release policy from observed platform and artifact evidence. This skill validates gates and evidence boundaries for Linux, macOS, and Windows; it does not publish a release or convert planning configuration into a successful run.

Load the [gate inventory](./references/gate-inventory.md) before adding a CI check, packaging rule, artifact scanner, or release claim.

## Process

### Step 1: Resolve the release contract and prerequisites

Read the exact CI, packaging, performance, E2E, and human-review task contracts. Record expected outputs, owners, dependencies, pinned tools, supported targets, and prohibited claims. Confirm that prerequisite implementation and performance or E2E outputs exist before claiming release readiness.

Refresh external advisories, patch versions, Rust and GitHub Actions behavior, cargo-dist, cargo-deny, cargo-audit, and CycloneDX guidance when current facts are required. If a planning version has drifted, then record the required update instead of silently changing a major version.

### Step 2: Classify gate, policy, and evidence

Use three separate labels:

- **gate:** the machine-checkable condition that must fail closed;
- **policy/configuration:** source that encodes the gate;
- **evidence:** actual output from a real run or generated artifact.

If only policy exists, then report that the gate is defined, not that the platform run, packaging, or publication passed.

### Step 3: Validate CI policy

Build and run the exact `ci_policy_contract` before or alongside workflow syntax. Require Linux, macOS, and Windows jobs; pinned tools; least privilege; concurrency control; no secrets on untrusted pull requests; format, clippy, fail-on-zero-tests, performance, RustSec, license, secret, lockfile, SQLite, binary smoke, and workflow-policy checks.

Run `actionlint` for `.github/workflows/ci.yml`, but do not treat syntax validity as proof that every referenced action or policy is correct.

### Step 4: Validate release-plan and packaging policy

Run the exact `release_policy_contract` and `actionlint .github/workflows/release.yml`. Require a tag-driven workflow dependent on validated CI, performance, E2E, and release-plan evidence. Verify the distribution plan targets semantic-versioned Linux, macOS, and Windows artifacts with source archive, checksums, license evidence, CycloneDX SBOM, and statically linked patched SQLite.

Planning and validation must not publish, upload, tag, install, mutate Discord, or create updater behavior.

### Step 5: Verify patched SQLite and generated artifacts

At the actual build or release run, inspect the linked runtime and generated artifacts. SQLite must be statically linked and at least the required patched version; a lockfile or successful build alone is insufficient.

When checksums, license reports, or SBOMs are generated, scan their inputs and outputs for secrets, prohibited updater/setup behavior, missing targets, stale plan content, and unsupported claims. Do not fabricate absent artifacts or substitute a policy file for their contents.

### Step 6: Qualify evidence by actual target

Report the exact target, command, run or artifact identity, and observed result. A local Linux run cannot establish macOS or Windows success. A declared matrix cannot establish a real matrix run. A release configuration cannot establish a published release.

For a human or live result, preserve the separate evidence boundary and never infer approval from automated checks.

## Gotchas

- **Lockfile treated as patched SQLite proof.** Verify the linked runtime and required patched version in the real build or artifact.
- **Local Linux result generalized to three platforms.** Require observed target-specific CI or artifact evidence.
- **`actionlint` treated as policy compliance.** Syntax validation does not inspect permissions, dependency, secret, artifact, or release-plan drift.
- **Release configuration treated as publication.** Planning cannot tag, upload, install, or publish.
- **Generated artifacts skipped by secret scanning.** Scan logs, fixtures, metadata, checksum inputs, SBOMs, documentation, and review files as well as source.
- **Planning versions treated as current.** Refresh advisories and patch versions before release and record drift.
- **Mocked E2E described as live Discord.** Keep automated protocol proof separate from human live acceptance.

## Validation

Self-check the release boundary:

- [ ] Every policy, gate, and observed evidence item is labeled separately.
- [ ] CI requires Linux, macOS, and Windows, pinned tools, least privilege, and token-free untrusted-pull-request behavior.
- [ ] `ci_policy_contract` and the CI workflow syntax check pass exactly.
- [ ] `release_policy_contract` and the release workflow syntax check pass exactly.
- [ ] Performance and token-free mocked E2E prerequisites report their actual sample and request evidence.
- [ ] Any real artifacts include patched static SQLite, checksums, license evidence, an SBOM, and secret scans.
- [ ] No local or policy-only result is reported as a platform run, release, live acceptance, or human sign-off.

Run the exact task commands, commonly:

```bash
cargo nextest run --no-tests fail -E 'binary_id(ci_policy_contract)'
actionlint .github/workflows/ci.yml
cargo nextest run --no-tests fail -E 'binary_id(release_policy_contract)'
actionlint .github/workflows/release.yml
```

If a contract or workflow check is unavailable, then report the missing prerequisite rather than substituting a weaker command.
