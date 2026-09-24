# Release Gate Inventory

> Load when: classifying a release condition, writing CI or packaging policy, scanning generated artifacts, or qualifying a release claim.

## Evidence Classes

| Class | Meaning | May claim | Must not claim |
|---|---|---|---|
| Policy/configuration | Source encodes a required condition | the gate is defined | target, run, artifact, or release passed |
| Machine contract | Deterministic validator tests policy | the policy contract passed | real platform or live service result |
| CI run | Observed target-specific job | that target's observed result | another target or publication |
| Performance sample | 100 warm no-network samples with environment and p50/p95/max | measured performance within stated scope | human UX or production latency |
| Mocked E2E | Token-free WireMock journey and request counts | mocked duplicate and no-network behavior | live Discord or human acceptance |
| Release artifact | Generated archive, installer, checksum, license report, SBOM | that exact artifact's inspected properties | unreviewed source or publication |
| Human review | Named review record | only the recorded decision and scope | evidence absent from the record |

## CI Gate Inventory

Require failure on:

- formatting or clippy drift;
- an empty selected-test set;
- missing or incorrect binary smoke checks;
- performance threshold or sample-shape failure;
- RustSec advisories;
- denied or unknown licenses;
- source, fixture, log, or generated secret findings;
- stale lockfile or unsupported SQLite runtime;
- invalid workflow syntax or policy drift;
- missing required target in the matrix.

CI policy should pin tools, use least privilege, protect secrets from untrusted pull requests, and declare Linux, macOS, and Windows jobs. The matrix declaration is not the run evidence.

## Release Gate Inventory

Require:

- semantic-versioned Linux, macOS, and Windows artifacts;
- archives or installers and a source archive;
- SHA-256 or stronger checksums;
- dependency license evidence;
- CycloneDX SBOM;
- statically linked patched SQLite at or above the required version;
- tag-driven release only after validated CI, performance, E2E, and release-plan evidence;
- no updater, automatic setup mutation, or remote deletion behavior.

## Generated-Artifact Scan

Scan and inspect:

- source and workflow files;
- logs and test fixtures;
- generated metadata and lockfiles;
- checksum inputs and outputs;
- license reports and SBOMs;
- documentation and review files.

Look for credentials, authorization values, real team content, wrong target names, stale release-plan fields, dynamic SQLite linkage, unexpected updater or setup behavior, and publication steps in planning tasks.

## Prohibited Shortcuts

- Do not infer macOS or Windows from a local Linux command.
- Do not infer static patched SQLite from a lockfile.
- Do not infer policy compliance from `actionlint` alone.
- Do not infer artifact existence from `dist-workspace.toml`.
- Do not infer a release from a release workflow file.
- Do not infer live Discord from Wiremock.
- Do not infer human approval from generated tests or snapshots.
- Do not invent checksums, license results, SBOM contents, advisory status, or run counts.

## Task-Local Commands

```bash
cargo nextest run --no-tests fail -E 'binary_id(ci_policy_contract)'
actionlint .github/workflows/ci.yml
cargo nextest run --no-tests fail -E 'binary_id(release_policy_contract)'
actionlint .github/workflows/release.yml
```

The exact commands remain owned by the task contract. Do not create or use a generic `run-rust-task-checks` package.
