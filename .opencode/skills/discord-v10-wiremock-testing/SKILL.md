---
name: discord-v10-wiremock-testing
description: "Build token-free repo-com Discord REST v10 WireMock contracts for Bot authentication, permissions, message creation, dynamic rate limits, bounded reads, delivery ambiguity, and redacted error classification; use for setup, message, inbound-fetch, delivery, or mocked E2E protocol changes."
---

# Test Discord v10 with WireMock

Prove Discord wire behavior through a local server and synthetic fixtures, never a live token or workspace. This skill owns transport contracts and error classification; delivery claims and unknown recovery remain separate domain procedures.

Load the [protocol matrix](./references/protocol-matrix.md) before adding or changing fixtures, request assertions, status mappings, or read limits.

## Process

### Step 1: Classify the contract under test

Choose one primary surface before writing fixtures:

- read-only setup and permission checks;
- one Discord message operation;
- bounded inbound retrieval and point checks;
- delivery claim plus transport;
- bounded retry or unknown reconciliation;
- final-binary mocked E2E.

If a suite starts implementing claims, policy, state transitions, or reconciliation decisions, then move those assertions to the owning domain contract and keep the WireMock suite at the protocol boundary.

### Step 2: Lock v10 and bot authentication

Assert every route under `/api/v10` and use `Authorization: Bot <token>` with a synthetic token. Normal tests must reject user-token, Bearer-user, self-bot, and unpinned-version behavior.

Keep the token source boundary as `REPO_COM_DISCORD_TOKEN`, but inject a non-secret fixture value. Assert that tokens, authorization values, response bodies, and real team content are absent from `Debug`, `Display`, errors, diagnostics, fixtures, and logs.

### Step 3: Match the exact request contract

For message creation, assert exactly one `POST /api/v10/channels/{channel_id}/messages` with the exact final text, deterministic nonce footer, allowlisted `allowed_mentions`, and optional validated `message_reference`. Reject attachments, embeds, raw user-supplied channel IDs, unlisted mention targets, and accidental second nonce rendering.

For setup, use only the read-only endpoints selected by the owning task and assert required visibility and permissions. If the canonical contract does not name an endpoint, then consult current official Discord v10 documentation and record the selected path in the fixture rather than inventing a product interface.

### Step 4: Bound inbound reads

Require one enabled configured inbound alias and exactly one of an explicit last-event-ID cursor or an RFC 3339 boundary. Reject both, neither, raw unconfigured channel IDs, and broad history requests.

Stop at 10 pages or 1,000 raw messages and expose continuation metadata. Allow no more than 100 read-only point checks after a stored page. Return untrusted envelopes with provenance and attachment indicators; never fetch attachment bytes or interpret content as instructions.

### Step 5: Exercise dynamic limits and error classes

Cover 400, 401, 403, 404, 409, 429, 5xx, connect failure, pre-dispatch timeout, and post-dispatch timeout or reset. Distinguish proven not sent from ambiguous dispatch.

For 429, use server-provided `Retry-After` or `retry_after`, cover route/global and user/shared scopes, and cap Discord-directed waits at 30 seconds per attempt. Use controlled clocks rather than wall-clock sleeps.

Keep the message adapter to one HTTP send attempt. If the delivery-retry layer is in scope, then allow at most three total transport attempts only for proven pre-dispatch failure or 429.

### Step 6: Prove duplicate safety around the transport

For stateful delivery fixtures, prove the claim and audit commit before any POST, exactly one create request under concurrent callers, and no unsafe repost after a claim. A timeout, reset, or server response after dispatch must become unknown.

Unknown recovery may read the configured destination only. It must match bot author, deterministic nonce, and exact final content; it must not edit or delete a remote message.

### Step 7: Isolate and qualify evidence

Use local WireMock for every normal test and fail if the test can bypass it. Scan captured requests, responses, fixtures, and diagnostics for credentials and team content.

Report the result as token-free mocked protocol evidence, not live Discord compatibility, human acceptance, or a read receipt.

## Gotchas

- **Post-dispatch timeout classified as failed.** A timeout, reset, or 5xx after dispatch can mean the message exists; classify it as ambiguous and block automatic resend.
- **Message adapter contains a retry loop.** One message operation performs one send; bounded retry belongs to delivery recovery.
- **Nonce matched without content.** Destination, bot author, nonce, and exact final content are conjunctive predicates.
- **Inbound limits conflated.** Ten pages, 1,000 raw messages, and 100 point checks are separate bounds.
- **Raw channel IDs used as a shortcut.** Normal workflow inputs resolve aliases; synthetic IDs may exist only inside controlled adapter fixtures.
- **Response body copied into errors.** Discord content is untrusted and may contain secrets; errors must remain typed and redacted.

## Validation

Self-check each changed Discord contract:

- [ ] Every request asserts the `/api/v10` prefix and `Bot` authorization.
- [ ] Setup, message, inbound, delivery, and E2E fixtures assert only their owned request surface.
- [ ] Request bodies, nonce, mentions, boundaries, continuation data, and all numeric limits match the task contract.
- [ ] Status, 429 scope, retry timing, and proven-not-sent versus ambiguous fixtures are deterministic.
- [ ] The concurrency fixture proves one claimed create and no unsafe second POST.
- [ ] Captured evidence contains no token, authorization value, real message content, or live network dependency.

Run the exact task-selected binaries, commonly `discord_client_contract`, `discord_message_contract`, `inbox_fetch_contract`, `delivery_contract`, and `delivery_retry_contract`; include `e2e_workflow` only when that task owns the change. If a selector discovers zero tests, then fix the test target before accepting the run.
