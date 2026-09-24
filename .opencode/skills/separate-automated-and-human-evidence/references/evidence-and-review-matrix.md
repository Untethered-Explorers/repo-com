# Evidence and Review Matrix

> Load when: classifying evidence, checking human-review prerequisites, validating a review file, or limiting a completion claim.

## Evidence Classification

| Evidence | Owner | Minimum content | Allowed claim | Forbidden claim |
|---|---|---|---|---|
| Contract and lint output | implementation task | exact command, exit status, selected count | deterministic implementation checks | human approval |
| Terminal snapshots and keyboard tests | implementation task | view, 80-column/no-color case, interaction result | renderer behavior | human usability or screen-reader sign-off |
| Performance samples | quality task | environment, 100 warm no-network samples, p50, p95, max, threshold | measured performance in stated scope | production latency guarantee |
| Mocked E2E | quality task | token-free server, request counts, concurrency, duplicate and no-network proof | mocked journey and safety properties | live Discord or teammate response |
| Documentation contract | technical-writer task | topic, alignment, residual-risk, claim-polarity, secret checks | documentation consistency | human comprehension or approval |
| CI policy and run | release task or CI | policy result or target-specific run identity | defined gate or observed target | unreported target or publication |
| Release artifact | release run | target, version, checksum, license, SBOM, patched SQLite evidence | properties of that artifact | unreviewed source or public release |
| Human review record | designated human task | task-specific fields below | recorded human decision only | automated substitution |

## Human Review Files

| Task | Human-only file | Required evidence |
|---|---|---|
| `REL-UX-HR-1` | `docs/reviews/terminal-ux.json` | reviewer, environment, commands, observations, defects, 0–4 UX/accessibility scores, blockers |
| `REL-SEC-HR-1` | `docs/reviews/security-privacy.json` | reviewer, reviewed scope, defects, blockers, 0–4 security/privacy/residual-risk scores |
| `REL-LIVE-HR-1` | `docs/reviews/discord-live-acceptance.json` | disposable workspace and bot, teammate participation, timestamps, redacted IDs, send/reply/ack/audit/purge observations, unknown recovery, token rotation or revocation, pass/fail |
| `REL-SIGN-HR-1` | `docs/reviews/release-signoff.json` | named reviewers, evidence hashes, decision, conditions, date |

No implementation agent may create or populate these files. A missing or incomplete human record remains pending or produces the explicit human decision; it is never an inferred approval.

## Dependency Sequence

- UX review follows terminal UI, final app routing, performance, documentation, and mocked E2E.
- Security review follows CI, packaging, documentation, and mocked E2E.
- Live Discord acceptance follows UX, security, CI, packaging, and documentation.
- Final sign-off follows all three reviews plus the complete automated evidence inventory.

## Secret Boundary

Scan implementation evidence and human-review inputs for:

- bot token values;
- authorization headers or values;
- private keys;
- real team-message content;
- live sensitive workspace data that is not explicitly redacted.

Use synthetic fixtures and secret-safe identifiers. A report may name the evidence category and location without reproducing the value.

## Final Decision Rules

Final sign-off must choose one explicit decision:

- approve;
- approve with conditions;
- reject.

Missing required evidence, an absent reviewer, or an unresolved critical blocker is pending or reject, not approval. An engine state file, expected-output existence, zero-exit model response, or generic attestation does not replace the task-specific human record.
