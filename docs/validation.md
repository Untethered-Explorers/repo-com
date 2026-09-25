# Human validation runbook

1. Build the validation binary.

   ```bash
   cd <repo-com-checkout>
   rustup toolchain install 1.98.1
   cargo build --release --locked --package command_routing_contract --bin repo-com
   export REPO_COM_BIN="$PWD/target/release/repo-com"
   "$REPO_COM_BIN" --version
   ```

2. Open the repository under test and set isolated validation values.

   ```bash
   cd <repository-under-test>
   export REPO_COM_REPOSITORY_ID='<repository_id from .repo-com.toml>'
   export REPO_COM_DESTINATION='<destination alias>'
   export REPO_COM_INBOUND_ALIAS='<enabled inbound alias>'
   export REPO_COM_STATE="$HOME/.local/state/repo-com/validation.sqlite3"
   mkdir -p "$(dirname "$REPO_COM_STATE")"
   ```

3. Create or update the secret-free repository configuration.

   ```bash
   test -f .repo-com.toml || cp <repo-com-checkout>/examples/repo-com.example.toml .repo-com.toml
   $EDITOR .repo-com.toml
   ```

   Confirm the following values:

   - `schema_version = 1`
   - `repository_id` matches `REPO_COM_REPOSITORY_ID`
   - `discord.workspace_id` is the configured workspace
   - `destinations.<alias>.channel_id` is the configured validation channel
   - `destinations.<alias>.allowed_mentions` contains only configured mention aliases
   - `inbound.<alias>.enabled = true` for the fetch and reply checks
   - `REPO_COM_INBOUND_ALIAS` matches the enabled inbound alias
   - `[retention]` contains `content_days` and `metadata_days`
   - `auto_send` is either `[]` or contains only exact configured tuples
   - the file contains no token, password, private key, or authorization value

4. Load the dedicated bot token into the current process environment.

   ```bash
   read -r -s REPO_COM_DISCORD_TOKEN
   export REPO_COM_DISCORD_TOKEN
   printf '%s\n' 'Discord token loaded'
   ```

5. Validate the configuration.

   ```bash
   "$REPO_COM_BIN" --config .repo-com.toml --state "$REPO_COM_STATE" --output json config validate <<JSON
   {"protocol_version":1,"command":"config.validate","input":{"repository_id":"$REPO_COM_REPOSITORY_ID"}}
   JSON
   ```

   Record the returned `config_hash`, destination aliases, and `auto_send_entries`.

6. Run the read-only Discord setup check.

   ```bash
   "$REPO_COM_BIN" --config .repo-com.toml --state "$REPO_COM_STATE" --output json setup-check <<JSON
   {"protocol_version":1,"command":"setup-check","input":{"repository_id":"$REPO_COM_REPOSITORY_ID"}}
   JSON
   ```

   Confirm bot identity, workspace membership, channel checks, mention checks, and remediation fields. Stop if the report is not `ready`.

7. Record the bot user ID and create a unique validation identifier.

   ```bash
   export BOT_USER_ID='<bot user id from setup-check>'
   export VALIDATION_ID="$(date -u +%Y%m%dT%H%M%SZ)"
   export DRAFT_ID="validation-$VALIDATION_ID"
   export REPLY_DRAFT_ID="reply-$VALIDATION_ID"
   ```

8. Create one outbound draft.

   ```bash
   "$REPO_COM_BIN" --config .repo-com.toml --state "$REPO_COM_STATE" --output json draft create <<JSON
   {"protocol_version":1,"command":"draft.create","input":{"repository_id":"$REPO_COM_REPOSITORY_ID","draft_id":"$DRAFT_ID","destination_alias":"$REPO_COM_DESTINATION","text":"repo-com validation $VALIDATION_ID","event_type":"build_failed","severity":"high","created_at":"2099-01-01T00:00:00Z","created_at_unix_seconds":4070908800,"expires_in_seconds":3600,"metadata":{}}}
   JSON
   ```

   Record the returned draft ID and positive revision. Use revision `1` below if the returned revision is `1`.

9. Preview the exact outbound revision.

   ```bash
   "$REPO_COM_BIN" --config .repo-com.toml --state "$REPO_COM_STATE" --output json draft preview <<JSON
   {"protocol_version":1,"command":"draft.preview","input":{"repository_id":"$REPO_COM_REPOSITORY_ID","draft_id":"$DRAFT_ID","revision":1}}
   JSON
   ```

10. Read the complete preview and copy its exact `preview_hash`.

    ```bash
    export PREVIEW_HASH='<exact preview hash from the previous command>'
    ```

11. Approve the exact revision in a real interactive terminal.

    ```bash
    "$REPO_COM_BIN" --config .repo-com.toml --state "$REPO_COM_STATE" draft approve
    ```

    When the process waits for structured input, enter:

    ```text
    {"protocol_version":1,"command":"draft.approve","input":{"repository_id":"<repository_id>","draft_id":"<draft_id>","revision":1}}
    ```

    Then enter the exact prompt response:

    ```text
    approve <preview-hash>
    ```

    Do not pipe this command or use a heredoc. Do not enter a short `yes` response.

12. Send the approved revision once.

    ```bash
    "$REPO_COM_BIN" --config .repo-com.toml --state "$REPO_COM_STATE" --output json send <<JSON
    {"protocol_version":1,"command":"send","input":{"repository_id":"$REPO_COM_REPOSITORY_ID","draft_id":"$DRAFT_ID","revision":1}}
    JSON
    ```

13. Record the delivery result.

    - For `accepted`, record the redacted remote message ID and continue.
    - For `failed`, record the typed error and stop this validation run.
    - For `unknown` or `unresolved`, stop and do not send again.
    - For `retry-wait`, record the state and do not assume another attempt occurred.

14. Ask a teammate to reply in the same channel with a direct mention of the bot and the text `repo-com validation $VALIDATION_ID`.

15. Fetch the bounded inbound page.

    ```bash
    "$REPO_COM_BIN" --config .repo-com.toml --state "$REPO_COM_STATE" --output json inbox fetch <<JSON
    {"protocol_version":1,"command":"inbox.fetch","input":{"repository_id":"$REPO_COM_REPOSITORY_ID","alias":"$REPO_COM_INBOUND_ALIAS","time":"2020-01-01T00:00:00Z","bot_user_id":"$BOT_USER_ID"}}
    JSON
    ```

    Confirm the fetched item is labeled `untrusted` and corresponds to the validation reply. Record the redacted inbound item ID.

16. Set the inbound item identifier.

    ```bash
    export INBOUND_ITEM_ID='<inbound item id from the fetch result>'
    ```

17. Acknowledge the inbound item locally.

    ```bash
    "$REPO_COM_BIN" --config .repo-com.toml --state "$REPO_COM_STATE" --output json inbox acknowledge <<JSON
    {"protocol_version":1,"command":"inbox.acknowledge","input":{"repository_id":"$REPO_COM_REPOSITORY_ID","item_ids":["$INBOUND_ITEM_ID"],"at":"2099-01-01T00:00:01Z"}}
    JSON
    ```

    Confirm the acknowledgement did not react to, edit, delete, or otherwise mutate the Discord message.

18. Create a linked reply draft.

    ```bash
    "$REPO_COM_BIN" --config .repo-com.toml --state "$REPO_COM_STATE" --output json reply draft-create <<JSON
    {"protocol_version":1,"command":"reply.draft-create","input":{"repository_id":"$REPO_COM_REPOSITORY_ID","inbound_item_id":"$INBOUND_ITEM_ID","draft_id":"$REPLY_DRAFT_ID","text":"repo-com validation reply $VALIDATION_ID","event_type":"build_failed","severity":"high","created_at":"2099-01-01T00:00:00Z","created_at_unix_seconds":4070908800,"expires_in_seconds":3600,"metadata":{}}}
    JSON
    ```

19. Preview the exact reply draft.

    ```bash
    "$REPO_COM_BIN" --config .repo-com.toml --state "$REPO_COM_STATE" --output json draft preview <<JSON
    {"protocol_version":1,"command":"draft.preview","input":{"repository_id":"$REPO_COM_REPOSITORY_ID","draft_id":"$REPLY_DRAFT_ID","revision":1}}
    JSON
    ```

20. Read the complete reply preview and copy its exact `preview_hash`.

    ```bash
    export REPLY_PREVIEW_HASH='<exact reply preview hash>'
    ```

21. Approve the exact reply draft in a real interactive terminal.

    ```bash
    "$REPO_COM_BIN" --config .repo-com.toml --state "$REPO_COM_STATE" draft approve
    ```

    When the process waits for structured input, enter:

    ```text
    {"protocol_version":1,"command":"draft.approve","input":{"repository_id":"<repository_id>","draft_id":"<reply_draft_id>","revision":1}}
    ```

    Then enter the exact prompt response:

    ```text
    approve <reply-preview-hash>
    ```

22. Send the approved reply draft once.

    ```bash
    "$REPO_COM_BIN" --config .repo-com.toml --state "$REPO_COM_STATE" --output json send <<JSON
    {"protocol_version":1,"command":"send","input":{"repository_id":"$REPO_COM_REPOSITORY_ID","draft_id":"$REPLY_DRAFT_ID","revision":1}}
    JSON
    ```

    Record the redacted remote reply message ID and delivery result.

23. Query bounded local audit evidence.

    ```bash
    "$REPO_COM_BIN" --config .repo-com.toml --state "$REPO_COM_STATE" --output json audit query <<JSON
    {"protocol_version":1,"command":"audit.query","input":{"repository_id":"$REPO_COM_REPOSITORY_ID","page_size":50}}
    JSON
    ```

    Confirm the audit page contains the expected redacted local transitions.

24. Verify the isolated local state database.

    ```bash
    "$REPO_COM_BIN" --config .repo-com.toml --state "$REPO_COM_STATE" --output json state verify <<JSON
    {"protocol_version":1,"command":"state.verify","input":{"repository_id":"$REPO_COM_REPOSITORY_ID","database_path":"$REPO_COM_STATE"}}
    JSON
    ```

25. Build a non-mutating purge plan.

    ```bash
    "$REPO_COM_BIN" --config .repo-com.toml --state "$REPO_COM_STATE" --output json purge plan <<JSON
    {"protocol_version":1,"command":"purge.plan","input":{"repository_id":"$REPO_COM_REPOSITORY_ID","scope":"content","cutoff":"2099-01-01T00:00:00Z"}}
    JSON
    ```

    Confirm `execution_performed` is `false`. Do not run `purge.execute` as part of this validation.

26. Confirm the remote result manually in Discord.

    - Exactly one validation request message exists.
    - Exactly one validation reply draft message exists.
    - The teammate reply is present with the direct bot mention.
    - No duplicate request or reply message was created.
    - No message was edited or deleted by the local acknowledgement.

27. Rotate or revoke the dedicated bot token in the Discord Developer Portal.

28. Remove the token from the current process environment.

    ```bash
    unset REPO_COM_DISCORD_TOKEN
    ```

29. Keep the isolated state database until the validation record is complete. Manually delete the validation messages in Discord if they are no longer needed.

30. Record the validation result without credentials or private content.

    - reviewer and date/time
    - repository ID and config hash
    - binary version and commit
    - setup-check result
    - outbound draft ID, revision, and redacted message ID
    - inbound item ID and redacted reply message ID
    - acknowledgement result
    - audit-query result
    - state-verification result
    - purge-plan result and `execution_performed: false`
    - duplicate check
    - token rotation or revocation confirmation
    - defects, blockers, and explicit pass or fail
