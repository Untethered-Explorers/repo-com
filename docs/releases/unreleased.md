# repo-com — Unreleased

**Workspace/package version:** `0.1.0`
**Release date:** Not released
**Source snapshot:** 2026-09-25 (`HEAD 8dd506c`)

The authoritative version sources (`Cargo.toml`, workspace package manifests,
and `dist-workspace.toml`) agree on `0.1.0`. The Product Vision's `1.0` entry
is a planned product target, not a released version. No Git tag, published
archive, installer, or publication date is recorded.

The detailed versioned source-snapshot communication is in
[`0.1.0.md`](0.1.0.md). This page remains the place for changes that are not a
published release.

## Current implementation

The checkout composes 31 Rust packages into the `repo-com` executable target.
It includes strict schema-version-1 configuration, repository-scoped SQLite
state, immutable drafts, exact approval and policy activation, dedicated-bot
Discord REST v10 setup/message/inbound adapters, local delivery claims,
untrusted inbound state, reply drafts, audit, lifecycle inspection, retention,
purge, accessible terminal output, and release/CI policies.

## Current evidence

On the inspected local source snapshot:

- `cargo fmt --all -- --check` passed;
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` passed;
- full nextest reported 325 passed tests across 35 binaries with 2 skipped;
- the documentation contract reported 7 passed tests; and
- the local performance harness collected 100 samples for each of 8 no-network
  command classes and stayed below the 500 ms p95 threshold on the inspected
  Linux host.

The performance host was not identified as the pinned CI reference runner.
These results are automated local evidence, not live Discord compatibility,
human UX/security acceptance, or release approval. The full commands are in
[the versioned source snapshot](0.1.0.md).

## Current limitations

- No tagged or published `0.1.0` artifact exists in this checkout.
- The release workflow is tag-driven and has no publication or updater step;
  its presence is not artifact evidence.
- The current command tree has no separate reconciliation, policy-deactivation,
  or retention-sweep command.
- The final `inbox.fetch` path currently supplies an empty accepted-delivery
  list to the fetcher, so direct bot mentions are the reliable CLI correlation
  path; non-mention replies may be omitted.
- Local state is not encrypted at rest, relies on user-only filesystem
  permissions, and remains exposed to local-account compromise, backups, and
  filesystem snapshots. The workspace sends no telemetry.
- There are no read receipts, response analytics, arbitrary destinations,
  user-token support, or automatic unknown-delivery resend.
- Human UX/security reviews, live Discord acceptance, and final release
  sign-off remain pending.

## Documentation

- [Versioned `0.1.0` source snapshot](0.1.0.md)
- [Release index](README.md)
- [User Guide](../user-guide.md)
- [Administrator Guide](../admin-guide.md)
- [Operator guide](../operator-guide.md)
- [Configuration contract](../configuration.md)
- [Discord setup](../discord-setup.md)
- [Security model](../security-model.md)
- [Threat model](../threat-model.md)
- [Changelog](../../CHANGELOG.md)

Automated documentation and test results support review; they do not grant
human approval or make a release decision.
