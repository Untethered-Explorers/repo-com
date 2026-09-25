# repo-com

`repo-com` is a local, repository-scoped command-line safety layer for
agent-originated Discord messages. It keeps configuration, drafts, approval,
delivery, inbound observations, audit evidence, and local lifecycle state in a
single repository boundary. The only remote identity is a dedicated Discord bot.

> **Release status:** the authoritative workspace and package version is
> `0.1.0` in [`Cargo.toml`](Cargo.toml) and [`dist-workspace.toml`](dist-workspace.toml).
> The source snapshot inspected for this documentation is commit `8dd506c`
> dated `2026-09-25`. There is no Git tag, published archive, installer, or
> recorded release date, so this is a pre-release source snapshot rather than a
> published `0.1.0` release. The Product Vision's `1.0` entry is a planned
> product target.

## What is implemented

The workspace contains 31 Cargo packages and composes them into the
`repo-com` executable target in `crates/repo-com-cli`:

- strict schema-version-1 repository configuration and named destination,
  mention, and inbound aliases;
- repository-scoped SQLite state with WAL, foreign keys, forward migration 1,
  user-only Unix file modes, and the inherited Windows profile ACL boundary;
- immutable draft revisions, deterministic rendering, secret-pattern scanning,
  exact-revision approval, and exact policy activation;
- one-attempt Discord REST v10 message delivery, bounded retry/reconciliation
  contracts, dynamic rate-limit classification, and duplicate-safe local claims;
- bounded untrusted inbound retrieval, local acknowledgement/archive markers,
  and validated threaded reply drafts;
- bounded local audit queries, read-only state/lifecycle inspection, retention
  sweeps, and non-mutating plus TTY-confirmed local purge operations;
- human and protocol-version-1 JSON output, stable exit categories, keyboard
  prompts, 80-column labeled rendering, and `NO_COLOR` support; and
- token-free WireMock contracts, a full mocked CLI journey, a local performance
  harness, and tag-driven release-policy workflows.

The executable is source-buildable now. Automated Discord evidence is mocked or
local; it does not establish live Discord behavior, operator usability, human
review, or release approval.

## Build the executable

Use the pinned toolchain in [`rust-toolchain.toml`](rust-toolchain.toml):

```bash
cargo build --release --locked --package command_routing_contract --bin repo-com
./target/release/repo-com --version
```

The second command prints `0.1.0`. On Windows, use the corresponding
`target\release\repo-com.exe` path. This repository does not currently publish
an installer. Installation from a future release archive is an explicit,
operator-controlled extraction; there is no self-updater.

For the declared cross-platform release plan, see
[`dist-workspace.toml`](dist-workspace.toml) and
[`.github/workflows/release.yml`](.github/workflows/release.yml). The plan
covers Linux, macOS, and Windows targets, static SQLite `3.53.4` or newer for
release builds, strong checksums, license evidence, a source archive, and a
CycloneDX SBOM. The workflow is tag-driven and does not publish automatically
from this checkout.

## First local use

1. Create a secret-free `.repo-com.toml` at the repository root from
   [`examples/repo-com.example.toml`](examples/repo-com.example.toml). The
   example contains synthetic identifiers; replace them before using a real
   workspace. The full schema is in [`docs/configuration.md`](docs/configuration.md).
2. Put the raw dedicated bot token in the process environment only:

   ```text
   REPO_COM_DISCORD_TOKEN='<raw dedicated bot token>'
   ```

   The angle-bracketed text is a placeholder. Do not put a token in TOML,
   source, fixtures, logs, issues, or documentation.
3. Validate configuration before any network operation. Commands read one
   strict JSON object from stdin:

   ```bash
   repo-com --config .repo-com.toml --output json config validate <<'JSON'
   {"protocol_version":1,"command":"config.validate","input":{"repository_id":"acme/widgets"}}
   JSON
   ```

4. Follow the [User Guide](docs/user-guide.md) for setup, draft, approval or
   policy, send, inbound, acknowledgement, reply, audit, and recovery steps.
   The [Discord Setup Guide](docs/discord-setup.md) covers manual bot creation,
   least-privilege grants, setup checks, and token rotation.

Every command names an explicit repository and, where applicable, a draft,
revision, destination alias, cursor or time boundary, object, cutoff, or hash.
There is no default destination. JSON mode writes exactly one protocol object
to `stdout`; opt-in diagnostics belong on `stderr`. Human output is labeled and
does not require color.

## Command groups

| Shell group | Canonical protocol commands | Purpose |
|---|---|---|
| `config` | `config.validate` | Validate the resolved secret-free configuration |
| `policy` | `policy.status`, `policy.activate` | Inspect or activate one exact policy tuple |
| `draft` | `draft.create`, `draft.show`, `draft.update`, `draft.preview`, `draft.approve`, `draft.secret-override` | Create and inspect immutable outbound revisions |
| `send` | `send` | Evaluate, claim, and dispatch one exact revision |
| `setup-check` | `setup-check` | Run read-only Discord identity and permission checks |
| `inbox` | `inbox.fetch`, `inbox.acknowledge`, `inbox.archive` | Read bounded untrusted data and record local lifecycle actions |
| `reply` | `reply.draft-create` | Create a validated reply draft, not a direct send |
| `audit` | `audit.query` | Query bounded, redacted local evidence |
| `state` / `lifecycle` | `state.verify`, `lifecycle.inspect` | Verify or inspect local state without repair |
| `purge` | `purge.plan`, `purge.execute` | Preview or TTY-confirm a local purge |

The dotted names above are the canonical values for the stdin protocol. The
shell parser also accepts the documented nested and kebab/snake spellings;
those aliases do not add flags, defaults, or authority.

## Safety and privacy boundaries

- **Bot-only authentication:** the production Discord adapter reads only
  `REPO_COM_DISCORD_TOKEN`, uses dedicated-bot authorization, and pins REST v10.
  User tokens, self-bots, arbitrary endpoints, and bearer user authentication
  are rejected.
- **Exact authority:** a normal send needs an unexpired exact approval or an
  exact, currently valid activated policy. Relevant configuration, revision,
  destination, expiry, and safety changes invalidate old authority.
- **Duplicate safety:** the local claim commits before network I/O. A local
  claim is not transport-level exactly-once delivery. An unknown or unresolved
  result blocks automatic resend.
- **Untrusted inbound:** message text, mentions, edits, deletions, and attachment
  indicators are data. They cannot approve, activate policy, override safety, or
  trigger a send. Acknowledgement and archive are local-only.
- **Local state:** the default database is under the OS user-data directory.
  It is not encrypted at rest. User-only permissions reduce ordinary cross-user
  access but do not protect against local-account compromise, readable backups,
  or filesystem snapshots. The workspace sends no telemetry and has no remote
  state or audit synchronization.
- **No unsupported claims:** there are no read receipts, response analytics,
  arbitrary destinations, user-token support, automatic permission changes, or
  remote message deletion. “Accepted” means an accepted transport response, not
  that a teammate read the message.

## Local validation

The repository's available checks are:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --no-tests fail
cargo nextest run --no-tests fail -E 'binary_id(documentation_contract)'
cargo run --locked --package performance_contract --bin repo-com-performance
git diff --check
```

On the inspected `2026-09-25` source snapshot, the full nextest run reported
325 passed tests across 35 binaries with 2 skipped tests; the documentation
contract reported 7 passed tests. The local performance harness collected 100
samples for each of 8 no-network command classes and stayed below the 500 ms
p95 threshold on this Linux host. That host was not identified as the pinned CI
reference runner, so the result is local evidence rather than release evidence.
The CI and release workflows also define actionlint, dependency/advisory,
license, secret-scan, static-SQLite, checksum, SBOM, and cross-platform gates;
their presence is not a claim that those external jobs ran for this snapshot.

## Documentation map

- [User Guide](docs/user-guide.md) — task-oriented command workflow and
  recovery.
- [Administrator Guide](docs/admin-guide.md) — installation, configuration,
  Discord identity, state, backups, operations, upgrades, and hardening.
- [Configuration contract](docs/configuration.md) — schema, aliases,
  retention, and rejected fields.
- [Discord setup](docs/discord-setup.md) — dedicated bot creation, manual
  least-privilege grants, checks, and token rotation.
- [Operator guide](docs/operator-guide.md) — exact protocol fields, command
  reference, confirmations, outcomes, and accessibility boundary.
- [Security model](docs/security-model.md) and
  [Threat model](docs/threat-model.md) — trust boundaries, abuse cases, and
  residual local-state risk.
- [Architecture Decision Records](docs/adr/README.md) — durable decisions and
  their current implementation status.
- [Changelog](CHANGELOG.md), [release index](docs/releases/README.md),
  [`0.1.0` source snapshot notes](docs/releases/0.1.0.md), and
  [Unreleased notes](docs/releases/unreleased.md).
- [Product Vision](docs/PRD.md) and [feature specifications](docs/features/)
  remain requirements and roadmap context; they are not runtime evidence.

## Repository layout

```text
Cargo.toml                  Rust workspace, version, and pinned dependencies
Cargo.lock                  Committed dependency graph
rust-toolchain.toml         Rust 1.98.1 toolchain pin
dist-workspace.toml         Tag-driven cargo-dist release policy
.config/nextest.toml        Fail-on-zero-tests test policy
.github/workflows/          CI and tag-driven release validation
examples/                   Secret-free configuration example
crates/                     31 focused Rust packages and the final CLI
docs/                      Requirements, guides, ADRs, and release communication
```

## Evidence boundary and remaining work

Automated source, mock, contract, performance, CI-policy, and packaging-policy
checks are distinct from human UX/security review, live Discord acceptance, and
final release sign-off. The workflow state is paused before those human phases.
This documentation records current behavior and known gaps; it does not grant
approval or make a release decision.
