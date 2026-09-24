# Project Idea

`repo-com` is a single-user local command-line application that a skill can call from within a repository to exchange important communications with a team. It is a safe transport and workflow layer for agent-originated requests, not a general-purpose team chat client.

The first release makes one workflow excellent: a skill can ask a teammate for attention or a decision through Discord, and the operator can safely approve, send, fetch, and audit the exchange.

## Problem

Agents can discover work, blockers, and decisions that require human attention, but sending a useful message from a skill is currently awkward and unsafe. A raw messaging command can send the wrong content, expose credentials, create duplicates, or leave the operator without a record of what happened.

`repo-com` gives repository skills a predictable communication interface with explicit recipients, reviewable drafts, durable state, safe delivery behavior, and a controlled way to retrieve replies and mentions.

## Users and context

- The primary user is a software operator working with an AI coding skill.
- The recipients are members of the operator's team; they do not need accounts in `repo-com`.
- A repository provides the team identity, channel aliases, destination policies, and approved notification classes.
- The operator remains the owner of local credentials, state, approvals, and retention.

## v1 promise

A skill can create a focused request, the operator can preview and approve the exact revision, Discord can deliver it, and the skill can later fetch replies or mentions without starting a background service. Every meaningful transition is attributable and recoverable.

The primary success signal is a teammate noticing an agent-originated request and replying to it. Reliable, non-duplicative delivery and a complete audit trail are guardrails rather than vanity metrics.

## v1 scope

`repo-com` v1 supports one Discord workspace per repository and one message per draft. It includes:

- A globally installed, versioned `repo-com` binary available on `PATH`.
- Linux, macOS, and Windows support.
- Interactive and non-interactive, non-TTY execution.
- A versioned TOML repository configuration containing non-secret aliases and policies.
- Durable, auditable drafts in user-level application data backed by SQLite.
- Channel-ready text with optional metadata such as severity, event type, and explicit repository context.
- Explicit destination aliases for channels and allowlisted role or user mentions.
- Explicit human preview and approval by default.
- Narrow, operator-controlled automatic sending for pre-approved event type, destination, and severity combinations.
- On-demand retrieval of replies and mentions from explicitly configured inbound channels.
- Local acknowledgement and archival of inbound items.
- Idempotent, concurrency-safe delivery with bounded retries and uncertain-delivery reconciliation.
- A guided, non-mutating Discord setup and validation experience.

Email is a planned follow-up, not a v1 deliverable. The shared message and inbox model should keep room for a second provider, but only the Discord adapter ships initially.

## Core workflow

1. A skill loads the repository configuration and creates a single-destination draft.
2. The operator previews the resolved aliases, exact text, metadata, policy decision, and expiry.
3. The operator explicitly approves an immutable draft revision, or a narrowly configured policy sends it automatically.
4. `repo-com` sends through the dedicated Discord bot, records the delivery attempt, and reconciles ambiguous outcomes before any retry.
5. A later skill invocation fetches unseen replies and mentions on demand.
6. The skill treats inbound content as untrusted data, interprets it explicitly, and may create a validated threaded reply.
7. The operator or skill acknowledges or archives inbound items locally.

## Configuration and state

Repository configuration is committed, versioned, and safe to review. It defines the Discord workspace, channel and recipient aliases, inbound destinations, and narrowly scoped auto-send policies. It never contains the bot token or other secrets.

A skill cannot widen its own auto-send permissions. Configuration changes may be proposed by a skill, but activating a new permission requires an explicit operator action.

Secrets are supplied through the environment and are never persisted by the CLI. Drafts, inbound cursors, acknowledgement state, and delivery history live in a user-level SQLite database keyed to the repository, protected by user-only filesystem permissions. The application does not upload or synchronize this state.

The configuration schema is explicit about supported versions. Unknown or unsafe versions fail closed rather than being silently migrated.

## Safety and delivery rules

- Discord operations use a dedicated bot identity with the minimum required permissions; user-token impersonation is not supported.
- Setup helps an administrator create and validate the bot but does not automatically create applications or change server permissions.
- Approval is bound to the exact text, metadata, destination, and draft revision. Any change invalidates approval.
- Sent messages are immutable. Corrections and follow-ups create new drafts or replies.
- Automatic sends are limited to explicitly configured event type, destination, and severity combinations. All other messages require explicit approval.
- Destination aliases are revalidated during approval and immediately before sending. Arbitrary raw destinations are rejected in normal skill runs.
- Duplicate or concurrent sends for the same draft and revision return the recorded outcome rather than creating another delivery attempt.
- Recognized transient failures receive bounded retries. Validation, authentication, and permission failures are reported without retry.
- A lost or ambiguous Discord response becomes an explicit `unknown` outcome and is reconciled before any resend.
- Accepted, failed, unknown, and replied states are recorded. The product does not claim read receipts or response analytics.
- Inbound replies and mentions are untrusted data. They cannot approve a draft or trigger a send on their own.
- Other bots and `repo-com`'s own messages are ignored by default.
- Edits and deletions preserve the original local audit snapshot and record the current remote state separately.
- Lightweight local checks detect obvious credentials before sending; overrides are explicit.
- Diagnostic logging is opt-in and redacts message content by default.

Message content is retained locally for 30 days by default. Non-content delivery and audit metadata is retained for one year. Retention is configurable per repository, and explicit purge controls are available. Encryption at rest and OS-keychain integration are later options if threat modeling requires them.

A small amount of durable state is intentional: it supports approval, retries, reply correlation, acknowledgement, and auditability. A plain transient send is not the product.

## Message boundaries

- Each draft targets one configured destination; broadcasts and fan-out require separate drafts.
- v1 sends text and optional metadata only. Attachments, embeds, reactions, files, and other rich-message behavior are deferred.
- Direct replies to fetched, valid inbound items are supported. Missing, expired, or unauthorized targets are rejected.
- Scheduled sending, arbitrary templates, full channel-history search, and AI-generated copy are deferred.
- Arbitrary direct messages are not supported in v1.
- Inbound fetching starts from an explicit cursor or time boundary. Historical backfill is a separate, bounded operation.
- Acknowledgement and archival are local-only; `repo-com` does not react to, edit, or otherwise mutate inbound Discord messages.
- Optional repository metadata may identify a repository, branch, or commit, but code contents are never attached automatically.

## Out of scope for v1

- Email delivery and email inbound handling.
- A shared multi-user service, team accounts, roles, or centralized state.
- A live inbox UI, assignment workflow, or team collaboration dashboard.
- A continuously running daemon, bot gateway process, or full channel monitor.
- Arbitrary channel history access or read-receipt analytics.
- Self-updating binaries.
- Automatic Discord application creation or permission mutation.
- Export/import tooling; local inspection, purge, and retention controls are sufficient for v1.

## Open Questions

These questions are intentionally deferred because they require implementation research, a prototype, or a later product decision rather than more abstract discussion:

- Which implementation language and packaging ecosystem should produce the cross-platform binary? The repository has no existing application scaffold to constrain this choice.
- What exact command names, JSON protocol schema, TOML configuration shape, and migration strategy should the first implementation use? The high-level contracts are settled, but their concrete syntax is not.
- What exact Discord permission matrix, API-version policy, rate-limit handling, token-rotation procedure, and administrator onboarding flow should the implementation validate? These should be confirmed against Discord’s current platform behavior and a real workspace.
- What terminal interaction and approval presentation feels clearest to operators? A small runnable prototype of draft creation, preview, approval, send, and inbound fetch should be tested with real users before polishing the interface.
- What authentication, addressing, threading, and inbound semantics should the future email adapter use? Email should be designed only after the Discord workflow validates the shared model.
- Does the threat model later justify encrypted local state or OS-keychain integration beyond the v1 filesystem-permission boundary?
