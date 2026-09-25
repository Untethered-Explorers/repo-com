#![cfg(test)]

mod support;

use std::{
    collections::BTreeSet,
    sync::{Mutex, MutexGuard},
    time::Duration,
};

use repo_com_delivery_retry::ReconciliationDecision;
use serde_json::{Value, json};
use tokio::runtime::Builder;

use support::{
    ACCEPTED_MESSAGE_ID, BOT_USER_ID, CHANNEL_ID, CONCURRENT_INVOCATIONS, DESTINATION_ALIAS,
    E2eHarness, EVENT_TYPE, HUMAN_MESSAGE_ID, HUMAN_REPLY_TEXT, OUTBOUND_TEXT, REPOSITORY_ID,
    SEVERITY, TUPLE, assert_create_request, assert_durable_accepted, assert_durable_unknown,
    assert_fixture_hygiene, emit_evidence, format_utc, human_reply_fixture,
    matching_reconciliation, now_unix_seconds, tuple_hash,
};

const ACTIVATION_MARKER: &str = "type `activate";
const PURGE_MARKER: &str = "type `purge";
const PURGE_CUTOFF: &str = "2099-01-01T00:00:00Z";
const PURGE_CUTOFF_UNIX: u64 = 4_070_908_800;
const CONCURRENT_PROCESSES: usize = CONCURRENT_INVOCATIONS;
static E2E_TEST_LOCK: Mutex<()> = Mutex::new(());

struct DraftSetup {
    draft_id: String,
    revision_hash: String,
    exact_text: String,
}

#[test]
fn fixtures_are_token_free_team_free_and_exactly_synthetic() {
    assert_fixture_hygiene();
    let accepted = support::accepted_message_fixture();
    let human = human_reply_fixture();
    assert_eq!(accepted["id"], ACCEPTED_MESSAGE_ID);
    assert_eq!(accepted["channel_id"], CHANNEL_ID);
    assert_eq!(human["id"], HUMAN_MESSAGE_ID);
    assert_eq!(human["content"], HUMAN_REPLY_TEXT);
}

#[test]
fn full_isolated_mocked_journey_preserves_durable_local_and_remote_correlation() {
    let _serial = serial_e2e_test();
    let harness = E2eHarness::start("happy", Duration::ZERO);
    runtime().block_on(async {
        harness.mount_create().await;
        harness.mount_inbound().await;
    });

    let now = now_unix_seconds();
    register_repository(&harness, now);
    let config_hash = activate_policy(&harness, "activation-happy", now);
    let draft = create_draft(&harness, "draft-happy", now);
    assert_eq!(
        draft.exact_text.lines().next(),
        Some(OUTBOUND_TEXT_FOR_TEST)
    );

    let sent = harness.run(
        "send.accepted",
        &["send"],
        json!({
            "repository_id": REPOSITORY_ID,
            "draft_id": draft.draft_id.clone(),
            "revision": 1
        }),
    );
    let sent_data = sent.success_json("send.accepted");
    assert_eq!(
        sent_data["outcome"]["outcome"],
        "accepted",
        "safe delivery outcome: {}; request counts: {:?}",
        sent_data["outcome"],
        runtime().block_on(harness.request_counts())
    );
    assert_eq!(sent_data["outcome"]["message_id"], ACCEPTED_MESSAGE_ID);
    assert_eq!(sent_data["exact_text"], draft.exact_text);
    assert_eq!(sent_data["revision_hash"], draft.revision_hash);
    let content_nonce = sent_data["content_nonce"]
        .as_str()
        .expect("durable content nonce");
    let attempt_id = sent_data["attempt_id"]
        .as_str()
        .expect("durable attempt ID");
    assert_eq!(content_nonce.len(), 64);
    assert!(!attempt_id.is_empty());
    assert_durable_accepted(harness.state_path(), REPOSITORY_ID, attempt_id);

    let fetched = harness.run(
        "inbox.fetch",
        &["inbox", "fetch"],
        json!({
            "repository_id": REPOSITORY_ID,
            "alias": DESTINATION_ALIAS,
            "time": "2015-01-01T00:00:00Z",
            "bot_user_id": BOT_USER_ID,
            "retrieved_at": format_utc(now)
        }),
    );
    let fetched_data = fetched.success_json("inbox.fetch");
    assert_eq!(fetched_data["trust"], "untrusted");
    assert_eq!(fetched_data["commit"]["stored_items"], 1);
    assert_eq!(fetched_data["point_checks_attempted"], 1);
    let inbound = &fetched_data["items"][0];
    assert_eq!(inbound["remote_message_id"], HUMAN_MESSAGE_ID);
    assert_eq!(inbound["text"], HUMAN_REPLY_TEXT);
    assert_eq!(inbound["trust"], "untrusted");
    assert_eq!(inbound["reply_context"]["reference_present"], true);
    assert_eq!(
        inbound["reply_context"]["referenced_message_id"],
        ACCEPTED_MESSAGE_ID
    );
    assert_eq!(inbound["mention_evidence"]["structured_bot_mention"], true);

    let reply = harness.run(
        "reply.draft-create",
        &["reply", "draft-create"],
        json!({
            "repository_id": REPOSITORY_ID,
            "inbound_item_id": HUMAN_MESSAGE_ID,
            "draft_id": "reply-draft-happy",
            "text": "Synthetic mocked acknowledgement draft.",
            "event_type": EVENT_TYPE,
            "severity": SEVERITY,
            "created_at": format_utc(now),
            "created_at_unix_seconds": now,
            "expires_in_seconds": 3600
        }),
    );
    let reply_data = reply.success_json("reply.draft-create");
    assert_eq!(reply_data["draft_id"], "reply-draft-happy");
    assert_eq!(reply_data["inbound_item_id"], HUMAN_MESSAGE_ID);
    assert_eq!(reply_data["draft_only"], true);
    assert_eq!(
        reply_data["message_reference"]["message_id"],
        HUMAN_MESSAGE_ID
    );
    assert_eq!(reply_data["message_reference"]["channel_id"], CHANNEL_ID);
    assert_eq!(reply_data["destination_alias"], DESTINATION_ALIAS);
    assert_eq!(reply_data["revision_hash"].as_str().map(str::len), Some(64));

    let acknowledged = harness.run(
        "inbox.acknowledge",
        &["inbox", "acknowledge"],
        json!({
            "repository_id": REPOSITORY_ID,
            "item_ids": [HUMAN_MESSAGE_ID],
            "at": format_utc(now)
        }),
    );
    let acknowledged_data = acknowledged.success_json("inbox.acknowledge");
    assert_eq!(acknowledged_data["action"], "acknowledge");
    assert_eq!(acknowledged_data["item_ids"][0], HUMAN_MESSAGE_ID);
    assert_eq!(acknowledged_data["remote_mutation"], false);

    let audit_before = query_audit(&harness);
    assert_audit_ids(
        &audit_before,
        &[
            "activation-happy",
            &draft.draft_id,
            attempt_id,
            HUMAN_MESSAGE_ID,
            "reply-draft-happy",
        ],
    );

    let plan = harness.run(
        "purge.plan",
        &["purge", "plan"],
        json!({
            "repository_id": REPOSITORY_ID,
            "scope": "content",
            "cutoff": PURGE_CUTOFF,
            "expected_config_hash": config_hash
        }),
    );
    let plan_data = plan.success_json("purge.plan");
    assert_eq!(plan_data["scope"], "content");
    assert_eq!(plan_data["cutoff_unix_seconds"], PURGE_CUTOFF_UNIX);
    assert_eq!(plan_data["execution_performed"], false);
    assert!(plan_data["counts"]["total_rows"].as_u64().unwrap_or(0) > 0);
    let plan_hash = plan_data["plan_hash"]
        .as_str()
        .expect("durable purge plan hash");
    assert_eq!(plan_data["config_hash"], config_hash);

    let purge_confirmation =
        format!("purge {REPOSITORY_ID} content {PURGE_CUTOFF_UNIX} {config_hash} {plan_hash}");
    let purge = harness.run_tty(
        "purge.execute",
        &["purge", "execute"],
        json!({
            "repository_id": REPOSITORY_ID,
            "scope": "content",
            "cutoff": PURGE_CUTOFF,
            "config_hash": config_hash,
            "plan_hash": plan_hash,
            "executed_at": format_utc(now)
        }),
        PURGE_MARKER,
        &purge_confirmation,
    );
    let purge_text = purge.text("purge.execute");
    assert!(purge_text.contains("Purge execution result"));
    assert!(purge_text.contains(plan_hash));
    assert!(purge_text.contains("executed"));

    let audit_after = query_audit(&harness);
    assert!(
        audit_after["event_count"].as_u64().unwrap_or(0)
            > audit_before["event_count"].as_u64().unwrap_or(0)
    );
    let serialized_audit = serde_json::to_string(&audit_after).expect("audit serializes");
    assert!(!serialized_audit.contains(OUTBOUND_TEXT_FOR_TEST));
    assert!(!serialized_audit.contains(HUMAN_REPLY_TEXT));
    assert!(!serialized_audit.contains("read_receipt"));
    assert!(!serialized_audit.contains("response_analytics"));

    let counts = runtime().block_on(harness.request_counts());
    assert_eq!(counts.create, 1);
    assert_eq!(counts.list, 1);
    assert_eq!(counts.point, 1);
    assert_eq!(counts.total, 3);
    let request = runtime().block_on(harness.create_request());
    assert_create_request(&request, &draft.exact_text, content_nonce);

    emit_evidence(
        "full-isolated-journey",
        counts,
        json!({
            "commands": [
                "config.validate", "policy.status", "policy.activate", "draft.create",
                "draft.preview", "send", "inbox.fetch", "reply.draft-create",
                "inbox.acknowledge", "audit.query", "purge.plan", "purge.execute"
            ],
            "durable_ids_verified": true,
            "correlation_verified": true,
            "acknowledgement_local_only": true,
            "purge_execution_state": "executed",
            "captured_outputs_scanned": harness.capture_count()
        }),
    );
}

#[test]
fn one_hundred_concurrent_invocations_create_one_post_and_one_accepted_attempt() {
    let _serial = serial_e2e_test();
    let harness = E2eHarness::start("concurrency", Duration::from_millis(750));
    runtime().block_on(harness.mount_create());
    let now = now_unix_seconds();
    register_repository(&harness, now);
    activate_policy(&harness, "activation-concurrent", now);
    let draft = create_draft(&harness, "draft-concurrent", now);

    // Every child is spawned with its structured-input pipe held open, so all
    // 100 final-binary processes are live at the same time before any send is
    // released. Small release groups avoid turning SQLite's bounded lock
    // timeout into a process-start storm while retaining the 100-process proof.
    let input = json!({
        "repository_id": REPOSITORY_ID,
        "draft_id": draft.draft_id.clone(),
        "revision": 1
    });
    let batch = harness.runner().run_released_batch(
        "send.concurrent",
        &["send"],
        input,
        CONCURRENT_PROCESSES,
    );
    assert_eq!(batch.spawned, CONCURRENT_INVOCATIONS);
    assert_eq!(batch.alive_before_release, CONCURRENT_INVOCATIONS);
    assert!(batch.release_group_size <= batch.alive_before_release);
    let alive_before_release = batch.alive_before_release;
    let release_group_size = batch.release_group_size;
    let results = batch.results;

    assert_eq!(results.len(), CONCURRENT_INVOCATIONS);
    let mut attempt_ids = BTreeSet::new();
    let mut nonces = BTreeSet::new();
    let mut accepted_invocations = 0_usize;
    let mut claimed_invocations = 0_usize;
    for (index, result) in results.into_iter().enumerate() {
        let data = result.success_json(&format!("send.concurrent.{index}"));
        match data["outcome"]["outcome"].as_str() {
            Some("accepted") => {
                accepted_invocations += 1;
                assert_eq!(data["outcome"]["message_id"], ACCEPTED_MESSAGE_ID);
            }
            Some("claimed") => claimed_invocations += 1,
            outcome => panic!("unexpected concurrent delivery outcome: {outcome:?}"),
        }
        assert_eq!(data["revision_hash"], draft.revision_hash);
        attempt_ids.insert(
            data["attempt_id"]
                .as_str()
                .expect("durable concurrent attempt ID")
                .to_owned(),
        );
        nonces.insert(
            data["content_nonce"]
                .as_str()
                .expect("durable concurrent content nonce")
                .to_owned(),
        );
    }
    assert_eq!(attempt_ids.len(), 1);
    assert_eq!(nonces.len(), 1);
    assert!(accepted_invocations >= 1);
    assert_eq!(
        accepted_invocations + claimed_invocations,
        CONCURRENT_INVOCATIONS
    );
    let attempt_id = attempt_ids.iter().next().expect("one attempt ID");
    assert_durable_accepted(harness.state_path(), REPOSITORY_ID, attempt_id);

    let audit = query_audit(&harness);
    let serialized = serde_json::to_string(&audit).expect("audit serializes");
    assert!(serialized.contains(attempt_id));
    assert!(serialized.contains("accepted"));
    assert!(!serialized.contains(OUTBOUND_TEXT_FOR_TEST));

    let counts = runtime().block_on(harness.request_counts());
    assert_eq!(counts.create, 1);
    assert_eq!(counts.total, 1);
    let request = runtime().block_on(harness.create_request());
    let content_nonce = nonces.iter().next().expect("one content nonce");
    assert_create_request(&request, &draft.exact_text, content_nonce);

    emit_evidence(
        "one-hundred-concurrent-invocations",
        counts,
        json!({
            "concurrent_invocations": CONCURRENT_INVOCATIONS,
            "max_live_processes": alive_before_release,
            "release_group_size": release_group_size,
            "accepted_invocations_observed": accepted_invocations,
            "claimed_invocations_observed": claimed_invocations,
            "durable_accepted_outcomes": attempt_ids.len(),
            "accepted_response_replays": accepted_invocations,
            "unique_accepted_attempts": attempt_ids.len(),
            "second_posts": 0
        }),
    );
}

#[test]
fn post_dispatch_timeout_remains_unknown_and_exact_match_reconciles_read_only() {
    let _serial = serial_e2e_test();
    let harness = E2eHarness::start("unknown", Duration::from_secs(12));
    runtime().block_on(harness.mount_create());
    let now = now_unix_seconds();
    register_repository(&harness, now);
    activate_policy(&harness, "activation-unknown", now);
    let draft = create_draft(&harness, "draft-unknown", now);
    let send_input = json!({
        "repository_id": REPOSITORY_ID,
        "draft_id": draft.draft_id,
        "revision": 1
    });

    let first = harness
        .run("send.unknown", &["send"], send_input.clone())
        .success_json("send.unknown");
    assert_eq!(first["outcome"]["outcome"], "unknown");
    assert_eq!(first["outcome"]["reason"], "post-dispatch-timeout");
    let attempt_id = first["attempt_id"]
        .as_str()
        .expect("durable unknown attempt ID")
        .to_owned();
    let exact_text = first["exact_text"]
        .as_str()
        .expect("retained exact unknown content")
        .to_owned();
    let content_nonce = first["content_nonce"]
        .as_str()
        .expect("retained deterministic nonce")
        .to_owned();
    assert_durable_unknown(harness.state_path(), REPOSITORY_ID, &attempt_id);

    let second = harness
        .run("send.blocked", &["send"], send_input)
        .success_json("send.blocked");
    assert_eq!(second["outcome"]["outcome"], "unknown");
    assert_eq!(second["attempt_id"], attempt_id);
    assert_durable_unknown(harness.state_path(), REPOSITORY_ID, &attempt_id);

    runtime().block_on(harness.mount_reconciliation(&exact_text));
    let (reads, decision) = matching_reconciliation(
        *harness.server().address(),
        harness.token(),
        &exact_text,
        &content_nonce,
        now_unix_seconds(),
    );
    assert_eq!(reads, 1);
    let message_id = match decision {
        ReconciliationDecision::Accepted { message_id, .. } => message_id,
        _ => panic!("one exact matching message reconciles acceptance"),
    };
    assert_eq!(message_id, "300000000000000003");
    assert_durable_unknown(harness.state_path(), REPOSITORY_ID, &attempt_id);

    let counts = runtime().block_on(harness.request_counts());
    assert_eq!(counts.create, 1);
    assert_eq!(counts.list, 1);
    assert_eq!(counts.total, 2);
    let request = runtime().block_on(harness.create_request());
    assert_create_request(&request, &exact_text, &content_nonce);

    emit_evidence(
        "post-dispatch-timeout-exact-reconciliation",
        counts,
        json!({
            "send_invocations": 2,
            "unknown_outcomes": 2,
            "create_requests": counts.create,
            "unsafe_second_posts": 0,
            "reconciliation_reads": reads,
            "reconciliation_decision": "accepted",
            "local_state_remained": "unknown"
        }),
    );
}

const OUTBOUND_TEXT_FOR_TEST: &str = OUTBOUND_TEXT;

fn serial_e2e_test() -> MutexGuard<'static, ()> {
    E2E_TEST_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn register_repository(harness: &E2eHarness, now: u64) {
    let created_at_unix_seconds = now.saturating_sub(60);
    let data = harness
        .run(
            "draft.create.repository-registration",
            &["draft", "create"],
            json!({
                "repository_id": REPOSITORY_ID,
                "draft_id": "repository-registration",
                "destination_alias": DESTINATION_ALIAS,
                "text": "Synthetic repository registration fixture.",
                "event_type": EVENT_TYPE,
                "severity": SEVERITY,
                "created_at": format_utc(created_at_unix_seconds),
                "created_at_unix_seconds": created_at_unix_seconds,
                "expires_in_seconds": 3600
            }),
        )
        .success_json("draft.create.repository-registration");
    assert_eq!(data["draft_id"], "repository-registration");
    assert_eq!(data["revision"], 1);
}

fn activate_policy(harness: &E2eHarness, activation_id: &str, now: u64) -> String {
    let validated = harness.run(
        "config.validate",
        &["config", "validate"],
        json!({
            "repository_id": REPOSITORY_ID,
            "config_path": harness.config_path().to_string_lossy()
        }),
    );
    let validated_data = validated.success_json("config.validate");
    assert_eq!(validated_data["status"], "valid");
    assert_eq!(validated_data["auto_send_entries"], 1);
    let config_hash = validated_data["config_hash"]
        .as_str()
        .expect("canonical config hash")
        .to_owned();
    assert_eq!(config_hash.len(), 64);

    let before = harness
        .run(
            "policy.status.before",
            &["policy", "status"],
            json!({
                "repository_id": REPOSITORY_ID,
                "event_type": EVENT_TYPE,
                "destination_alias": DESTINATION_ALIAS,
                "severity": SEVERITY
            }),
        )
        .success_json("policy.status.before");
    assert_eq!(before["state"], "not-activated");

    let confirmation = format!(
        "activate {REPOSITORY_ID} {activation_id} {config_hash} {} {TUPLE}",
        tuple_hash()
    );
    let activated = harness.run_tty(
        "policy.activate",
        &["policy", "activate"],
        json!({
            "repository_id": REPOSITORY_ID,
            "event_type": EVENT_TYPE,
            "destination_alias": DESTINATION_ALIAS,
            "severity": SEVERITY,
            "activation_id": activation_id,
            "activated_at": format_utc(now)
        }),
        ACTIVATION_MARKER,
        &confirmation,
    );
    let activated_text = activated.text("policy.activate");
    assert!(activated_text.contains("Policy activation"));
    assert!(activated_text.contains("Outcome: activated"));

    let after = harness
        .run(
            "policy.status.after",
            &["policy", "status"],
            json!({
                "repository_id": REPOSITORY_ID,
                "event_type": EVENT_TYPE,
                "destination_alias": DESTINATION_ALIAS,
                "severity": SEVERITY
            }),
        )
        .success_json("policy.status.after");
    assert_eq!(after["state"], "active");
    assert_eq!(after["activation_id"], activation_id);
    assert_eq!(after["config_hash"], config_hash);
    assert_eq!(after["tuple_hash"], tuple_hash());
    config_hash
}

fn create_draft(harness: &E2eHarness, draft_id: &str, now: u64) -> DraftSetup {
    let created_at_unix_seconds = now.saturating_sub(60);
    let created_at = format_utc(created_at_unix_seconds);
    let created = harness.run(
        "draft.create",
        &["draft", "create"],
        json!({
            "repository_id": REPOSITORY_ID,
            "draft_id": draft_id,
            "destination_alias": DESTINATION_ALIAS,
            "text": OUTBOUND_TEXT_FOR_TEST,
            "event_type": EVENT_TYPE,
            "severity": SEVERITY,
            "metadata": {
                "repository_label": "repo-com-e2e",
                "branch": "fixture",
                "commit": "synthetic"
            },
            "created_at": created_at,
            "created_at_unix_seconds": created_at_unix_seconds,
            "expires_in_seconds": 3600
        }),
    );
    let created_data = created.success_json("draft.create");
    assert_eq!(created_data["draft_id"], draft_id);
    assert_eq!(created_data["revision"], 1);
    let revision_hash = created_data["revision_hash"]
        .as_str()
        .expect("immutable revision hash")
        .to_owned();
    assert_eq!(revision_hash.len(), 64);

    let preview = harness.run(
        "draft.preview",
        &["draft", "preview"],
        json!({
            "repository_id": REPOSITORY_ID,
            "draft_id": draft_id,
            "revision": 1
        }),
    );
    let preview_data = preview.success_json("draft.preview");
    assert_eq!(preview_data["revision_hash"], revision_hash);
    let exact_text = preview_data["exact_text"]
        .as_str()
        .expect("exact rendered outbound text")
        .to_owned();
    assert!(exact_text.starts_with(OUTBOUND_TEXT_FOR_TEST));
    assert!(exact_text.contains("-- repo-com delivery-nonce:"));

    DraftSetup {
        draft_id: draft_id.to_owned(),
        revision_hash,
        exact_text,
    }
}

fn query_audit(harness: &E2eHarness) -> Value {
    harness
        .run(
            "audit.query",
            &["audit", "query"],
            json!({
                "repository_id": REPOSITORY_ID,
                "page_size": 100
            }),
        )
        .success_json("audit.query")
}

fn assert_audit_ids(audit: &Value, ids: &[&str]) {
    assert_eq!(audit["repository_id"], REPOSITORY_ID);
    assert_eq!(audit["remote_fetch_performed"], false);
    assert!(audit["event_count"].as_u64().unwrap_or(0) > 0);
    let serialized = serde_json::to_string(audit).expect("audit serializes");
    for id in ids {
        assert!(serialized.contains(id), "audit omitted durable ID {id}");
    }
    assert!(!serialized.contains(OUTBOUND_TEXT_FOR_TEST));
    assert!(!serialized.contains(HUMAN_REPLY_TEXT));
}

fn runtime() -> tokio::runtime::Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test WireMock runtime starts")
}
