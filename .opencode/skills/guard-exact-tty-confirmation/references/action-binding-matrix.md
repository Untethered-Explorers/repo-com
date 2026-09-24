# Action Binding Matrix

> Load when: selecting preview fields, hash inputs, non-TTY behavior, audit evidence, or final revalidation for an operator-only action.

## Action Matrix

| Action | Required TTY mode | Exact preview and binding | Non-TTY result | Final revalidation |
|---|---|---|---|---|
| Policy activation or widening | Yes | event type, destination alias, severity, affected policy, canonical relevant configuration hash, policy tuple hash, activation time | handler's `approval-required` or `operator-action-required` outcome | current normalized configuration and exact tuple |
| Draft approval | Yes | repository, revision, exact rendered text, metadata, resolved destination and mentions, expiry, current approval or policy basis | handler's typed outcome; existing valid authority may be consumed elsewhere | repository, revision, configuration, destination, mentions, expiry, policy or approval, safety state |
| Secret override | Yes | repository, exact revision, redacted finding category and reason, current scan result | fail closed; no override is created | current revision and identical finding state |
| Purge execution | Yes | repository, scope, cutoff, exact category and row counts, configuration hash, deterministic plan hash | fail closed; plan generation may be non-mutating but execution is not | current repository, scope, cutoff, configuration, plan, and counts |
| Eligibility evaluation | No creation prompt | current hashes and all required current-state inputs | may consume existing valid authority | required again inside delivery claim |
| Policy inspection | No | read-only status | allowed | not applicable |
| Permission-reducing deactivation | No creation prompt | exact current policy status | allowed if not widening permission | current policy identity and repository |

Use the owning command contract's exact typed category. Do not invent a new protocol status or numeric exit code.

## Hash Profiles

- Policy tuple hash covers the exact event type, destination alias, severity, and affected policy.
- Policy validity also binds the complete relevant normalized configuration so unrelated-in-practice retention or alias changes can stale it.
- Draft revision hash covers canonical body, normalized metadata, repository, destination alias, resolved channel and mentions, and expiry.
- Approval binds the exact repository, revision, relevant configuration, resolved destination, policy or approval basis, and expiry.
- Purge binds repository, scope, cutoff, configuration hash, deterministic plan hash, and exact counts.

Canonical ordering must remain stable. Equivalent normalized ordering must not change a hash; an approval-bound value change must invalidate authority.

## Preview and Prompt Checks

- Complete at 80 columns by wrapping or repeating identity.
- No ANSI when color is disabled.
- No color-only meaning.
- Keyboard reachable without a mouse.
- Visible focus or selection.
- Explicit cancel, default, invalid, expiry, and changed-plan/hash cases.
- Prompt output stays on its designated interactive stream.

## Audit Checks

Record only redacted action evidence:

- repository and action identity;
- object, revision, or plan identity;
- hashes and timestamps;
- typed outcome and reason category;
- count-only purge results;
- no token, matched secret, authorization value, or raw message content.

Repeat the same exact action and confirm it does not create a second authority record or duplicate audit event.
