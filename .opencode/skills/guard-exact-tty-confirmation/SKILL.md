---
name: guard-exact-tty-confirmation
description: "Add repo-com operator confirmation bound to the exact repository, object, revision or plan, scope, hashes, and current state while failing closed in non-TTY mode; use for policy activation, draft approval, secret override, eligibility authority, or confirmed purge."
---

# Guard Exact TTY Confirmation

Turn an operator-only action into a current-state, exact-object authorization rather than a generic yes/no prompt. This skill covers confirmation boundaries and revalidation; it does not define rendering style, state transactions, or Discord delivery.

Load the [action binding matrix](./references/action-binding-matrix.md) before choosing preview fields, hashes, typed non-TTY results, or audit evidence.

## Process

### Step 1: Classify authority and permission change

Resolve the exact action before prompting:

- policy activation or widening;
- draft approval;
- secret-finding override;
- purge execution;
- non-authorizing eligibility, policy inspection, or permission-reducing deactivation.

If the action is read-only or permission-reducing, then do not add an approval prompt merely for consistency. If it creates authority, widens permission, overrides safety, or executes a destructive plan, then require exact interactive confirmation.

### Step 2: Resolve explicit TTY mode

Require the caller to pass explicit `TtyMode`; domain code must not probe the terminal or infer authority from ambient process state. In non-TTY mode, do not invoke a prompt and do not create approval, policy activation, secret override, or purge authority.

Return the owning handler's typed `approval-required` or `operator-action-required` outcome. A non-TTY caller may consume an existing valid approval or policy, but it cannot create one.

### Step 3: Build the complete exact preview

Render all fields needed to identify and judge the action before asking for input. Include repository, object or plan, revision, resolved destination, scope, expiry, policy or approval basis, safety finding, current status, hashes or exact counts, and next action as applicable.

At 80 columns, wrap and repeat identity rather than truncate a hash, revision, destination, finding, or plan count. Text and labels must carry the decision; color and cursor position may supplement but never replace them.

### Step 4: Recompute current state immediately before confirmation

Reload the current configuration, repository, object, destination, expiry, policy, scan result, or deterministic plan. Recompute every action-specific canonical hash and reject any mismatch or cross-repository object.

If state changed after the preview, then invalidate the interaction and render a fresh preview. Never silently refresh the preview, broaden scope, or accept the old confirmation against new state.

### Step 5: Collect a keyboard-operable exact confirmation

Make the prompt reachable and operable without a mouse, with visible focus or selection. Handle cancel, invalid input, expiry, and changed hashes or plans. The prompt adapter returns an interaction result only; it must not mutate policy, approval, safety, or purge state.

Use the task's exact confirmation vocabulary. Do not add a product-wide `--yes` bypass because Forge's build flags are unrelated to repo-com operator authority.

### Step 6: Persist authority atomically and idempotently

After the owning domain service accepts the exact interaction, record the action-specific binding and timestamp in the same transaction as its redacted audit event. Repeating the same exact confirmation must be idempotent and must not create duplicate authority or audit spam.

A secret override records a redacted reason code and current finding state, never the matched secret value. An approval or policy is not a reusable bypass: final eligibility and delivery claim must revalidate current state again.

### Step 7: Revalidate at the action boundary

Immediately before the protected mutation or claim, repeat the same repository, revision, configuration, destination, expiry, policy, finding, plan, and hash checks. If any value differs from the persisted authority, then invalidate it and require a new confirmation.

## Gotchas

- **Policy activation collapsed into draft approval.** They have different tuple, configuration, preview, and owner contracts; preserve both bindings.
- **Approval expiry fixed at fifteen minutes.** Use the earlier of draft expiry and the task's approval limit, then recheck current expiry before confirmation and claim.
- **Prompt treated as authority.** A UI can collect input, but only the domain service may revalidate and persist approval, policy, override, or purge execution.
- **Non-TTY generic yes bypass.** Permission-widening and destructive actions fail closed in automation; consuming pre-existing authority is a different path.
- **Purge plan reused after change.** Repository, scope, cutoff, configuration hash, plan hash, or exact counts must match; otherwise replan and reconfirm.
- **Secret value copied into evidence.** Record the finding category and redacted reason only.

## Validation

Self-check the confirmation path:

- [ ] Every authority-creating action requires explicit TTY mode and fails closed otherwise.
- [ ] The complete preview contains every identity, scope, current-state, hash or count, and next-action field.
- [ ] Any repository, revision, destination, expiry, policy, finding, or plan change invalidates the old interaction.
- [ ] Keyboard cancel, invalid input, visible focus, and no-color behavior are deterministic.
- [ ] Domain state and redacted audit evidence commit together and remain idempotent on exact replay.
- [ ] Final eligibility or execution revalidates the persisted binding and creates no generic bypass.

Run the exact contract for the changed action, commonly `policy_contract`, `approval_contract`, `send_eligibility_contract`, `purge_contract`, or the relevant terminal and handler binary. If no tests are selected, then fix the test target before accepting the result.
