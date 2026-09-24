# Threat model

This document owns the abuse-case and residual-risk view of repo-com v1. It
complements [`security-model.md`](security-model.md), which owns trust-boundary
mechanics. The model is based on the canonical requirements and implemented
contracts; it is not a penetration test, compliance certification, human
approval, or release sign-off.

## Method and evidence boundary

The model treats the local process, repository configuration, operator, Discord
service, and remote message authors as separate trust zones. It distinguishes
an observed local transition from a claim about a person or a current remote
value. Automated WireMock contracts can test request classification and local
safety behavior without proving live Discord behavior.

A documented mitigation is not a guarantee. The residual risks below remain
operator responsibilities unless a later, separately evidenced implementation
changes the contract.

## Threat actors

| Actor | Capability assumed | Primary concern |
|---|---|---|
| Malicious or compromised repository skill | Can submit structured input and request plausible drafts. | Raw destination injection, secret-bearing content, unsafe metadata, or attempting to self-approve. |
| Curious or accidental operator | Can run local commands and change configuration. | Overbroad policy, accidental purge, stale authorization, or treating a preview as current state. |
| Local account attacker | Can read files available to the operator account and inspect process environment. | Reading the bot token, drafts, inbound content, state, and audit evidence. |
| Discord workspace administrator or compromised workspace role | Can change roles, overwrites, channels, or bot membership. | Permission drift, inaccessible channels, mention-target changes, or a compromised bot. |
| Remote message author or bot | Can post text, replies, mentions, edits, and deletion events in a readable channel. | Prompt injection, forged authority, content exfiltration, or misleading a local operator. |
| Discord service or network failure | Can delay, reject, rate-limit, or ambiguously terminate a request. | Duplicate sends, uncertain delivery, unsafe retries, or false absence conclusions. |
| Backup, snapshot, or support recipient | Can receive a copy of local storage outside the live process. | Reading retained content after the original retention period or across accounts. |

## Trust assumptions

1. The operator controls the local account, the token environment, the
   repository, and the decision to invoke a command.
2. A dedicated Discord bot identity is used for all remote requests; a user
   token or self-bot is outside the model and rejected.
3. The local filesystem permission boundary is configured for the current user,
   but the local account, its administrator, backups, and snapshots are not
   assumed trustworthy.
4. Discord is external and can change permissions, content, availability, and
   response behavior. A local report is not a promise about current remote state.
5. Inbound content is untrusted. Its text, mentions, edits, deletions, and
   attachment indicators have no authority by themselves.
6. The secret scanner detects high-confidence patterns only. It is not complete
   data-loss prevention.
7. A human TTY confirmation is meaningful only when the preview and current
   state are revalidated. A confirmation is not a reusable bypass.
8. A human review task, live round trip, and final release decision are separate
   evidence classes. This document records none of them.

## Assets

- the raw dedicated bot token in process environment;
- committed configuration, policy tuples, and destination aliases;
- draft bodies, metadata, revision hashes, and approval records;
- delivery attempts, nonces, message IDs, outcomes, and audit events;
- inbound content, current snapshots, cursors, acknowledgements, archives, and
  reply links;
- local SQLite files, WAL/sidecar files, filesystem permissions, and backups;
- operator decisions, activation hashes, purge plans, and count-only results; and
- the integrity of the exact destination and bot identity used for recovery.

## Abuse cases and mitigations

| Abuse case | Boundary or failure | Implemented mitigation | Residual risk |
|---|---|---|---|
| Skill chooses a raw channel or unlisted mention | Configuration bypass | Strict schema, alias resolution, raw-destination rejection, destination allowlists, and revalidation before send | A compromised local operator can still change files and hashes together |
| Skill tries to approve, activate, or override without a human | Permission widening | TTY-only gates; non-TTY returns `operator-action-required`; no reusable approval bypass | A malicious local process running in the operator's account may impersonate an operator at the OS boundary |
| Draft contains a credential pattern | Accidental exfiltration | Final text/metadata scan blocks by default; exact TTY override is redacted and audited | The scanner is not complete DLP; a secret that does not match its patterns can pass |
| Configuration changes after approval or activation | Stale authority | Canonical config, tuple, destination, revision, and expiry hashes are rechecked | A local attacker with write access can replace both data and process inputs before revalidation |
| Concurrent send for one revision | Duplicate remote message | Atomic local claim, uniqueness boundary, attempt/audit transaction, idempotent duplicate result | Remote ambiguity remains until read-only reconciliation; no cryptographic remote idempotency is claimed |
| Timeout after dispatch | Unknown delivery | Deterministic nonce and exact content; `unknown` state; no automatic resend | Discord can be unavailable or the observation window can remain unresolved |
| Malicious inbound reply or mention | Prompt injection or forged command | Untrusted envelope, no inbound authority, configured alias/boundary, local-only acknowledgement | An operator can still be misled by displayed text; the UI must label it untrusted |
| Remote edit or deletion | Stale local snapshot | Separate first/current/deleted snapshots and point reconciliation | A stored snapshot is not current remote truth; a later fetch may still be incomplete |
| Bot token authentication failure | Credential compromise or expiry | Redacted error, token rotation remediation, bot-only auth, REST v10 pinning | Environment, shell, crash, and backup exposure remain outside token redaction |
| Permission drift or role hierarchy change | Send/fetch denial or wrong mention | Read-only setup check and safe remediation; no automatic grant | Setup is point-in-time evidence; remote state can change immediately afterward |
| Overbroad Discord permissions | Excess remote capability | Least-privilege documented checks and manual-only grants | Discord administrators can grant broader permissions; repo-com does not police every remote role |
| Retention bypass or accidental deletion | Privacy harm | Transactional retention, content-expired marker, bounded local purge plan, exact TTY confirmation, count-only audit | A local account, backup, or snapshot can retain a copy after local deletion |
| State corruption or lock failure | Unsafe mutation or data loss | Read-only verification, forward-only migration, bounded busy timeout, storage-integrity failure | No automatic repair or destructive recreation; operator must choose a recovery plan |
| Diagnostic leakage | Credential/content exposure | Diagnostics off by default, stderr separation, redaction, safe errors | Material copied by an operator or external support system is outside repo-com's control |

## Mitigations and limits

### Authorization and duplicate safety

Exact-revision approval, exact tuple activation, destination revalidation, and
atomic claims reduce the chance that a stale or repeated request creates a
new remote message. These controls are local and fail closed. They do not make
Discord itself idempotent and do not convert an unknown outcome into proof of
absence.

### Untrusted input and redaction

Inbound data is retained as untrusted evidence, not executed as a command.
Audit and diagnostic paths use stable redacted values and do not retain remote
response bodies. The secret scanner is deliberately narrow; operators remain
responsible for reviewing exact outbound text and metadata.

### Local privacy and recovery

Retention, content-expiry replacement, repository-scoped purge, read-only
lifecycle inspection, and redacted count-only audit evidence limit future local
retention. They do not revoke a copy already held by a local account, backup,
snapshot, filesystem provider, or support recipient.

## Residual risk

The following v1 disclosures are affirmative and intentionally not softened:

- local state has **no encryption at rest**;
- protection relies on **user-only filesystem permissions**;
- the product performs **no telemetry** and does not synchronize local audit or
  state;
- a **local-account compromise** can expose the token and retained content;
- local **backups** can expose retained content and metadata; and
- **filesystem snapshots** can capture the same readable data.

A backup or snapshot can outlive a retention sweep or a confirmed local purge.
User-only permissions reduce ordinary cross-user access; they do not protect
against an administrator, malware running as the user, a forensic image, or a
copied backup.

## Unsupported protections

This version does not provide:

- encryption at rest, encrypted backups, or an OS keychain;
- a guarantee against local-account compromise, backup exposure, or snapshot
  exposure;
- complete DLP or a scanner that finds every credential representation;
- arbitrary destination or user-token authorization;
- automatic permission mutation, automatic resend, or remote deletion;
- no read receipts, no response analytics, and no proof that a teammate read a message;
- live Discord compatibility evidence from automated tests alone; or
- no human review result, no compliance certification, and no release approval.

The product intentionally fails closed at important boundaries, but failing
closed does not mean that every environmental threat is eliminated.

## Human and live evidence boundary

A dependent human task may test a disposable workspace, keyboard flows,
security/privacy behavior, and a real teammate reply. Such a task must keep
its evidence separate from this automated model. The presence of a documented
procedure is not a claim that the procedure passed, that live Discord behavior
was observed, or that a release is not approved.

## Sources and review triggers

Primary sources are `docs/PRD.md#10. Security and Privacy`,
`docs/features/release-readiness.md#REL-CON-06`, the Discord delivery,
inbound, state, retention, purge, audit, and approval feature contracts, and
the implemented Rust modules named in those contracts. Revisit this model
before adding encryption, a keychain, remote synchronization, additional
providers, or any capability that changes the trust boundaries.
