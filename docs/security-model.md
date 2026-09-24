# Security model

This document owns the trust boundaries and security behavior for the local
repo-com workflow. It is an analysis of the implemented contracts, not a
security certification or a human sign-off. The primary requirements are
`RC-SEC-01` through `RC-SEC-09`, `RC-PRIV-01` and `RC-PRIV-02` in
`docs/PRD.md#10`, together with the delivery, inbound, state, and privacy
feature contracts.

## Scope and security objectives

The security objective is to keep a repository-scoped, local communication
workflow explicit: a dedicated bot is the only remote identity; an exact draft
revision and destination are bound to an approval or exact activated policy;
local state and audit evidence are bounded and redacted; and an ambiguous
remote result cannot silently create a duplicate message.

This model does not claim complete data-loss prevention, protection from a
compromised local account, encryption, secure deletion against every storage
medium, or proof of a teammate's attention.

## Actors and assets

| Actor or asset | Trust level and security relevance |
|---|---|
| Repository skill | Semi-trusted producer of structured draft requests. It may request work but cannot approve, activate policy, override safety, or choose a raw destination. |
| Operator | The human who controls local files, the environment token, TTY confirmations, policy activation, and purge confirmation. |
| Dedicated Discord bot | Remote identity used for bot-only REST v10 authentication. It is not the operator and cannot approve local state by itself. |
| Discord workspace and channels | External system outside the local trust boundary. Availability, permissions, edits, deletions, and current contents are not assumed. |
| Inbound author or bot | Untrusted remote data. Message text, mentions, edits, deletions, and attachment indicators never grant authority. |
| Local configuration | Committed non-secret policy and routing data subject to strict schema and alias validation. |
| Local SQLite state | Confidential local repository-scoped drafts, content, cursors, acknowledgements, delivery history, policy activations, and audit evidence. |
| Environment and process | Holds the raw bot token for the process lifetime and determines TTY behavior. A local process with sufficient account access may read state or the environment. |
| Audit evidence | Append-only local transition facts with redacted metadata; useful for recovery and review, not a remote read receipt. |

## Trust boundaries

```text
repository skill input
        │ strict protocol and identifier validation
        ▼
local command handlers ── config, hashes, current-state checks ──► local SQLite
        │                                                        (user-only boundary)
        │ TTY-only approval / activation / secret override
        ▼
delivery eligibility ── atomic local claim and audit ──► one-attempt transport
        │                                                        │
        │ dedicated bot token                                   │ Discord REST v10
        ▼                                                        ▼
local exact output ◄──── typed accepted/failed/unknown/reconciled result
        │
        ├── bounded read-only inbound fetch ──► untrusted snapshots + cursor
        │
        └── local audit/lifecycle/retention/purge ──► local evidence only
```

The boundary between local and remote is explicit. A local `accepted` outcome
does not establish that a person read the message, and a local snapshot is not
proof of current remote state. A later fetch is a new observation.

## Bot-only authentication and token flow

The Discord adapter is **bot-only**. It reads the raw credential from
`REPO_COM_DISCORD_TOKEN`, sends it using the Discord Bot authorization scheme,
and pins routes to REST API v10. It rejects user-token authentication, Bearer
user authentication, self-bots, and unpinned endpoints. The token is not a TOML
field, SQLite field, draft field, protocol data field, or audit value.

The environment variable is a process boundary, not a vault. The client
redacts owned copies in diagnostics and zeroes owned memory where the
implementation provides that behavior. Environment inspection, shell history,
a crash dump, a backup, or a process listing may still expose a token outside
repo-com's control. Rotate the dedicated bot token after any suspected
exposure; do not paste the value into a ticket or documentation.

A 401 produces a safe authentication remediation. A setup check never changes
Discord, and a token rotation never changes the local authorization model.

## Configuration and input boundary

Configuration is parsed as schema version 1 with strict unknown-field
rejection. Secret-like fields, raw destinations, invalid aliases, duplicate
aliases, invalid mention prefixes, unsafe schema versions, and invalid policy
tuples fail closed. The local configuration crate does not call Discord; a
cross-workspace reference is rejected when a separately populated
`WorkspaceReferenceIndex` is supplied, and remote membership/visibility is
checked by `setup-check`. Mention targets use portable local identifiers until
the Discord adapter verifies the remote target. Destination aliases are
resolved again at approval and immediately before every send attempt. A raw
Discord channel, role, user, or message ID cannot be smuggled through a normal
skill-facing command.

The foundation's protocol version is 1. The outer command envelope and each
command input reject unknown fields. The handler crates validate identifiers,
timestamps, boundaries, page sizes, hashes, and mutually exclusive fields
before calling a domain service. Validation is not authorization: a later
domain owner must re-evaluate current state.

## Exact authorization and permission-widening actions

A draft revision is eligible only when it has an unexpired exact-revision human
approval or an unexpired exact activated policy and no unresolved safety
finding. The eligibility decision carries repository, revision, configuration,
destination, approval/policy, and scan hashes for revalidation immediately
before the claim. A non-TTY invocation may use an already-valid authority but
cannot create one.

Human approval is bound to the exact repository and revision hash, including
body, normalized metadata, destination alias, resolved destination, and expiry.
It expires at the earlier of draft expiry or 15 minutes after approval and is
invalidated by a changed configuration, destination, revision, or policy basis.
Approval is consumed idempotently for one claim; it is not a reusable bypass.

An auto-send policy is an exact `(event_type, destination alias, severity)`
tuple. There is no wildcard, prefix, broader severity, or implicit default.
Activation is a separate interactive TTY action bound to canonical
configuration and tuple hashes. A changed hash makes the activation stale.

A secret finding blocks send by default. Only an interactive TTY operator may
override the exact revision after reviewing the preview; the audit event stores
a redacted reason code and never the matched value. The scanner is a
high-confidence safety layer, not complete DLP.

## Untrusted inbound data

Discord messages, edits, deletions, mentions, authors, and attachment
indicators cross an untrusted boundary. The inbound contract retains structured
facts but treats their text as data. No inbound field can:

- approve a draft or activate a policy;
- override a secret finding;
- authorize a send by itself;
- change a configured destination; or
- turn a local acknowledgement into a remote mutation.

Bounded fetch requires an enabled inbound alias and exactly one cursor or time
boundary, with hard 10-page/1,000-raw-message limits. The first observed
snapshot, later current snapshot or deletion marker, and local lifecycle events
are separate. Acknowledgement, archival, cursor movement, and reply links are
local-only. A stored inbound snapshot is not current remote content.

A reply draft must resolve an authorized stored inbound item and a validated
message reference. It then passes the same approval, policy, safety, claim,
and delivery gates as any other draft. Creating a reply draft does not send it.

## Delivery, duplicate prevention, and recovery

The local claim, authorization revalidation, attempt creation, and matching
audit transition commit atomically before network I/O. A uniqueness boundary
prevents concurrent or repeated invocations for one revision from both
reaching a new POST. Existing accepted, failed, retry-wait, unknown, or
reconciled outcomes are returned to duplicate callers rather than replaced.

An ambiguous pre/post-dispatch result is `unknown`. The read-only reconciler
uses the exact configured destination, bot author, deterministic nonce, and
intended content. A single exact match becomes reconciled acceptance; an
incomplete read or unproven observation window remains unknown; conservative
absence evidence may become reconciled absence; conflicting evidence remains
unresolved. An unknown or unresolved delivery is never automatically resent.
Any later new attempt requires a fresh, explicit operator-authorized decision
after reconciliation.

## Redaction and diagnostic boundary

Audit events contain repository/object identifiers, transition, timestamp, actor
kind, outcome, and redacted metadata. Diagnostics are off by default and, when
enabled, belong on stderr. Token values, authorization values, private-key
material, message content, and secret-like values are not safe diagnostic
content. The scanner reports a stable reason and redacted location rather than
returning a matched secret.

Redaction is applied to local audit and supported diagnostics; it cannot erase
a secret that an operator already copied into an external system. Avoid
putting content, credentials, or response bodies in support tickets and logs.

## Local state and residual risk

The default local database layout is under the platform's user-data location
as `repo-com/state.sqlite3`. The state boundary is repository-keyed and
user-only: Unix file creation uses user-only mode and Windows retains the
inherited user-profile ACL. This is a filesystem permission boundary, not a
cryptographic boundary.

The following disclosures are mandatory for v1:

- v1 relies on **user-only filesystem permissions**;
- v1 has **no encryption at rest** for local state;
- v1 performs **no telemetry**, analytics, crash upload, or remote audit
  synchronization;
- a **local-account compromise** can read retained content and configuration;
- local **backups** can retain readable content and metadata; and
- **filesystem snapshots** can capture readable local state.

Retention and purge reduce future local retention but do not revoke copies
already held by a local account, backup, snapshot, filesystem, or other
operator-controlled copy. Purge is local only and never deletes a Discord
message.

See [`threat-model.md`](threat-model.md) for abuse cases and the full residual
risk register. Neither document is a compliance certification, security
approval, or release decision.

## Unsupported security behavior

Repo-com does not provide encryption at rest, an OS keychain, remote state
synchronization, complete DLP, arbitrary destination authorization, user-token
support, self-bot support, automatic Discord permission changes, automatic
resend of an unknown delivery, or a guarantee that stored data is absent from
backups and snapshots. It does not claim live Discord compatibility or a
human-approved release.

## Sources and limitations

Authoritative sources are `docs/PRD.md#10. Security and Privacy`,
`docs/features/draft-and-approval-workflow.md`,
`docs/features/discord-delivery-and-reconciliation.md`,
`docs/features/inbound-retrieval-and-reply.md`,
`docs/features/privacy-and-lifecycle-operations.md`, and the implemented
configuration, approval, eligibility, delivery, inbound, audit, state, and
redaction crates. The contract test checks documentation topics and negative
claim polarity; it does not replace human security review.
