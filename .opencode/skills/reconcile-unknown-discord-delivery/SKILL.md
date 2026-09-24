---
name: reconcile-unknown-discord-delivery
description: "Recover an ambiguous repo-com Discord send with read-only exact destination, bot-author, nonce, and content reconciliation before any explicitly authorized new attempt; use for unknown delivery state, duplicate prevention, edited or deleted evidence, or conservative absence proof."
---

# Reconcile Unknown Discord Delivery

Recover an ambiguous send by reading exact remote evidence without creating a duplicate. This skill starts only after a local attempt is durably `unknown`; it does not create the claim, retry transport, fetch arbitrary history, or silently authorize another POST.

Load the [recovery decision tree](./references/recovery-decision-tree.md) before classifying evidence or changing an unknown attempt.

## Process

### Step 1: Verify the recovery entry condition

Load the attempt's repository, configured destination, configured bot identity, deterministic nonce, exact intended final content, existing state, and redacted dispatch error. Confirm the attempt is already claimed and in the task's unknown state.

If no durable claim exists, then the send path was not used correctly; stop and follow the owning delivery contract. If the attempt is already terminal, then do not reopen it through reconciliation.

### Step 2: Keep recovery read-only

Read only the configured destination with the required message-history permission. Bound the operation by the owning task and server-directed rate limits. Do not edit, delete, react to, repost, search arbitrary channels, use a Gateway, or mutate Discord state.

If a read is unauthorized, incomplete, rate-limited beyond the permitted bound, or otherwise insufficient, then preserve the blocker; failed evidence collection is not proof of absence.

### Step 3: Evaluate every candidate conjunctively

For each observed message require all four predicates:

1. channel equals the configured destination;
2. author equals the configured bot identity;
3. nonce equals the attempt's deterministic nonce;
4. content equals the exact final text sent, including the nonce footer.

Classify the evidence:

- exactly one exact match and no conflict: accepted under the owning type's label;
- multiple exact matches: unresolved;
- same destination, bot, and nonce but edited or conflicting content: unresolved;
- deleted or tombstoned candidate evidence: unresolved unless the task defines a narrower exact mapping;
- no candidate: continue observation without changing to absent.

Never normalize or re-render content during recovery; compare the bytes or canonical text of the already-rendered attempt.

### Step 4: Require conservative absence evidence

Advance to reconciled absence only after both conditions hold:

- at least five minutes of observation have elapsed;
- at least three successful, complete reads have found no exact match.

If either condition is missing, then remain unknown. If evidence conflicts or is incomplete, then remain unresolved regardless of elapsed time or read count.

Do not invent a polling cadence, search bound, or observation-clock origin absent from the task contract. Use dynamic Discord limits and a bounded implementation, and record the actual successful-read count and observation timestamps.

### Step 5: Persist the transition and evidence

Use the delivery-retry contract to store the terminal decision and matching redacted audit event atomically. Preserve the attempt's local evidence and any known Discord message ID without storing raw unrelated content.

If source wording uses `accepted` or `reconciled_accepted`, then use the owning task's exact type label and one explicit mapping; do not create two semantic accepted states.

### Step 6: Require a separate authorization boundary for a new attempt

Unknown, unresolved, and reconciled absence do not authorize automatic resend. A later new attempt is a separate command and must obtain explicit operator authorization, then pass current eligibility, revision, destination, uniqueness, and claim rules.

If that authorization is absent or any current value changed, then stop. Never reuse reconciliation to manufacture a send permission.

## Gotchas

- **Unknown treated as failed.** A post-dispatch timeout, reset, or 5xx can mean the message exists.
- **Nonce-only match.** Destination, bot author, nonce, and exact content are all required.
- **Re-rendering during comparison.** Whitespace, normalization, or a second nonce pass can invalidate an otherwise exact match.
- **Edited or deleted evidence treated as absence.** Conflict or tombstone evidence remains blocked unless the task defines an exact transition.
- **Three reads treated as a maximum.** The requirement is at least three successful reads plus at least five minutes; insufficient evidence never proves absence.
- **Recovery POSTs.** Reconciliation is read-only. Any new attempt is a separate explicit authorization path.
- **Local accepted rendered as read.** A matching message does not prove a teammate read or replied.

## Validation

Self-check the recovery path before handoff:

- [ ] Entry requires a durable unknown claim and preserves the exact intended final content.
- [ ] Every fixture asserts configured destination, configured bot author, deterministic nonce, and exact content.
- [ ] Zero, one, multiple, edited, deleted, unauthorized, incomplete, and rate-limited evidence are distinguished.
- [ ] Reconciled absence requires both five minutes and three successful complete reads.
- [ ] Conflicts remain unresolved and incomplete reads cannot advance absence.
- [ ] Recovery performs no remote mutation and cannot bypass separate new-attempt authorization.
- [ ] WireMock or injected transport evidence is labeled mocked and contains no token or real team content.

Run the exact delivery-recovery contract, normally `delivery_retry_contract`, plus `discord_message_contract` and `delivery_contract` when their boundaries changed. If the selected test count is zero, then fix the target before accepting the run.
