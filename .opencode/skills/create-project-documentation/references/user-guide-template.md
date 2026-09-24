# User Guide

> Write for a user who wants to complete a task, not for a maintainer who wants
> every implementation detail.

## Overview

State what the product does in one or two sentences and link to deeper
architecture or API documentation.

## Install and first use

Include prerequisites, the shortest installation path, the first successful
command, and any local-checkout versus installed-package distinction.

## Core workflow

Present the happy path as numbered user actions. Each action should include a
copy-paste command or concrete UI step:

1. Discover or connect the available resources.
2. Inspect the current state or capabilities.
3. Preview the decision or result.
4. Perform the real action.

## Recipes

Add short, end-to-end recipes for the most common jobs. Prefer one command
block followed by the expected outcome over long conceptual explanations.

## Command or feature reference

Use a compact table for commands, options, statuses, or limits. Link to
specialist reference documents instead of reproducing their full content.

## Configuration

Show the smallest valid configuration example, explain precedence, and list
only the settings users commonly need.

## Safety and data handling

Call out destructive actions, permissions, external access, cost, sensitive
inputs, and dry-run or cancellation controls near the commands that need them.

## Troubleshooting

Organize by user-visible symptom. For each symptom, provide one diagnostic
command, the likely cause, and the next corrective action.

## Further help

Link to the README, API/adapter reference, changelog, ADRs, and issue tracker
where applicable.

### Authoring rules

- Keep the first successful path near the top.
- Use task-oriented headings such as “Discover harnesses” and “Run a prompt”.
- Avoid repeating schema definitions or architecture rationale.
- Explain stable behavior and link to deeper documents for internals.
- Ensure every command and option is valid for the current release.
