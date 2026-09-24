# Operator guide

This guide owns installation status, the exact protocol-version-1 command and
JSON surfaces implemented by `repo-com-cli-operations` and
`repo-com-cli-messaging`, operator procedures, recovery, accessibility, and
unsupported behavior. The canonical contracts are `REL-FR-02`, `REL-FR-03`,
and `REL-FR-08` in `docs/features/release-readiness.md`, with domain details in
the linked feature documents.

## Installation status

The current checkout is a pre-release Rust workspace of focused library
contracts. It has **no installed `repo-com` binary**, no composed final command
router, and no packaged installer. Do not present a library API or a WireMock
contract as an installed product or as live Discord evidence.

For source-level development, install Rust through the official
[rustup installation guidance](https://doc.rust-lang.org/stable/cargo/getting-started/installation.html),
then use the repository's pinned Rust 1.98.1 toolchain and workspace:

```text
rustup toolchain install 1.98.1
cargo build --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo nextest run --no-tests fail -E 'binary_id(documentation_contract)'
```

`cargo build --workspace` builds the current library crates; it does not
install an executable. Installation of a final `repo-com` binary, semantic
version command, and release artifact belongs to the later executable and
packaging tasks. This guide does not invent an installer name, archive layout,
or packaging command.

The handler crates below are the implemented command contract that the final
executable must compose. They do not open state, contact Discord, or perform
transport themselves; the composition layer supplies domain services, clocks,
configuration, and stream decisions.

## Protocol version 1 and streams

Both handler crates parse one strict outer object from structured input:

```json
{
  "protocol_version": 1,
  "command": "<canonical command>",
  "input": { "<command-specific fields>": "..." }
}
```

The only accepted protocol version is **protocol version 1**. The outer object
rejects unknown fields. The `input` object is decoded into a strict
command-specific type and rejects unknown fields as well. No command is
selected by a default.

In machine mode, stdout contains **exactly one JSON object** in either shape:

```json
{"protocol_version":1,"status":"success","data":{},"error":null}
```

or:

```json
{"protocol_version":1,"status":"error","data":null,"error":{"code":"usage-schema","message":"safe detail"}}
```

All four fields are serialized on every outcome. For a success, `data` contains
the payload and `error` is `null`; for an error, `data` is `null` and `error`
contains the typed error. Diagnostics are optional stderr values. Prompts and
diagnostic text never contaminate machine stdout.

The implemented global option values are:

| Option | Values | Default | Boundary |
|---|---|---|---|
| `--config PATH` | one explicit path | none | The path is normalized and must be within the detected repository root. |
| `--output` | `human`, `json` | `human` | JSON selects one protocol object; human mode is labeled text. |
| `--color` | `auto`, `always`, `never` | `auto` | `never` is the plain-text choice; meaning is also carried by text labels. |
| `--diagnostics` | `off`, `on` | `off` | Diagnostics are opt-in and belong on the diagnostic stream. |

These are the implemented `GlobalArgs` values, not a claim that a final
executable already exists in this checkout.

Stable error categories and deterministic process exits are:

| Protocol code | Exit |
|---|---:|
| `usage-schema` | 2 |
| `operator-action-required` | 3 |
| `policy-blocked` | 4 |
| `authentication` | 5 |
| `permission` | 6 |
| `remote-conflict` | 7 |
| `unknown-delivery` | 8 |
| `storage-integrity` | 9 |
| `connectivity-rate-limit` | 10 |
| `internal-failure` | 1 |

A successful protocol outcome exits 0. A non-TTY invocation never receives an
ambient prompt. TTY is an explicit stream decision: both relevant streams must
be interactive for an operator prompt to be allowed.

## Canonical command surface

The dotted names below are the canonical protocol names. The parsers also
accept the documented kebab/snake shell-style spellings where implemented, but
the canonical dotted name is what automation should record. No command accepts
a raw destination or supplies a hidden default.

### Operations handler commands

| Command | Required and explicit input fields |
|---|---|
| `config.validate` | `repository_id`; optional `config_path`. |
| `policy.status` | `repository_id`, `event_type`, `destination_alias`, `severity`. |
| `policy.activate` | `repository_id`, `event_type`, `destination_alias`, `severity`, `activated_at`; optional `activation_id`. |
| `state.verify` | `repository_id`, `database_path`; optional `expected_migration` (the current schema is used when omitted). |
| `lifecycle.inspect` | `repository_id`, `object_type`, and one explicit `page_size` or `limit` from 1 through 100. Use `object_id` for object families that require it, `revision` only for `draft_revision`, and optional `after`; `include_retained_content` is an explicit opt-in. |
| `audit.query` | `repository_id` and one explicit `page_size` or `limit` from 1 through 100; optional time bounds, `object_type`, `object_id`, `transition`, and repository-scoped `cursor`. |
| `purge.plan` | `repository_id`, `scope` (`content`, `metadata`, or `all`), and exactly one canonical cutoff representation; optional `expected_config_hash`. |
| `purge.execute` | `repository_id`, `scope`, exactly one cutoff representation, `config_hash`, `plan_hash`, and `executed_at`. |

`lifecycle.inspect` accepts these object families: `repository`, `draft`,
`draft_revision`, `delivery_attempt`, `inbound_item`, `acknowledgement`,
`archive`, `reply_link`, and `audit_transition`. `repository` and
`audit_transition` do not take an object identifier. A `draft_revision` takes
an explicit `object_id` and positive `revision`.

For a purge cutoff, use either the single `cutoff` RFC 3339 value or the paired
`cutoff_unix_seconds` and `cutoff_utc` values. The two representations must
agree. Purge execution hashes must be SHA-256 hex values. The current
operations handler does not expose a retention-sweep command; retention is a
local service invoked by the composing application as described below.

### Messaging handler commands

| Command | Required and explicit input fields |
|---|---|
| `draft.create` | `repository_id`, `draft_id`, `destination_alias`, `text`, `event_type`, `severity`, `created_at`, `created_at_unix_seconds`; optional `metadata` and `expires_in_seconds`. |
| `draft.show` | `repository_id`, `draft_id`, positive `revision`. |
| `draft.update` | `repository_id`, `draft_id`, current `revision`, `destination_alias`, `text`, `event_type`, `severity`, `created_at`, `created_at_unix_seconds`; optional `metadata` and `expires_in_seconds`. |
| `draft.preview` | `repository_id`, `draft_id`, positive `revision`. |
| `draft.approve` | `repository_id`, `draft_id`, positive `revision`. |
| `draft.secret-override` | `repository_id`, `draft_id`, positive `revision`; the domain still requires an interactive TTY and the exact reviewed preview. |
| `send` | `repository_id`, `draft_id`, positive `revision`. |
| `setup-check` | `repository_id`. |
| `inbox.fetch` | `repository_id`, enabled inbound `alias`, a caller-supplied `bot_user_id` used as untrusted-filter context, and exactly one of `cursor` or `time`; optional `retrieved_at`. The handler does not independently authenticate that ID. |
| `inbox.acknowledge` | `repository_id`, nonempty `item_ids`, canonical `at`. |
| `inbox.archive` | `repository_id`, nonempty `item_ids`, canonical `at`. |
| `reply.draft-create` | `repository_id`, `inbound_item_id`, new `draft_id`, `text`, `event_type`, `severity`, `created_at`, `created_at_unix_seconds`; optional `metadata` and `expires_in_seconds`. |

Draft metadata accepts only the bounded fields `repository_label`, `branch`,
and `commit`. Unknown metadata fields fail. Normal draft and reply input uses
an alias; a raw channel, role, user, or message target is not accepted as a
workflow destination.

The handler crate's command enum also accepts these shell-style aliases where
implemented: `draft-create`, `draft-show`, `draft-update`, `draft-preview`,
`draft-approve`, `draft-secret-override`, `setup.check`, `inbox-fetch`,
`inbox-acknowledge`, `inbox-archive`, `reply-draft-create`, `config`,
`policy-status`, `policy-activate`, `state`, `lifecycle-inspect`, `state.inspect`, `state-inspect`, `audit`,
`purge-plan`, and `purge-execute`. These aliases do
not add flags or defaults.

## Operator procedures

### 1. Resolve and validate configuration

1. Work from the repository root or provide an explicit in-repository path.
2. Ensure `.repo-com.toml` is secret-free and uses schema version 1.
3. Validate the exact repository scope with `config.validate`.
4. Read the returned configuration hash and alias lists. If validation fails,
   correct the path or field and retry; do not bypass the resolver by adding a
   raw destination.

The validation result is local evidence only. It does not create a Discord
application, grant a permission, or prove that a workspace is live.

### 2. Check Discord setup

Use `setup-check` after setting the environment-only bot token. The result
contains safe bot identity, workspace membership, channel, mention, issue, and
remediation fields. Follow [`discord-setup.md`](discord-setup.md) for manual
permission and rotation actions. A setup report never mutates Discord.

### 3. Create and preview a draft

1. Create one draft with a named destination and bounded text/metadata.
2. Treat the returned draft ID and revision as explicit local identifiers.
3. Run `draft.preview` for the exact repository, draft, and revision.
4. Read the complete labeled preview: resolved destination, exact final text,
   metadata, expiry, approval basis, policy basis, safety finding, and next
   action.
5. Do not send until the preview is current and the safety decision is known.

A revision is immutable. Use `draft.update` to create a new revision; it does
not rewrite an accepted remote message. The default draft expiry is 24 hours,
and a caller may choose at most seven days. Expired revisions cannot be
approved, made eligible, or sent.

The secret scanner checks the final rendered text and metadata for
high-confidence credential patterns. A finding blocks send by default. Only an
interactive TTY operator may review the exact preview and record a redacted
`draft.secret-override`; non-TTY override is forbidden. The scanner is a
lightweight safety check, not complete data-loss prevention.

### 4. Approve exactly what was previewed

`draft.approve` is an interactive TTY action. The operator must see the full
preview first. Approval is bound to the exact repository and revision hash,
which covers text, metadata, destination alias, resolved destination, and
expiry. It expires at the earlier of draft expiry or 15 minutes after approval.
Changing the revision, configuration hash, destination resolution, or policy
basis invalidates it.

Approval is permission to evaluate and claim one exact revision, not a
reusable send bypass. A non-TTY process cannot create approval, activate a
policy, or override a safety finding.

### 5. Inspect and activate a narrow policy

1. Use `policy.status` with the exact `event_type`, `destination_alias`, and
   `severity`.
2. Review the policy state, configuration hash, tuple hash, activation
   snapshot, and next action.
3. If an exact activation is intended, use `policy.activate` in an interactive
   TTY and review the exact activation preview.
4. Record the returned activation ID and hashes. Re-check status before relying
   on the activation.

The policy is equality-only: no wildcard, prefix, broader severity, raw
channel, or implicit default is eligible. Activation is stale after a relevant
configuration or tuple change. Deactivation/inspection may be performed
without a prompt; permission-widening activation may not.

### 6. Send one exact revision and read the outcome

`send` accepts only `repository_id`, `draft_id`, and positive `revision`. The
coordinator must revalidate the current repository/configuration, revision
hash, expiry, destination alias and mention allowlist, approval or exact policy
activation, and safety scan immediately before the local claim. The claim,
attempt record, and audit transition commit before network I/O.

Interpret the typed delivery outcome as one of the implemented states:

- `accepted` — a validated remote message identifier was returned;
- `failed` — a definitive rejection or non-retryable failure was recorded;
- `retry-wait` — a bounded safe retry is pending;
- `unknown` — dispatch may have reached Discord but the result is not known;
- `reconciled-accepted` — read-only reconciliation found one exact message;
- `reconciled-absent` — conservative read evidence satisfied the absence gate;
- `unresolved` — evidence conflicts or is insufficient;
- `eligibility-rejected`, `expired`, `stale-authority`, or `error` — no
  authorized send may proceed.

The local persisted/audit state names are `unclaimed`, `claimed`, `accepted`,
`failed`, `retry_wait`, `unknown`, `reconciled_accepted`, `reconciled_absent`,
and `unresolved`. The terminal renderer may display the corresponding
hyphenated labels for an operator-facing outcome.

The transport policy permits at most three total attempts. A retry is eligible
only for a proven pre-dispatch failure or an HTTP 429 response; an ambiguous
post-dispatch result becomes `unknown` rather than a retry. A Discord-directed
wait is bounded to 30 seconds per attempt.

`accepted` means the local delivery contract observed an accepted response; it
does not mean a teammate read the message. Repo-com has no read receipt and
no response analytics. It also does not edit or delete an accepted remote message.

### 7. Recover an unknown delivery without duplication

If the outcome is `unknown`, stop the normal send loop. **There is no automatic resend.**
The read-only recovery contract is:

1. Read the configured destination only; do not substitute a raw channel.
2. Match the configured bot author, deterministic delivery nonce, and exact
   intended content.
3. Treat one exact match as reconciled acceptance.
4. Keep the result unknown while the observation window lacks a complete safe
   absence proof.
5. Treat a complete conservative absence window as reconciled absence.
6. Keep conflicting, incomplete, or otherwise insufficient evidence unresolved.

An incomplete read is not absence evidence. A later attempt requires a fresh,
explicit operator-authorized decision after reconciliation; it is not a retry
hidden inside `send`, and it is not permission to resend an unresolved result.
The current command-handler crates do not expose a `reconcile` command. The
implemented read-only reconciler is a library service that the final
composition layer may expose after its command contract is defined. Likewise,
`DeliveryCoordinator::claim_retry` is only a local state-machine claim: it
performs no network I/O and is not itself an operator confirmation. The
composition/policy owner must authorize a new attempt separately.

### 8. Fetch inbound replies and mentions safely

`inbox.fetch` requires an enabled configured inbound alias and exactly one
explicit boundary: a numeric last-event cursor or an RFC 3339 time boundary.
The adapter follows deterministic pagination and stops at the implemented hard
limits of 10 pages or 1,000 raw messages. It ignores bot-authored and
repo-com-authored messages in v1 and retains only human replies to accepted
local deliveries or direct mentions of the configured bot.

Every returned envelope is untrusted remote data. The `bot_user_id` input is
validated as a filter identifier; it is not proof that the caller supplied the
real bot identity. A composed application should bind it to the identity
reported by the read-only setup check.

The returned envelope can contain remote IDs,
author, timestamps, text, reply context, mention evidence, attachment
indicators, and provenance, but no field can approve, activate policy, override
safety, or trigger a send. The first observed snapshot, a later current
snapshot or deleted marker, and local transitions are separate records. A
stored snapshot is not current remote truth.

`inbox.acknowledge` and `inbox.archive` are idempotent local actions. They do
not react, edit, delete, assign, or otherwise mutate Discord. The fetch commits
items and the authoritative cursor together; a storage failure leaves the prior
cursor available for a safe repeat.

### 9. Create a reply draft, not a direct reply

`reply.draft-create` validates the repository, stored inbound item, current
retained snapshot, workspace/channel, and authorization. It creates a new
immutable draft with a validated Discord message reference. It does not send
directly, bypass preview/approval, or accept an arbitrary remote message ID.

After the linked reply is accepted through the normal draft lifecycle, local
state can mark the inbound item replied and retain the link/audit event. The
product does not claim that the human-authored response was delivered or read.

### 10. Inspect local audit and lifecycle state

`audit.query` is a bounded local query with filters for repository, time range,
object type, object ID, and transition. `page_size`/`limit` is required and is
1 through 100; a returned continuation is repository-scoped. Audit evidence is
append-only, redacted, and local. A query is not remote history, a read receipt,
or a response analytics endpoint; it provides no response analytics.

`state.verify` checks an existing database's quick check, foreign keys,
migration, repository scope, and filesystem permissions without creating,
repairing, or migrating it. `lifecycle.inspect` returns bounded repository,
draft, revision, delivery-attempt, inbound, acknowledgement, archive, reply
link, and audit views. It performs no implicit migration, repair, backup upload,
or remote fetch.

A remote snapshot shown by lifecycle inspection is an observation from the
last recorded fetch, not proof of current Discord state. Preserve the boundary
between local evidence and remote truth in every report.

### 11. Apply retention and plan a local purge

Retention uses the configured defaults and bounds in
[`configuration.md`](configuration.md): content defaults to 30 days, metadata
defaults to 365 days, content overrides range from 1 through 365 days, and
metadata overrides range from 30 through 3,650 days, with metadata at least as
long as content. The retention service is transactional and runs before a
state-mutating operation when composed, as well as on an explicit service
invocation. It removes or replaces expired local content with
`[content-expired]` and later removes expired local metadata. A failed sweep
blocks the new mutation with a storage-integrity result. The current handler
surface has no invented `retention.sweep` command; a future composition must
use the implemented service rather than a guessed flag.

For destructive local work:

1. Use `purge.plan` with an explicit repository, `content`/`metadata`/`all`
   scope, and cutoff.
2. Review the exact counts, table counts, state fingerprint, configuration
   hash, plan hash, and `execution_performed: false` value.
3. If the plan is still current, use `purge.execute` with the same scope,
   cutoff, `config_hash`, and `plan_hash` in an interactive TTY.
4. If the plan or configuration changes, discard it and create a new plan.
5. Read the count-only execution result and local audit event ID.

A purge never edits, deletes, reacts to, or otherwise mutates a Discord
message. The plan is local, not a remote deletion request. Purge and retention
are repository-scoped and do not silently cross repository boundaries.

## Accessibility and safe terminal use

The terminal contract is designed for keyboard operation, linear screen-reader
reading, visible focus or selection, and no color-only meaning:

- human output is labeled by field and announces state, destination, revision,
  approval or policy basis, safety state, outcome, and next action in text;
- the renderer supports a minimum of 80 columns and wraps rather than truncating
  security or approval information;
- keyboard prompts support confirmation, cancellation, exact-hash input, and
  invalid-input recovery; no pointer is required;
- non-TTY mode fails closed for approval, policy activation, secret override,
  and purge execution;
- `--color never` and `NO_COLOR` provide plain text, and labels remain
  meaningful when ANSI styling is absent; and
- stdout protocol data and stderr diagnostics remain separate in machine mode.

The documentation contract checks these topics and prohibited claims, but it
does not substitute for a human accessibility or usability review.

## Unsupported behavior and evidence boundary

Repo-com v1 does not provide:

- no read receipts, no proof of a teammate's attention, and no response analytics;
- arbitrary destinations, broadcasts, fan-out, direct messages, or user-token
  authentication;
- Gateway monitoring, a daemon, arbitrary history, or a background scheduler;
- automatic permission changes, automatic unknown-delivery resend, or remote
  edit/delete operations;
- encryption at rest, an OS keychain, encrypted backups, or a guarantee against
  local-account compromise; or
- no claim of live Discord compatibility, human approval, release sign-off, or
  compliance certification from automated tests or this guide.

The current implementation evidence is local, automated, and in part
WireMock-based. A human live round trip and any human rubric or release decision
belong to separate human-review tasks. This guide is evidence for review, not
human approval.

## Sources

Primary sources are `docs/features/release-readiness.md` (`REL-FR-02`,
`REL-FR-03`, `REL-FR-08`), `docs/features/cli-foundation.md` (`FOUND-FR-02`
through `FOUND-FR-06`), the command-handler input/dispatcher modules, the
terminal renderer and prompt contracts, and the linked domain feature documents.
No statement here authorizes a command, remote mutation, human judgment, or
release decision that the implemented interfaces do not own.
