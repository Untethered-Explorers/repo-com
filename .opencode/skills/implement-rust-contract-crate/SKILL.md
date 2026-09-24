---
name: implement-rust-contract-crate
description: "Implement repo-com Rust 2024 implementation tasks with exact declared outputs, workspace integration, named contract-test binaries, and fail-on-zero-tests validation; use when creating or changing a task-owned crate, binary, fixture, workflow, or documentation contract."
---

# Implement a repo-com Rust Contract

Implement one exact `forge-task` without changing its owner, outputs, dependencies, or acceptance boundary. This skill applies to Rust implementation tasks, including the documented exceptions; it does not replace a task contract or run human-review tasks.

Load the [task and test map](./references/task-and-test-map.md) before editing so the package, test target, and exception shape are selected from the owning contract.

## Process

### Step 1: Resolve the owning task

Read the complete task block and record its ID, owner, dependencies, expected outputs, validation commands, requirement and constraint references, acceptance criteria, and exclusions. If the task is `human-review`, then stop before creating implementation outputs or running Cargo checks; the designated human task owns its review file.

If the current task is not the named owner, then do not broaden the implementation into its work.

### Step 2: Classify the artifact shape

Choose exactly one shape from the task's declared outputs:

- a new focused library crate;
- the final binary or an extension to its existing tests;
- a crate with extra configuration, workflow, or documentation outputs;
- a human-review task with no implementation output.

If a task adds a new major dependency or changes the pinned foundation, then stop and surface the contract gap instead of silently changing the workspace.

### Step 3: Scaffold only declared outputs

Create the exact paths named by the task. For ordinary library tasks, use `crates/repo-com-<domain>/Cargo.toml`, `src/lib.rs`, and the named behavior modules. Register workspace dependencies centrally and preserve Rust 1.98.1, edition 2024, and the committed `Cargo.lock` unless the foundation task authorizes a change.

Keep the smallest domain boundary owned by the task. Do not add convenience wrappers, alternate state stores, remote clients, prompts, or files that merely seem reusable.

### Step 4: Implement acceptance behavior

Turn each acceptance criterion into at least one observable contract assertion. Preserve the task exclusions: pure crates stay I/O-free, state changes stay local and repository-scoped, Discord uses a dedicated bot token and REST v10, machine protocol stdout stays singular, and a prompt adapter never creates authority.

Keep external time, randomness, terminal, filesystem, and network boundaries injectable when acceptance tests must be deterministic.

### Step 5: Build the named contract test

Use the exact test filename and preserve its Cargo test target stem. Test boundaries, failure paths, rollback, and no-side-effect cases rather than only successful construction. For an existing binary such as the E2E journey, extend its existing test tree instead of creating a new crate.

Add `--no-tests fail` to every named nextest selection so an empty selection cannot pass.

### Step 6: Run and inspect the task commands

Run the task's commands in their declared order. If a command fails, then fix the implementation or contract target and rerun that exact command; do not replace it with a broad workspace test.

Inspect exit status, discovered binary ID, selected test count, and diagnostics before reporting completion. Report only observed results and only the task-owned outputs.

## Gotchas

- **Always creating a new crate.** `REL-APP-1` is the final binary and `REL-E2E-1` extends the existing CLI test tree. Follow declared outputs, not a universal crate template.
- **Selecting the package instead of the test binary.** `binary_id(audit_query_contract)` and `binary_id(e2e_workflow)` are exact selectors; a typo can produce a green no-tests run.
- **Treating a successful build as contract evidence.** Compilation does not prove acceptance criteria, rollback, stream separation, or absence of remote effects.
- **Letting foundation ownership drift.** Workspace, toolchain, lockfile, and nextest setup belong to the foundation task; later tasks integrate with them rather than recreating them.
- **Inventing missing task commands.** Planning versions and examples are not evidence. Use the current task contract and actual runtime output.

## Validation

Self-check the implementation against the owning task before handoff:

- [ ] Every `expectedOutputs` path exists, and no out-of-scope implementation file was added.
- [ ] The frontmatter-independent package name, modules, and public boundaries match the task owner and exclusions.
- [ ] The exact named test target exists and contains assertions for each acceptance boundary.
- [ ] The exact selected nextest expression reports a non-zero discovered test count.
- [ ] Formatting, clippy, and any task-specific workflow or policy command pass exactly as declared.
- [ ] The completion report distinguishes observed automated evidence from human approval, live acceptance, or release sign-off.

If a selected test is missing, fix the test target before changing the selector. If the current repository has not implemented the task yet, report the prerequisite instead of claiming validation passed.
