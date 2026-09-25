# Administrator Guide

> **Status:** this guide covers the composed `repo-com` executable in the
> `0.1.0` source snapshot inspected on 2026-09-25. The workspace has a final
> binary target, but no Git tag, published archive, installer, or recorded
> release date. Human UX/security review, live Discord acceptance, and final
> release sign-off remain separate pending work.

## Responsibilities and Architecture

The operator or embedding application is responsible for:

- supplying a secret-free schema-version-1 `.repo-com.toml` and registering the
  repository before repository-scoped state is used;
- controlling the local account, process environment, and the raw dedicated bot
  token in `REPO_COM_DISCORD_TOKEN`;
- creating a dedicated Discord bot and manually granting least-privilege access;
- reviewing complete draft previews before approval, policy activation, secret
  override, or confirmed purge;
- preserving exact identifiers, hashes, timestamps, and transaction boundaries;
- protecting the SQLite database, its parent directory, WAL sidecars, and every
  backup or snapshot; and
- treating inbound content and remote results as untrusted observations.

The current workspace has 31 Cargo packages. The final process is composed by
`crates/repo-com-cli`; it routes structured protocol-version-1 input to the
operations and messaging handlers, which in turn use focused state, policy,
draft, Discord, delivery, inbound, reply, audit, retention, purge, lifecycle,
and terminal crates. SQLite WAL, foreign-key enforcement, and a bounded busy
timeout are the local concurrency boundary. There is no daemon, shared service,
or background scheduler.

## Prerequisites

For source development and validation:

- Rust installed through the official [rustup installation guidance](https://doc.rust-lang.org/stable/cargo/getting-started/installation.html)
  or an equivalent toolchain manager;
- the pinned Rust `1.98.1` toolchain from [`rust-toolchain.toml`](../rust-toolchain.toml);
- Cargo and the dependency graph in [`Cargo.lock`](../Cargo.lock);
- `cargo-nextest` for the declared fail-on-zero-tests commands; and
- a writable OS user-data directory for the default state database.

A release build enforces linked SQLite `3.53.4` or newer before creating or
mutating operational state. Debug contract tests may use a newer local SQLite
engine, but that is not release-artifact evidence. The release workflow requires
a pre-provisioned static SQLite library and rejects dynamic SQLite linkage.

A Discord bot, token, workspace, and network connection are not needed for local
configuration, state, audit, lifecycle, or purge operations. A command that uses
the production Discord adapter requires a raw dedicated bot token in
`REPO_COM_DISCORD_TOKEN` and network access to the fixed official Discord REST
v10 origin.

## Installation

### Build from this checkout

There is no published `repo-com` package or installer in this snapshot. Build the
binary explicitly:

```bash
cargo build --release --locked --package command_routing_contract --bin repo-com
./target/release/repo-com --version
```

The version output is `0.1.0`. On Windows, use the `.exe` binary. Copying or
placing that binary on `PATH` is an explicit operator action; the project does
not silently install or update it.

### Release packaging policy

[`dist-workspace.toml`](../dist-workspace.toml) selects only the
`command_routing_contract` package and declares:

- `x86_64-unknown-linux-gnu`;
- `x86_64-apple-darwin`;
- `aarch64-apple-darwin`; and
- `x86_64-pc-windows-msvc`.

It configures a source tarball, SHA-256 checksums, dependency license evidence,
CycloneDX SBOM generation, and no installers or updater. The tag-driven
[release workflow](../.github/workflows/release.yml) validates the policy,
performance evidence, release plan, local artifacts, checksums, SBOM, linkage,
and secret scans. It is a policy/workflow, not evidence that an archive was
published from this checkout. A future operator who receives a release archive
must verify its checksum, inspect the license and SBOM evidence, extract the
binary explicitly, and retain the source/version association. No setup mutation
or self-update is part of installation.

## Configuration

The configuration resolver searches for `.repo-com.toml` from the current
directory through the nearest repository root. It accepts exactly one candidate;
an explicit `--config PATH` must be normalized and remain inside that root.
Missing, ambiguous, out-of-root, unsafe, or unsupported documents fail closed.

The strict v1 model requires:

- `schema_version = 1` and a valid `repository_id`;
- one `[discord]` workspace ID;
- named `[destinations.<alias>]` channel records and `allowed_mentions` aliases;
- named `[mentions.<alias>]` targets using only `role:<id>` or `user:<id>`;
- enabled `[inbound.<alias>]` records that inherit the matching destination;
- `[retention]` with `content_days` and `metadata_days`; and
- exact `[[auto_send]]` tuples of `event_type`, destination alias, and
  `severity`.

Unknown fields, future schema versions, secret-like keys, raw destinations,
unlisted mention targets, duplicate aliases, malformed IDs, wildcard policy
components, and known cross-workspace references are rejected when a separately
populated workspace reference index is supplied. A valid local
file does not prove that Discord permissions or current remote state are ready.

The normalized configuration is hashed with SHA-256. Approval, policy, and
purge decisions carry that hash so a relevant configuration change invalidates
old authority. Start with the synthetic
[`examples/repo-com.example.toml`](../examples/repo-com.example.toml); do not
copy production message content or credentials into a fixture.

Retention defaults are 30 days for draft/inbound content and 365 days for
non-content metadata. Content overrides are 1 through 365 days; metadata
overrides are 30 through 3,650 days, and metadata retention must be at least
content retention. The current executable has no `retention.sweep` command; the
retention service must be invoked by an embedding application or a separately
reviewed local process.

## Identity, Secrets, and TLS

The Discord adapter accepts only a raw dedicated bot token from
`REPO_COM_DISCORD_TOKEN`. It uses the Bot authorization scheme, pins REST v10,
and stores owned token copies in zeroizing wrappers. User tokens, self-bots,
bearer user authentication, arbitrary endpoints, and unlisted API versions fail
before remote use. The production origin is fixed; loopback origins are for
controlled test constructors only.

A workspace administrator should grant only the permissions reported by the
setup check:

| Use | Permission | Why |
|---|---|---|
| Destination channel | `VIEW_CHANNEL` | Resolve and inspect the configured channel |
| Destination channel | `SEND_MESSAGES` | Permit the one outbound text message |
| Enabled inbound channel | `READ_MESSAGE_HISTORY` | Read the explicit inbound page and point checks |
| Allowlisted role mention | `MENTION_ROLES` | Permit a role mention under current guild rules |

`setup-check` is read-only. It uses REST v10 GET requests to check bot
identity, workspace membership, channel visibility/permissions, and resolved
mentions. It never creates an application, invites a bot, changes roles, grants
permissions, or edits a Discord message. Review the structured remediation and
make any permission change manually; remote state can change immediately after
the check.

For token rotation or revocation:

1. Use the official Discord Developer Portal's bot reset/revoke control.
2. Replace the process environment value without writing it to a file or log.
3. Remove the old value from the process and any approved secret manager.
4. Run `setup-check` again and review identity, membership, channel, and mention
   results.

A token value, authorization header value, private key, or response body is not
a safe ticket, log, fixture, or documentation artifact.

## Storage and Backups

### Location and protection

The default database path is:

```text
<dirs::data_local_dir()>/repo-com/state.sqlite3
```

`database_path_in(root)` provides the same `repo-com/state.sqlite3` layout below
an explicit data root for isolated tests or a controlled deployment. Keep live
state outside the repository. On Unix, the state layer creates the parent
`0700` and database `0600` and refuses a symbolic-link database path. On
Windows, the implementation relies on the inherited user-profile ACL and does
not rewrite portable permission bits as a substitute for an ACL.

The schema version is `1` and migrations are forward-only. SQLite WAL may create
`-wal` and `-shm` sidecars while a connection is active. A main database file
alone is not a complete live backup. Coordinate a copy with the embedding
application and SQLite's backup/checkpoint semantics; this repository does not
provide a backup command.

### Privacy boundary

The database is not encrypted at rest. User-only permissions reduce ordinary
cross-user access but do not protect against a compromised local account,
malware running as that account, readable backups, filesystem snapshots, or a
support recipient. Retention and local purge reduce future copies in the
current store; they cannot revoke copies already held elsewhere. The workspace
sends no telemetry and does not synchronize local state or audit evidence to a
remote service.

## Health Checks and Monitoring

The local quality gates are:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --no-tests fail
cargo nextest run --no-tests fail -E 'binary_id(documentation_contract)'
```

The state verifier is read-only. `state verify` accepts an explicit
`database_path` and reports quick-check, foreign-key, migration, repository
scope, and filesystem-permission status without creating, repairing, or
migrating a database. `state inspect`/`lifecycle inspect` returns bounded local
projections. Audit queries are bounded to 100 rows per page, repository-scoped,
redacted, and read-only. None of these operations contacts Discord or uploads
state.

The final executable has no monitoring daemon, crash uploader, health endpoint,
or automatic retention scheduler. The Discord setup check is a point-in-time
read. The local performance harness is a separate command; its result is not
proof of network latency, live compatibility, or a pinned CI runner unless the
corresponding CI evidence says so.

### Network validation boundary

Discord setup, message, inbound, and mocked end-to-end contracts use token-free
WireMock fixtures and controlled responses. They validate request shape,
authentication, permissions, rate-limit classification, untrusted filtering,
correlation, and duplicate prevention within that boundary. They do not prove
current Discord policy or a live workspace. A production token and disposable
workspace belong to a separate human-controlled acceptance task.

## Upgrades and Rollback

There is no self-updater and no packaged rollback procedure in this source
snapshot. Before changing or replacing a binary:

1. Preserve the database and active WAL sidecars using a SQLite-aware backup
   procedure and equivalent local access controls.
2. Record the binary/package version, toolchain, configuration hash, and
   migration version.
3. Run formatting, clippy, full tests, the documentation contract, and the
   relevant CI/release-policy checks.
4. Verify the linked SQLite version and linkage for a release build before
   opening operational state.
5. Keep the previous source or binary for diagnosis, but do not assume it can
   open a newer schema.

A changed configuration, draft, destination, expiry, or authority invalidates
old approval or policy decisions. Generate a new preview and decision; do not
reuse an old confirmation or plan hash.

## Troubleshooting

| Symptom | Likely cause | Next action |
|---|---|---|
| `config-not-found` | No configuration in the bounded repository search | Add a valid `.repo-com.toml` or pass an in-root path |
| `multiple-config-candidates` | More than one ancestor configuration | Remove the extra file or use `--config` explicitly |
| `unsupported-schema-version` | Configuration is not schema 1 | Update it using the example |
| `secret-field` / `raw-destination-field` | Forbidden key or raw channel | Remove it; use aliases and keep secrets in the environment |
| `operator-action-required` | TTY-only authority action was requested in automation | Run the human flow in a real terminal; never self-approve |
| `policy-blocked` | Exact tuple, hash, or activation is missing/stale/ambiguous | Inspect status and create a fresh exact decision |
| `authentication` | Discord rejected the dedicated bot credential | Rotate/revoke and update `REPO_COM_DISCORD_TOKEN` |
| `permission` | Required remote access is absent | Apply only the reported manual permission and rerun setup |
| `unknown-delivery` | Remote dispatch may have taken effect | Keep blocked and perform read-only reconciliation; do not resend |
| `connectivity-rate-limit` | Network or dynamic Discord limit | Honor server-provided delay metadata and retry only a proven safe operation |
| `storage-integrity` | SQLite, lock, migration, permission, or transaction failure | Preserve the database and sidecars; do not delete or recreate it |
| `UnsupportedSqliteRuntime` | Release runtime is below 3.53.4 | Use the approved release runtime or remain in development validation |
| Purge plan changed | Configuration or local state changed | Generate a new plan and obtain a new exact TTY confirmation |

## Security and Privacy Checklist

- [ ] Keep `.repo-com.toml`, examples, fixtures, logs, and support material free
  of tokens, passwords, private keys, authorization values, and real team
  content.
- [ ] Provide a raw dedicated bot token only through
  `REPO_COM_DISCORD_TOKEN`.
- [ ] Use a dedicated bot and manually grant only `VIEW_CHANNEL`,
  `SEND_MESSAGES`, `READ_MESSAGE_HISTORY`, and required mention access.
- [ ] Keep the state database and sidecars outside the repository and out of
  shared storage.
- [ ] Verify owner-only Unix modes or the inherited Windows profile ACL.
- [ ] Protect unencrypted backups and snapshots with equivalent access control.
- [ ] Treat inbound text, mentions, edits, and deletions as untrusted data.
- [ ] Treat accepted delivery as transport evidence, not a read receipt.
- [ ] Keep unknown and unresolved delivery blocked until read-only
  reconciliation resolves it.
- [ ] Use the redacted audit writer and never put response bodies in logs.
- [ ] Remember that v1 has no encryption at rest, sends no telemetry, and does
  not protect against local-account compromise, backups, or snapshots.
- [ ] Do not describe automated evidence as human approval, live acceptance, or
  release sign-off.

## Recovery and Support

- **Stale policy:** inspect the current configuration and tuple hashes. A
  changed hash or ambiguous active row grants no authority; resolve it under
  operator control.
- **Changed draft or destination:** create a new immutable revision, preview it,
  and obtain a new exact approval or policy decision.
- **Unknown delivery:** retain exact destination, bot author, nonce, content,
  and attempt evidence. Use read-only reconciliation; the current command tree
  does not expose a `reconcile` command, and no automatic resend is permitted.
- **Inbound page or cursor failure:** preserve the prior local state. A page
  commit does not prove that later point checks completed as one remote
  transaction.
- **Corrupt or unsupported state:** preserve the database and WAL sidecars and
  inspect the typed error. The verifier does not repair or recreate state.
- **Configuration mistake:** use the safe error code and field path; do not
  include original scalar values in an issue or log.
- **Discord setup failure:** follow the structured remediation for bot identity,
  membership, channel visibility, permissions, and mention access. Setup is
  read-only.

## Further documentation

- [User Guide](user-guide.md)
- [Operator guide](operator-guide.md)
- [Configuration contract](configuration.md)
- [Discord setup](discord-setup.md)
- [Security model](security-model.md)
- [Threat model](threat-model.md)
- [Changelog](../CHANGELOG.md)
- [`0.1.0` source snapshot release notes](releases/0.1.0.md)
- [Unreleased release notes](releases/unreleased.md)
- [Architecture Decision Records](adr/README.md)

This guide documents current behavior and residual risk. It does not authorize
a live Discord action, a human review decision, or a release.
