# Release Notes

This directory records release communication for `repo-com` without implying
that a source snapshot is a published release.

## Version status

The authoritative workspace/package version is `0.1.0`. It is consistent across
[`Cargo.toml`](../../Cargo.toml), the workspace package manifests, and
[`dist-workspace.toml`](../../dist-workspace.toml). The Product Vision's `1.0`
is a planned product target.

The inspected source snapshot is dated 2026-09-25 at commit `8dd506c`. The
repository has no Git tag, published archive, installer, or release date.
Accordingly, the current communication is split into:

| Record | Meaning |
|---|---|
| [`0.1.0.md`](0.1.0.md) | Versioned source-snapshot notes; **not released** |
| [`unreleased.md`](unreleased.md) | Ongoing changes and the current evidence boundary |

## Implemented snapshot

The source contains 31 Rust packages and a final `repo-com` executable target.
It includes configuration, local state, draft/approval/policy workflows,
Discord setup/message/inbound adapters, delivery recovery contracts, local
audit/lifecycle/retention/purge operations, accessible terminal output,
mocked E2E coverage, performance tooling, and CI/release policies.

The Discord evidence is token-free and WireMock-based. The performance result
is local to the inspected host, not a pinned CI reference-runner result. No
human UX/security review, live Discord acceptance, final release sign-off, or
published cross-platform artifact is recorded.

## Release and upgrade policy

The release workflow is tag-driven and validates CI policy, performance,
release-plan drift, actionlint, dependency/advisory/license checks, secret
scans, static SQLite linkage, checksums, source archives, license evidence,
and CycloneDX SBOMs. It does not publish from this checkout, configure an
updater, or mutate Discord during setup. Installation and future upgrades
remain explicit operator actions.

When a real tag and publication date exist:

1. Verify the tag and authoritative package version.
2. Replace the unreleased status with the real release date; do not reuse the
   source-snapshot date as a release date.
3. Create the real release notes file from the release-notes template; retain
   `0.1.0.md` as a source-snapshot record rather than treating it as the tagged
   release.
4. Move the corresponding changelog entries under the real version and date.
5. Link the new notes from the [README](../../README.md), this index, and the
   changelog.
6. Re-run build, test, lint, link, secret, and documentation checks, keeping
   automated evidence separate from human and live-service sign-off.
