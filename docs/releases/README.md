# Release Notes

This directory holds release communication for `repo-com`.

The workspace version is `0.1.0`, but there are no Git tags, packaged artifacts,
or installable `repo-com` binary. The current release boundary is therefore
[Unreleased](unreleased.md), which documents the 19 implemented Rust library
packages and the remaining product work.

## Current status

- The current source is a Rust 2024 library workspace.
- The full local validation run is documented in the [Unreleased notes](unreleased.md).
- Discord REST v10, message, and inbound adapters exist as focused libraries;
  automated evidence uses token-free WireMock fixtures, not a live Discord
  workspace.
- The product vision and feature documents describe planned CLI and human-facing
  behavior; they are not release evidence.
- No final command tree, terminal UI, packaged cross-platform release, purge or
  lifecycle command layer, performance result, live Discord acceptance, or human
  release sign-off has been recorded.

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
