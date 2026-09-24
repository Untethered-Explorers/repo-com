use std::sync::Mutex;

use repo_com_state::{DraftInput, RepositoryInput, StateError, StateStore};
use serde_json::json;

use crate::{
    AuditError, AuditEvent, AuditWriter, DiagnosticConfig, DiagnosticEmitter, DiagnosticLevel,
    REDACTED, append_in_transaction, disable_diagnostics, init_diagnostics, is_secret_like_field,
    parse_diagnostic_level, redact_metadata, redact_metadata_checked, redact_text,
    render_diagnostic,
};

const NOW: &str = "2026-01-01T00:00:00Z";
const FRACTIONAL_UTC: &str = "2026-01-01T00:00:00.123Z";
const REPOSITORY: &str = "acme/widgets";
const EVENT_ID: &str = "event-a";

static DIAGNOSTIC_TEST_LOCK: Mutex<()> = Mutex::new(());

fn register(store: &mut StateStore) {
    store
        .upsert_repository(&RepositoryInput::new(
            REPOSITORY,
            "123456789012345678",
            "config-hash",
            NOW,
        ))
        .expect("register repository");
}

fn event(event_id: &str, metadata: serde_json::Value) -> AuditEvent {
    AuditEvent::with_metadata(
        REPOSITORY,
        event_id,
        "draft",
        "draft-a",
        "revision_created",
        NOW,
        "agent",
        "success",
        metadata,
    )
}

fn simple_event(event_id: &str) -> AuditEvent {
    event(event_id, json!({"status": "ok"}))
}

fn audit_error_to_state(error: AuditError) -> StateError {
    StateError::Transaction {
        message: error.code().to_owned(),
    }
}

#[test]
fn event_envelope_is_stable_and_utc_validation_is_fail_closed() {
    let event = AuditEvent::with_metadata(
        REPOSITORY,
        EVENT_ID,
        "draft",
        "draft-a",
        "revision_created",
        FRACTIONAL_UTC,
        "agent",
        "success",
        json!({"revision": 1}),
    );
    assert!(event.validate().is_ok());

    let mut non_utc = event.clone();
    non_utc.occurred_at = "2026-01-01T00:00:00+00:00".to_owned();
    assert!(matches!(
        non_utc.validate(),
        Err(AuditError::InvalidTimestamp)
    ));

    let mut invalid_date = event;
    invalid_date.occurred_at = "2026-02-30T00:00:00Z".to_owned();
    assert!(matches!(
        invalid_date.validate(),
        Err(AuditError::InvalidTimestamp)
    ));
}

#[test]
fn writer_and_caller_state_mutation_commit_in_one_transaction() {
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store);
    let event = simple_event(EVENT_ID);

    store
        .with_transaction(|transaction| {
            transaction
                .repositories()
                .drafts()
                .create(&DraftInput::new(
                    REPOSITORY,
                    "draft-a",
                    "build_failed",
                    "release",
                    NOW,
                ))?;
            append_in_transaction(transaction, &event).map_err(audit_error_to_state)?;
            Ok(())
        })
        .expect("state and event commit together");

    assert!(
        store
            .draft(REPOSITORY, "draft-a")
            .expect("draft lookup")
            .is_some()
    );
    let stored = store
        .audit_event(REPOSITORY, EVENT_ID)
        .expect("event lookup")
        .expect("stored event");
    assert_eq!(stored.repository_id, REPOSITORY);
    assert_eq!(stored.object_id, "draft-a");
    assert_eq!(stored.transition, "revision_created");
    assert_eq!(stored.metadata_json, r#"{"status":"ok"}"#);
}

#[test]
fn a_failed_audit_append_rolls_back_the_state_mutation() {
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store);
    let original = simple_event(EVENT_ID);
    {
        let mut writer = AuditWriter::new(&mut store);
        writer.append(&original).expect("seed event");
    }

    let duplicate = AuditEvent::with_metadata(
        REPOSITORY,
        EVENT_ID,
        "draft",
        "draft-duplicate",
        "revision_created",
        NOW,
        "agent",
        "success",
        json!({"status": "duplicate"}),
    );
    let result = store.with_transaction(|transaction| {
        transaction
            .repositories()
            .drafts()
            .create(&DraftInput::new(
                REPOSITORY,
                "draft-rollback",
                "build_failed",
                "release",
                NOW,
            ))?;
        append_in_transaction(transaction, &duplicate).map_err(audit_error_to_state)?;
        Ok(())
    });

    assert!(result.is_err());
    assert!(
        store
            .draft(REPOSITORY, "draft-rollback")
            .expect("rolled-back draft lookup")
            .is_none()
    );
    let preserved = store
        .audit_event(REPOSITORY, EVENT_ID)
        .expect("original event lookup")
        .expect("original event remains");
    assert_eq!(preserved.object_id, "draft-a");
    assert_eq!(preserved.metadata_json, r#"{"status":"ok"}"#);
}

#[test]
fn existing_events_are_append_only_and_later_transitions_do_not_overwrite() {
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store);
    {
        let mut writer = AuditWriter::new(&mut store);
        writer
            .append(&simple_event("event-created"))
            .expect("created");
        writer
            .append(&AuditEvent::new(
                REPOSITORY,
                "event-edited",
                "inbound_item",
                "item-a",
                "remote_edited",
                NOW,
                "discord",
                "observed",
            ))
            .expect("edited");
    }

    let update = store.connection().execute(
        "UPDATE audit_events SET outcome = 'tampered' WHERE repository_id = ?1 AND event_id = ?2",
        [REPOSITORY, "event-created"],
    );
    assert!(update.is_err());
    let delete = store.connection().execute(
        "DELETE FROM audit_events WHERE repository_id = ?1 AND event_id = ?2",
        [REPOSITORY, "event-created"],
    );
    assert!(delete.is_err());

    let count: i64 = store
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM audit_events WHERE repository_id = ?1",
            [REPOSITORY],
            |row| row.get(0),
        )
        .expect("count local events");
    assert_eq!(count, 2);
    assert!(
        store
            .audit_event(REPOSITORY, "event-edited")
            .expect("later event lookup")
            .is_some()
    );
}

#[test]
fn repository_scope_is_required_for_every_appended_event() {
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store);
    {
        let mut writer = AuditWriter::new(&mut store);
        writer.append(&simple_event("event-a")).expect("repo event");
    }

    assert!(
        store
            .audit_event("other/repository", "event-a")
            .expect("cross-repository lookup")
            .is_none()
    );
    let count: i64 = store
        .connection()
        .query_row(
            "SELECT COUNT(*) FROM audit_events WHERE repository_id = ?1",
            ["other/repository"],
            |row| row.get(0),
        )
        .expect("cross-repository count");
    assert_eq!(count, 0);
}

#[test]
fn metadata_redacts_content_tokens_authorization_and_private_keys() {
    let discord_token = "MTIzNDU2Nzg5MDEyMzQ1Njc4.Gabcde.fghijklmnopqrstuvwxyz123456";
    let private_key = "-----BEGIN PRIVATE KEY-----\nnot-a-real-key\n-----END PRIVATE KEY-----";
    let raw = json!({
        "message": "raw message body",
        "authorization": format!("Bot {discord_token}"),
        "headers": {
            "Authorization": format!("Bot {discord_token}"),
            "X-Request-Id": "request-123"
        },
        "bot_token": discord_token,
        "private_key": private_key,
        "password": "correct horse battery staple",
        "nested": {
            "api_key": "api-secret",
            "content": "nested message",
            "status": "safe"
        },
        "unknown_text": "must not be copied"
    });

    let redacted = redact_metadata(&raw);
    let serialized = serde_json::to_string(&redacted).expect("safe JSON");
    for forbidden in [
        "raw message body",
        discord_token,
        "correct horse battery staple",
        "api-secret",
        "nested message",
        "must not be copied",
    ] {
        assert!(!serialized.contains(forbidden), "leaked: {forbidden}");
    }
    assert!(serialized.contains(REDACTED));
    assert_eq!(redacted["nested"]["status"], "safe");

    let event = event("event-redacted", raw);
    assert_eq!(event.metadata["message"], REDACTED);
    assert_eq!(event.metadata["bot_token"], REDACTED);
    assert_eq!(event.metadata["private_key"], REDACTED);
    assert_eq!(event.metadata["password"], REDACTED);
    assert_eq!(event.metadata["nested"]["api_key"], REDACTED);
    assert_eq!(event.metadata["headers"]["Authorization"], REDACTED);
    assert_eq!(event.metadata["headers"]["X-Request-Id"], "request-123");

    let mut raw_event = simple_event("event-serialization");
    raw_event.metadata = json!({
        "message": "serialized message content",
        "token": discord_token,
        "private_key": private_key
    });
    let serialized_event = serde_json::to_string(&raw_event).expect("safe event JSON");
    let debug_event = format!("{raw_event:?}");
    for forbidden in ["serialized message content", discord_token, private_key] {
        assert!(!serialized_event.contains(forbidden));
        assert!(!debug_event.contains(forbidden));
    }

    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store);
    {
        let mut writer = AuditWriter::new(&mut store);
        writer.append(&event).expect("append redacted event");
    }
    let stored = store
        .audit_event(REPOSITORY, "event-redacted")
        .expect("stored event")
        .expect("redacted event");
    assert!(!stored.metadata_json.contains("raw message body"));
    assert!(!stored.metadata_json.contains(discord_token));
    assert!(!stored.metadata_json.contains(private_key));
}

#[test]
fn every_enabled_diagnostic_level_keeps_the_same_redaction_boundary() {
    let discord_token = "MTIzNDU2Nzg5MDEyMzQ1Njc4.Gabcde.fghijklmnopqrstuvwxyz123456";
    let event = event(
        "event-diagnostic",
        json!({
            "message": "diagnostic message content",
            "authorization": format!("Bot {discord_token}"),
            "private_key": "-----BEGIN PRIVATE KEY-----abc-----END PRIVATE KEY-----",
            "api_key": "diagnostic-secret",
            "status": "safe"
        }),
    );

    for level in [
        DiagnosticLevel::Error,
        DiagnosticLevel::Warn,
        DiagnosticLevel::Info,
        DiagnosticLevel::Debug,
        DiagnosticLevel::Trace,
    ] {
        let mut output = Vec::new();
        let mut emitter = DiagnosticEmitter::new(DiagnosticConfig::enabled(level), &mut output);
        assert!(emitter.emit(level, &event).expect("diagnostic emission"));
        let output = String::from_utf8(output).expect("UTF-8 diagnostic");
        assert!(output.contains(REDACTED));
        assert!(!output.contains("diagnostic message content"));
        assert!(!output.contains(discord_token));
        assert!(!output.contains("diagnostic-secret"));
        assert!(!output.contains("BEGIN PRIVATE KEY"));
    }
}

#[test]
fn diagnostics_are_off_by_default_and_never_write_protocol_stdout() {
    let _lock = DIAGNOSTIC_TEST_LOCK.lock().expect("diagnostic test lock");
    let previous = disable_diagnostics();
    let event = simple_event("event-default-off");
    let mut output = Vec::new();
    let mut emitter = DiagnosticEmitter::new(DiagnosticConfig::default(), &mut output);
    assert!(
        !emitter
            .emit(DiagnosticLevel::Trace, &event)
            .expect("disabled emission")
    );
    assert!(output.is_empty());
    assert!(
        render_diagnostic(DiagnosticConfig::default(), DiagnosticLevel::Trace, &event)
            .expect("disabled render")
            .is_none()
    );

    init_diagnostics(DiagnosticConfig::enabled(DiagnosticLevel::Trace));
    assert!(crate::diagnostics_enabled());
    assert_eq!(
        crate::current_diagnostic_config().level,
        DiagnosticLevel::Trace
    );
    assert_eq!(disable_diagnostics(), DiagnosticLevel::Trace);
    assert!(!crate::diagnostics_enabled());
    let _ = previous;
}

#[test]
fn standalone_text_redaction_and_errors_never_echo_secret_values() {
    assert!(is_secret_like_field("api_key"));
    assert!(is_secret_like_field("Authorization"));
    assert!(!is_secret_like_field("author"));
    assert_eq!(redact_text("Authorization: Bot secret-value"), REDACTED);
    assert_eq!(redact_text("Bot short-value"), REDACTED);
    assert_eq!(
        redact_text("前缀 MTIzNDU2.Gabcde.fghijklmnopqrstuvwxyz123456 后缀"),
        REDACTED
    );
    assert_eq!(redact_text("api_key=secret-value"), REDACTED);
    assert_eq!(redact_text("private_key=secret-value"), REDACTED);
    assert_eq!(redact_text("-----BEGIN PRIVATE KEY-----"), REDACTED);
    assert_eq!(redact_text("safe status"), "safe status");
    assert_eq!(
        parse_diagnostic_level("trace").expect("trace level"),
        DiagnosticLevel::Trace
    );
    assert!(parse_diagnostic_level("not-a-level").is_err());
    assert!(matches!(
        redact_metadata_checked(&json!("message text")),
        Err(AuditError::RedactionFailed)
    ));

    let mut event = simple_event("event-error");
    event.event_id = "MTIzNDU2Nzg5MDEyMzQ1Njc4.Gabcde.fghijklmnopqrstuvwxyz123456".to_owned();
    let error = event
        .validate()
        .expect_err("token-shaped identifier must fail");
    assert!(!error.to_string().contains("MTIzNDU2"));
}
