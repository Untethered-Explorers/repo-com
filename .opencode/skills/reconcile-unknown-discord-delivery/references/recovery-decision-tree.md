# Recovery Decision Tree

> Load when: classifying unknown-delivery evidence, deciding absence, or separating recovery from a later authorized attempt.

## Entry Evidence

A recovery attempt must already have a durable local claim and unknown state. Preserve:

- repository identity;
- configured destination;
- configured bot author;
- deterministic nonce;
- exact final rendered content, including nonce footer;
- dispatch boundary and redacted error class;
- prior local attempts and audit state.

Reconciliation does not recreate the claim, send a message, edit or delete content, or create authorization for a new attempt.

## Decision Tree

```text
unknown claim
  -> read configured destination only
  -> require every observed candidate to match:
       channel + bot author + nonce + exact final content
  -> exactly one exact match and no conflict
       -> accepted under the owning task's exact state label
  -> multiple exact matches
       -> unresolved
  -> same destination/bot/nonce with edited or conflicting content
       -> unresolved
  -> deleted or tombstoned candidate evidence
       -> unresolved unless the owning contract defines a narrower mapping
  -> no candidate
       -> observation time at least five minutes?
            no  -> remain unknown
            yes -> successful complete reads at least three?
                     no  -> remain unknown
                     yes -> reconciled_absent
  -> unauthorized, incomplete, or insufficient read
       -> preserve blocker; do not advance to reconciled_absent
```

Conflict evidence takes precedence over elapsed time. Never count an incomplete, unauthorized, or rate-limit-rejected read as a successful complete observation.

## Matching Rules

A match requires all four predicates:

1. exact configured destination;
2. exact configured bot author;
3. exact deterministic nonce;
4. exact intended final content.

Use already-rendered content. Do not re-normalize, re-render, trim, or append another nonce during recovery.

## Absence and Limits

- Minimum observation window: five minutes.
- Minimum successful complete reads: three.
- Reconciliation performs no remote mutation.
- Normal transport retry remains bounded to at most three total attempts and a 30-second Discord-directed wait cap.
- Inbound limits such as 10 pages, 1,000 raw messages, and 100 point checks are not imported into outbound recovery without an explicit requirement.

The canonical plan does not specify a polling interval, maximum recovery reads, search-page bound, or whether waits consume the observation window. Do not present an invented value as a product guarantee; bound the implementation and preserve the explicit two-part absence gate.

## State Naming

Some source wording distinguishes `accepted` from `reconciled_accepted`. Use the owning implementation type and preserve one documented mapping. Do not introduce both as independent states or treat either as read, replied, or live-human acceptance.

## New Attempt Boundary

A later new attempt requires:

- explicit operator authorization outside reconciliation;
- current eligibility;
- current repository, revision, configuration, destination, policy or approval, and secret decision;
- a new valid uniqueness and claim decision;
- the normal single-send message path.

If any value changed or authorization is absent, then remain blocked.
