# Discord setup and least-privilege procedure

This document owns dedicated bot creation, manual permission setup, channel and
mention checks, environment-only token use, and token rotation. The implemented
setup adapter is `crates/repo-com-discord-client`; its contract is
`DISC-CLIENT-1` in `docs/features/discord-delivery-and-reconciliation.md`.

The steps below combine two kinds of statements:

- **repo-com contract:** what the implemented client and command handler accept,
  check, redact, and do not mutate; and
- **official Discord guidance:** the current external setup model and permission
  vocabulary. External guidance is not evidence that a particular live
  workspace has passed a check. The implemented client pins Discord REST v10
  and applies the least privilege checks described below.

Current official references are the [Discord Developer Platform quick start](https://discord.com/developers/quick-start/getting-started),
the [Discord OAuth2 bot-user section](https://discord.com/developers/docs/topics/oauth2#bot-users),
the [Discord permissions reference](https://discord.com/developers/docs/topics/permissions),
and the [current bot identity endpoint](https://discord.com/developers/docs/resources/user#get-current-user).

## 1. Create a dedicated bot identity

1. In the official Discord Developer Portal, create an application for this
   operator's use and add its bot user. Give the bot a purpose-specific name.
2. Install that bot manually into the disposable or configured workspace. Use
   only the bot installation/authorization flow; do not automate a user login.
3. Copy the raw bot token from the bot settings page. The token is a secret.
   Do not put it in `.repo-com.toml`, a draft, a fixture, a log, a review file,
   or a command-line argument.
4. Export it only for the process that needs it:

   ```text
   REPO_COM_DISCORD_TOKEN=<raw dedicated bot token>
   ```

   The angle-bracketed text is a placeholder, not a credential. The implemented
   client reads the environment variable and does not persist the value in
   repository configuration or local SQLite state.

A **dedicated Discord bot** is the only supported identity. A user token,
self-bot credential, OAuth access token for the operator, or mechanism that
impersonates a person is unsupported and must be rejected. Discord's official
documentation describes bot users as a separate user type; repo-com does not
turn that external distinction into permission to accept a user token.

## 2. Configure the repository before granting permissions

Create a secret-free `.repo-com.toml` using the [configuration contract](configuration.md):

- one `discord.workspace_id`;
- named destination aliases, one channel per alias;
- a named mention target for every alias in a destination's
  `allowed_mentions` list; and
- enabled inbound aliases only for channels that need bounded retrieval.

The setup check resolves aliases to channels. It does not accept a raw
channel as a replacement for an alias and does not change a destination in
configuration. Keep the workspace and channel IDs synthetic in examples and
use a disposable workspace for any later human acceptance.

## 3. Apply least-privilege grants manually

A workspace administrator should grant only the permissions needed by the
configured channel roles. The implemented setup report checks these names:

| Configured use | Required check | Why it is needed |
|---|---|---|
| Destination channel | `VIEW_CHANNEL` | Resolve and read the configured channel. |
| Destination channel | `SEND_MESSAGES` | Permit the one text message used by the send path. |
| Enabled inbound channel | `VIEW_CHANNEL` | Resolve the channel during a bounded read. |
| Enabled inbound channel | `READ_MESSAGE_HISTORY` | Read replies and mentions for the explicit fetch boundary. |
| Destination with an allowlisted role mention | `MENTION_ROLES` in the implemented check | Check whether the bot can mention the configured role under current guild rules. |

User-target mention access depends on the target user and guild rules; it is
reported as a separate **mention check** rather than assumed from
`SEND_MESSAGES`. A role must also be mentionable. Remove an unusable mention
alias from the destination allowlist rather than broadening permissions.

Do not grant `ADMINISTRATOR`, `MANAGE_MESSAGES`, Gateway administration, or
other unrelated capabilities for this workflow. The setup operation is
**read-only**: it never creates an application, joins another workspace,
changes roles, overwrites channels, grants permissions, or performs a setup
mutation. A missing grant is a manual administrator action, not an automatic
repair.

Discord permissions are affected by guild roles and channel overwrites. Use the
official permissions reference to understand the remote rule; the setup report
records only the computed result and safe aliases for this repository.

## 4. Run the read-only setup check

The messaging handler's canonical protocol command is `setup-check`. Its input
contains only the exact repository scope:

```json
{
  "protocol_version": 1,
  "command": "setup-check",
  "input": {
    "repository_id": "acme/widgets"
  }
}
```

The handler returns a repository-scoped report containing:

- the bot user ID and whether Discord marked the identity as a dedicated bot;
- workspace membership;
- alias-based channel checks with required, granted, and missing permissions;
- alias-based mention checks; and
- structured issue and remediation fields.

A `ready` report means only that the implemented checks observed no issue at
that time. It is not a live acceptance result, a read receipt, a statement that
a teammate saw a message, or a release approval. Re-run the check after a role,
channel, mention, workspace, or token change.

The setup report is safe to display, but response bodies and credentials are
not retained by the client. If identity validation fails, use the
`authentication` remediation and rotate the dedicated bot token rather than
embedding a diagnostic value in a ticket.

## 5. Troubleshoot channel and mention checks

| Result | Operator action |
|---|---|
| Identity is not a dedicated bot | Replace the credential with a dedicated bot identity; do not use a user or self-bot token. |
| Workspace membership is missing | Invite the bot manually to the configured workspace, then retry. |
| Channel is unavailable or outside the workspace | Correct the destination/inbound alias and its channel; do not add a raw channel override. |
| `VIEW_CHANNEL`, `SEND_MESSAGES`, or `READ_MESSAGE_HISTORY` is missing | Ask a workspace administrator to grant that exact permission on the configured channel, then retry. |
| Role mention is not ready | Use a mentionable role or remove the alias. |
| User mention is not ready | Correct the target or remove the alias; the bot cannot make an arbitrary user mentionable. |
| Authentication failed | Rotate/reset the bot token in the Developer Portal, update the environment, and retry. |

These are **manual** remediations. Repo-com does not silently change Discord
permissions and does not promise that a later remote change will remain true.

## 6. Rotate or revoke a token

Rotation is an operator action, not a configuration edit:

1. Open the bot's settings in the official Discord Developer Portal.
2. Use the portal's reset/rotate-token action to issue a new raw bot token.
3. Replace the value held by `REPO_COM_DISCORD_TOKEN` in the running process
   environment. Do not write the new value to a file, commit, or diagnostic.
4. Remove the old environment value from the process and any local secret
   manager according to that manager's procedure. Revoke or invalidate the old
   token if the portal offers that control.
5. Run `setup-check` again and inspect authentication, membership, channel, and
   mention results.
6. If the old token may have been exposed, revoke it first or immediately
   after issuing the replacement, then investigate local logs and backups
   according to the operator's incident procedure.

The implementation's authentication remediation explicitly says to rotate the
dedicated bot token in `REPO_COM_DISCORD_TOKEN` and retry setup. A token value,
an authorization header, or a response body is never a safe documentation or
support artifact.

## Unsupported setup behavior

Repo-com does not:

- accept a user token, self-bot credential, or arbitrary Discord endpoint;
- create a Discord application, invite a bot, or grant/change permissions;
- support arbitrary destinations, direct messages, broadcasts, or fan-out;
- claim live Discord compatibility from token-free WireMock contracts; or
- treat `ready`, a stored snapshot, or a local report as proof of a teammate's
  attention or a human release decision.

The official links above are setup references. A live round trip belongs to a
separate human-controlled acceptance task and is not established by this guide.

## Sources and limitations

Authoritative sources are `docs/PRD.md#10. Security and Privacy`, the
`DISC-FR-01`/`DISC-FR-02` and Discord constraints in
`docs/features/discord-delivery-and-reconciliation.md`, and the implemented
`repo-com-discord-client` setup/auth modules. External Discord pages may change;
their guidance is attributed above and is not converted into a repo-com
guarantee.
