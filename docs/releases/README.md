# Release Notes

This directory holds release communication for `repo-com`.

The workspace version is `0.1.0`, but there are no Git tags, packaged artifacts,
or installable `repo-com` binaries. The current release boundary is therefore
[Unreleased](unreleased.md), which documents the four implemented Rust libraries
and the remaining product work.

## Current status

- The current source is a Rust 2024 library workspace.
- Local validation is documented in the [Unreleased notes](unreleased.md).
- The product vision and feature documents describe planned Discord and CLI
  behavior; they are not release evidence.
- No live Discord, human UX/security acceptance, cross-platform package, or
  release sign-off has been performed.

## When a release is tagged

1. Use the authoritative workspace or package version and its release date.
2. Create `<version>.md` from the release-notes template and record only
   implemented behavior plus explicit limitations.
3. Move the corresponding `[Unreleased]` changelog entries under the new version
   and date in [`CHANGELOG.md`](../../CHANGELOG.md).
4. Link the versioned notes from the [README](../../README.md), release index,
   and changelog.
5. Re-run build, test, lint, link, secret, and documentation checks. Keep
   automated evidence distinct from human and live-service sign-off.
