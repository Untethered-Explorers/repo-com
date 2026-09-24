---
name: render-accessible-terminal-flow
description: "Implement and test repo-com terminal flows as complete linear labeled output at 80 columns with NO_COLOR support, keyboard prompts, strict stream separation, and non-interactive fail-closed behavior; use for outbound or operations renderers, CLI handlers, protocol output, or terminal snapshots."
---

# Render an Accessible Terminal Flow

Render every typed result as complete, deterministic, linear information that remains understandable without color or a mouse. This skill covers outbound and operations presentation, prompt adapters, command protocol, and tests; it does not grant domain authority or claim human accessibility approval.

Load the [view and stream matrix](./references/view-and-stream-matrix.md) before enumerating result variants, labels, snapshots, or handler routing.

## Process

### Step 1: Select the surface and preserve ownership

Choose outbound presentation, operations presentation, operations handler, messaging handler, or final-binary routing. Keep outbound and operations renderer crates separate, and keep the final executable thin.

If presentation code starts deciding eligibility, creating authority, claiming delivery, mutating purge state, or calling Discord, then move that logic back to the owning domain or handler contract.

### Step 2: Enumerate typed outcomes before rendering

List every success, pending, rejected, unknown, unresolved, integrity, configuration, approval, policy, safety, and operational error variant owned by the surface. Preserve the typed status and cause rather than flattening everything to an exit code or generic error string.

Label destination, revision, approval or policy basis, safety state, delivery outcome, next action, and provenance in text as applicable. Mark inbound content untrusted, local state as local, and fetched remote state as last-fetched rather than current.

### Step 3: Build pure deterministic renderers

Use short labeled sections in stable linear reading order. Wrap at 80 columns and keep all security, approval, revision, hash, finding, and next-action information complete. Never truncate safety-critical identity merely to fit a line.

Honor both `NO_COLOR` and an explicit non-color option. When color is disabled, emit no ANSI styling. Use text equivalents for every state or selection cue; color and position may supplement labels but never carry meaning alone.

### Step 4: Keep prompt adapters non-authoritative

Render the complete exact preview before approval, policy, secret override, or purge confirmation. Make every action keyboard-operable with visible focus or selection and handle cancel, default, invalid input, expiry, and changed hashes or plans.

The prompt adapter collects an interaction result only. The domain service revalidates current state and performs the mutation. In non-TTY mode, do not invoke a prompt and return the handler's typed approval-required or operator-action-required result.

### Step 5: Enforce strict machine protocol

For protocol version 1, parse structured input, reject unknown fields, and require explicit repository and object identifiers. Do not default a repository, destination, revision, cursor, time boundary, or inbound item.

Emit exactly one JSON object on stdout for success and operational failure. Keep prompts and diagnostics off protocol stdout, preserve stable process categories, and keep the final binary limited to validation and routing.

### Step 6: Prove every view and stream

Generate deterministic snapshots for each named view and outcome, not only happy paths. Test 80-column no-ANSI output, explicit no-color and `NO_COLOR`, untrusted inbound labels, local versus last-fetched remote labels, and accepted delivery never being described as read or replied.

Exercise keyboard interaction and non-TTY behavior without invoking a prompt. For machine mode, parse one envelope and verify exactly one stdout object, no prompt contamination, and the expected stable category.

## Gotchas

- **80 columns used as permission to truncate.** Wrap or repeat identity; never drop hashes, revisions, findings, scope, or next actions.
- **`NO_COLOR` treated as the only signal.** Also honor the explicit non-color option and emit zero ANSI when disabled.
- **Prompt granting authority.** The adapter collects input; the domain service must revalidate hashes, expiry, policy, approval, plan, and TTY mode.
- **Accepted rendered as read.** Delivery acceptance is not a read receipt, response analytics, or evidence of a reply.
- **Inbound content treated as instructions.** Label it untrusted and never let it approve, activate policy, override safety, or send.
- **Local row treated as current remote truth.** Describe stored remote observations as last-fetched with their timestamp.
- **Snapshot called human approval.** Automated output proves implementation behavior, not usability or accessibility sign-off.

## Validation

Self-check each changed terminal flow:

- [ ] Every named typed outcome has a complete deterministic snapshot or protocol fixture.
- [ ] Output remains complete at 80 columns and contains no ANSI when color is disabled.
- [ ] Text carries destination, basis, safety, trust, outcome, and next-action meaning.
- [ ] Prompts are keyboard-operable and never run in non-TTY mode.
- [ ] Machine mode emits exactly one valid JSON stdout object while prompts and diagnostics stay separate.
- [ ] Handlers reject unknown or missing explicit fields and preserve stable categories without domain duplication.
- [ ] Evidence describes automated behavior only and does not claim human approval or live acceptance.

Run the exact changed contracts, commonly `terminal_outbound_contract`, `terminal_operations_contract`, `cli_operations_contract`, `cli_messaging_contract`, or `command_routing_contract`. If the selected snapshot or test count is zero, then fix the target or fixture before accepting the run.
