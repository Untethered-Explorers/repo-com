use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
    time::Duration,
};

use crate::{
    MIN_SUCCESSFUL_READS, ManualClock, OneAttemptTransport, ReadError, ReadPage, Reconciler,
    ReconciliationDecision, ReconciliationError, ReconciliationReader, ReconciliationReason,
    ReconciliationRequest, RecoveryTarget, RetryDecision, RetryPolicy, RetryRunner,
    TransportOutcome, UnknownReason, UnknownRecoveryEvidence, is_exact_match,
};

struct SequenceTransport {
    outcomes: VecDeque<TransportOutcome>,
    calls: Arc<AtomicUsize>,
}

impl SequenceTransport {
    fn new(outcomes: Vec<TransportOutcome>) -> (Self, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Self {
                outcomes: outcomes.into(),
                calls: Arc::clone(&calls),
            },
            calls,
        )
    }
}

impl OneAttemptTransport for SequenceTransport {
    fn send_attempt(&mut self, _attempt_number: u8) -> TransportOutcome {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.outcomes
            .pop_front()
            .unwrap_or_else(|| TransportOutcome::definitive_failure("sequence-exhausted"))
    }
}

struct SharedClock {
    now: Arc<AtomicU64>,
    waits: Arc<Mutex<Vec<Duration>>>,
}

impl SharedClock {
    fn new(now: u64) -> (Self, Arc<AtomicU64>, Arc<Mutex<Vec<Duration>>>) {
        let now = Arc::new(AtomicU64::new(now));
        let waits = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                now: Arc::clone(&now),
                waits: Arc::clone(&waits),
            },
            now,
            waits,
        )
    }
}

impl crate::Clock for SharedClock {
    fn now_unix_seconds(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }

    fn wait(&mut self, delay: Duration) -> Result<(), crate::ClockError> {
        self.waits.lock().expect("wait lock").push(delay);
        self.now.fetch_add(delay.as_secs(), Ordering::SeqCst);
        Ok(())
    }
}

type ScriptedReaderResult = (
    ScriptedReader,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    Arc<Mutex<Vec<ReconciliationRequest>>>,
);

struct ScriptedReader {
    pages: VecDeque<Result<ReadPage, ReadError>>,
    reads: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<ReconciliationRequest>>>,
}

impl ScriptedReader {
    fn new(pages: Vec<Result<ReadPage, ReadError>>) -> ScriptedReaderResult {
        let reads = Arc::new(AtomicUsize::new(0));
        let mutations = Arc::new(AtomicUsize::new(0));
        let requests = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                pages: pages.into(),
                reads: Arc::clone(&reads),
                requests: Arc::clone(&requests),
            },
            reads,
            mutations,
            requests,
        )
    }
}

impl ReconciliationReader for ScriptedReader {
    fn read_destination(&mut self, request: &ReconciliationRequest) -> Result<ReadPage, ReadError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.requests
            .lock()
            .expect("request lock")
            .push(request.clone());
        self.pages
            .pop_front()
            .unwrap_or_else(|| Ok(ReadPage::empty()))
    }
}

fn target(unknown_since: u64) -> RecoveryTarget {
    RecoveryTarget::new(
        "release",
        "100",
        "200",
        "300",
        "deterministic-nonce",
        "build failed\nnonce: deterministic-nonce",
        unknown_since,
    )
    .expect("valid recovery target")
}

fn exact_message(target: &RecoveryTarget) -> crate::ObservedMessage {
    crate::ObservedMessage::new(
        "400",
        &target.channel_id,
        &target.bot_author_id,
        &target.nonce,
        &target.exact_content,
    )
}

#[test]
fn definitive_http_rejections_never_enter_retry() {
    for status in [400, 401, 403, 404, 409] {
        let (mut transport, calls) =
            SequenceTransport::new(vec![TransportOutcome::from_status(status, None)]);
        let mut clock = ManualClock::new(0);
        let result = RetryRunner::new(&mut clock)
            .run(&mut transport, 1)
            .expect("definitive result");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "status {status}");
        assert!(matches!(result.decision, RetryDecision::Failed { .. }));
        assert!(clock.waits().is_empty());
    }
}

#[test]
fn retry_stops_before_a_fourth_transport_attempt() {
    let (mut transport, calls) = SequenceTransport::new(vec![
        TransportOutcome::pre_dispatch(repo_com_discord_message::PreDispatchFailure::ConnectFailed),
        TransportOutcome::pre_dispatch(
            repo_com_discord_message::PreDispatchFailure::ConnectTimeout,
        ),
        TransportOutcome::pre_dispatch(repo_com_discord_message::PreDispatchFailure::ConnectFailed),
        TransportOutcome::accepted("must-not-be-called"),
    ]);
    let mut clock = ManualClock::new(0);
    let result = RetryRunner::new(&mut clock)
        .run(&mut transport, 1)
        .expect("bounded result");
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert_eq!(result.request_count, 3);
    assert_eq!(result.last_attempt, 3);
    assert!(matches!(
        result.decision,
        RetryDecision::Stop {
            reason: crate::RetryStopReason::AttemptsExhausted,
            ..
        }
    ));
    assert_eq!(clock.waits().len(), 2);
}

#[test]
fn injected_pre_dispatch_jitter_is_clamped_to_policy_bounds() {
    let policy =
        RetryPolicy::with_jitter_bounds(Duration::from_millis(200), Duration::from_millis(300))
            .expect("jitter policy");
    for (injected, expected) in [
        (Duration::from_nanos(1), Duration::from_millis(200)),
        (Duration::from_secs(5), Duration::from_millis(300)),
    ] {
        let (mut transport, calls) = SequenceTransport::new(vec![
            TransportOutcome::pre_dispatch(
                repo_com_discord_message::PreDispatchFailure::ConnectFailed,
            ),
            TransportOutcome::accepted("401"),
        ]);
        let mut clock = ManualClock::new(0);
        let result = RetryRunner::with_policy(&mut clock, policy)
            .with_jitter(injected)
            .run(&mut transport, 1)
            .expect("jitter result");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(clock.waits(), &[expected]);
        assert!(matches!(result.decision, RetryDecision::Accepted { .. }));
    }
}

#[test]
fn rate_limit_uses_server_delay_and_caps_each_directed_wait() {
    for (server_delay, expected) in [
        (Duration::from_secs(5), Duration::from_secs(5)),
        (Duration::from_secs(120), Duration::from_secs(30)),
    ] {
        let (mut transport, calls) = SequenceTransport::new(vec![
            TransportOutcome::rate_limited(Some(server_delay)),
            TransportOutcome::accepted("402"),
        ]);
        let mut clock = ManualClock::new(0);
        let result = RetryRunner::new(&mut clock)
            .run(&mut transport, 1)
            .expect("rate-limit result");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert_eq!(clock.waits(), &[expected]);
        assert!(matches!(result.decision, RetryDecision::Accepted { .. }));
    }
}

#[test]
fn missing_rate_limit_delay_is_not_guessed() {
    let (mut transport, calls) = SequenceTransport::new(vec![
        TransportOutcome::rate_limited(None),
        TransportOutcome::accepted("must-not-be-called"),
    ]);
    let mut clock = ManualClock::new(0);
    let result = RetryRunner::new(&mut clock)
        .run(&mut transport, 1)
        .expect("missing-delay result");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(clock.waits().is_empty());
    assert!(matches!(
        result.decision,
        RetryDecision::Stop {
            reason: crate::RetryStopReason::MissingServerDelay,
            ..
        }
    ));
}

#[test]
fn post_dispatch_timeout_reset_and_server_response_are_unknown_without_resend() {
    let outcomes = [
        TransportOutcome::unknown(UnknownReason::Timeout),
        TransportOutcome::unknown(UnknownReason::ConnectionReset),
        TransportOutcome::from_status(503, None),
    ];
    for outcome in outcomes {
        let (mut transport, calls) = SequenceTransport::new(vec![outcome.clone()]);
        let mut clock = ManualClock::new(0);
        let result = RetryRunner::new(&mut clock)
            .run(&mut transport, 1)
            .expect("unknown result");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(result.is_unknown());
        assert!(!result.decision.permits_next_attempt());
        assert!(clock.waits().is_empty());
    }
}

#[test]
fn unknown_result_retains_exact_nonce_and_content_for_reconciliation() {
    let evidence = UnknownRecoveryEvidence {
        destination_alias: "release".to_owned(),
        workspace_id: "100".to_owned(),
        channel_id: "200".to_owned(),
        bot_author_id: Some("300".to_owned()),
        request_nonce: "request-nonce".to_owned(),
        content_nonce: "content-nonce".to_owned(),
        exact_content: "body\nnonce: content-nonce".to_owned(),
    };
    let (mut transport, calls) =
        SequenceTransport::new(vec![TransportOutcome::unknown(UnknownReason::Timeout)]);
    let mut clock = ManualClock::new(0);
    let result = RetryRunner::new(&mut clock)
        .with_unknown_evidence(evidence.clone())
        .run(&mut transport, 1)
        .expect("unknown result");
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(result.unknown_evidence(), Some(&evidence));
}

#[test]
fn one_exact_reconciliation_match_is_accepted_and_reads_only_the_target() {
    let target = target(1_000);
    let (reader, reads, mutations, requests) =
        ScriptedReader::new(vec![Ok(ReadPage::new(vec![exact_message(&target)]))]);
    let (clock, _now, _waits) = SharedClock::new(1_000);
    let mut reconciler = Reconciler::new(reader, clock);
    let decision = reconciler
        .reconcile(&target)
        .expect("reconciliation result");
    assert!(matches!(
        decision,
        ReconciliationDecision::Accepted {
            message_id,
            ..
        } if message_id == "400"
    ));
    assert_eq!(reads.load(Ordering::SeqCst), 1);
    assert_eq!(mutations.load(Ordering::SeqCst), 0);
    let requests = requests.lock().expect("requests lock");
    assert_eq!(requests[0].api_version(), "v10");
    assert_eq!(requests[0].target, target);
}

#[test]
fn reconciliation_requires_destination_author_nonce_and_exact_content() {
    let target = target(1_000);
    let mut wrong = exact_message(&target);
    wrong.channel_id = "other".to_owned();
    assert!(!is_exact_match(&target, &wrong));
    let mut wrong_author = exact_message(&target);
    wrong_author.author_id = "other".to_owned();
    assert!(!is_exact_match(&target, &wrong_author));
    let mut wrong_nonce = exact_message(&target);
    wrong_nonce.nonce = "other".to_owned();
    assert!(!is_exact_match(&target, &wrong_nonce));
    let mut wrong_content = exact_message(&target);
    wrong_content.content.push_str(" changed");
    assert!(!is_exact_match(&target, &wrong_content));
}

#[test]
fn multiple_edited_and_deleted_matches_remain_unresolved() {
    let target = target(1_000);
    let cases = [
        (
            vec![exact_message(&target), exact_message(&target)],
            ReconciliationReason::MultipleMatches,
        ),
        (
            vec![
                exact_message(&target),
                crate::ObservedMessage::new(
                    "401",
                    &target.channel_id,
                    &target.bot_author_id,
                    &target.nonce,
                    "different content",
                ),
            ],
            ReconciliationReason::ConflictingContent,
        ),
        (
            vec![exact_message(&target).edited()],
            ReconciliationReason::EditedMatch,
        ),
        (
            vec![exact_message(&target).deleted()],
            ReconciliationReason::DeletedMatch,
        ),
    ];
    for (messages, expected) in cases {
        let (reader, _reads, _mutations, _requests) =
            ScriptedReader::new(vec![Ok(ReadPage::new(messages))]);
        let (clock, _now, _waits) = SharedClock::new(2_000);
        let mut reconciler = Reconciler::new(reader, clock);
        let decision = reconciler.reconcile(&target).expect("conflict result");
        assert_eq!(
            decision,
            ReconciliationDecision::Unresolved {
                reason: expected,
                successful_reads: 0
            }
        );
    }
}

#[test]
fn absence_requires_five_minutes_and_three_successful_reads() {
    let target = target(1_000);
    let (reader, reads, _mutations, _requests) = ScriptedReader::new(vec![
        Ok(ReadPage::empty()),
        Ok(ReadPage::empty()),
        Ok(ReadPage::empty()),
    ]);
    let (clock, now, _waits) = SharedClock::new(1_000);
    let mut reconciler = Reconciler::new(reader, clock);

    let first = reconciler.reconcile(&target).expect("first read");
    assert!(matches!(first, ReconciliationDecision::Unknown { .. }));
    now.store(1_300, Ordering::SeqCst);
    let second = reconciler.reconcile(&target).expect("second read");
    assert!(matches!(
        second,
        ReconciliationDecision::Unknown {
            successful_reads: 2,
            ..
        }
    ));
    let third = reconciler.reconcile(&target).expect("third read");
    assert_eq!(
        third,
        ReconciliationDecision::ReconciledAbsent {
            successful_reads: MIN_SUCCESSFUL_READS,
            observation_started_at_unix_seconds: 1_000,
            observed_at_unix_seconds: 1_300,
        }
    );
    assert_eq!(reads.load(Ordering::SeqCst), 3);
}

#[test]
fn failed_and_incomplete_reads_do_not_count_as_absence_evidence() {
    let target = target(1_000);
    let (reader, _reads, _mutations, _requests) = ScriptedReader::new(vec![
        Err(ReadError::Timeout),
        Ok(ReadPage::incomplete(Vec::new())),
        Ok(ReadPage::empty()),
        Ok(ReadPage::empty()),
    ]);
    let (clock, now, _waits) = SharedClock::new(1_000);
    let mut reconciler = Reconciler::new(reader, clock);
    let first = reconciler.reconcile(&target).expect("failed read result");
    assert!(matches!(
        first,
        ReconciliationDecision::Unresolved {
            reason: ReconciliationReason::ReadFailed { .. },
            successful_reads: 0
        }
    ));
    now.store(1_300, Ordering::SeqCst);
    let second = reconciler.reconcile(&target).expect("incomplete result");
    assert!(matches!(
        second,
        ReconciliationDecision::Unresolved {
            reason: ReconciliationReason::IncompleteRead,
            successful_reads: 0
        }
    ));
    let third = reconciler
        .reconcile(&target)
        .expect("first successful read");
    assert!(matches!(
        third,
        ReconciliationDecision::Unknown {
            successful_reads: 1,
            ..
        }
    ));
    let fourth = reconciler
        .reconcile(&target)
        .expect("second successful read");
    assert!(matches!(
        fourth,
        ReconciliationDecision::Unknown {
            successful_reads: 2,
            ..
        }
    ));
}

#[test]
fn unknown_and_unresolved_results_never_emit_retry_permission() {
    let target = target(1_000);
    let (reader, _reads, _mutations, _requests) =
        ScriptedReader::new(vec![Ok(ReadPage::new(vec![
            exact_message(&target).edited(),
        ]))]);
    let (clock, _now, _waits) = SharedClock::new(1_000);
    let mut reconciler = Reconciler::new(reader, clock);
    let decision = reconciler.reconcile(&target).expect("decision");
    assert!(!decision.permits_automatic_resend());
    assert_eq!(
        decision.delivery_state(),
        repo_com_delivery::DeliveryState::Unresolved
    );
}

#[test]
fn one_hundred_concurrent_reconcilers_share_one_terminal_read() {
    let target = target(1_000);
    let (reader, reads, _mutations, _requests) =
        ScriptedReader::new(vec![Ok(ReadPage::new(vec![exact_message(&target)]))]);
    let (clock, _now, _waits) = SharedClock::new(1_000);
    let reconciler = Arc::new(Mutex::new(Reconciler::new(reader, clock)));
    let barrier = Arc::new(std::sync::Barrier::new(100));
    let results = std::thread::scope(|scope| {
        let handles = (0..100)
            .map(|_| {
                let reconciler = Arc::clone(&reconciler);
                let barrier = Arc::clone(&barrier);
                let target = target.clone();
                scope.spawn(move || {
                    barrier.wait();
                    let mut reconciler = reconciler.lock().expect("reconciler lock");
                    reconciler.reconcile(&target)
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("reconciler thread"))
            .collect::<Vec<_>>()
    });
    assert!(results.iter().all(Result::is_ok));
    assert!(
        results
            .into_iter()
            .all(|result| { matches!(result, Ok(ReconciliationDecision::Accepted { .. })) })
    );
    assert_eq!(reads.load(Ordering::SeqCst), 1);
}

#[test]
fn recovery_api_exposes_no_remote_mutation_operation() {
    let source = [
        include_str!("../src/lib.rs"),
        include_str!("../src/policy.rs"),
        include_str!("../src/reconcile.rs"),
    ]
    .concat();
    for forbidden in [
        "pub fn edit",
        "pub fn delete",
        "pub async fn edit",
        "pub async fn delete",
        "pub fn edit_message",
        "pub fn delete_message",
    ] {
        assert!(
            !source.contains(forbidden),
            "forbidden mutation API: {forbidden}"
        );
    }
}

#[test]
fn reconciliation_rejects_a_non_unknown_entry_state() {
    let target = target(1_000);
    let (reader, _reads, _mutations, _requests) = ScriptedReader::new(vec![Ok(ReadPage::empty())]);
    let (clock, _now, _waits) = SharedClock::new(1_000);
    let mut reconciler = Reconciler::new(reader, clock);
    let mut attempt = repo_com_delivery::DeliveryAttempt {
        repository_id: "repo".to_owned(),
        draft_id: "draft".to_owned(),
        revision: 1,
        attempt_id: "attempt".to_owned(),
        attempt_number: 1,
        request_nonce: "request".to_owned(),
        content_nonce: target.nonce.clone(),
        state: repo_com_delivery::DeliveryState::Accepted,
        started_at: "2026-01-01T00:00:00Z".to_owned(),
        completed_at: None,
        error_code: None,
        remote_message_id: None,
        exact_content: target.exact_content.clone(),
        destination_alias: target.destination_alias.clone(),
        resolved_destination: repo_com_config::ResolvedDestination {
            alias: target.destination_alias.clone(),
            workspace_id: target.workspace_id.clone(),
            channel_id: target.channel_id.clone(),
            allowed_mentions: Vec::new(),
        },
        revision_hash: "hash".to_owned(),
        config_hash: "hash".to_owned(),
        destination_hash: "hash".to_owned(),
        scan_hash: "hash".to_owned(),
        authority: repo_com_send_eligibility::EligibilityAuthority::ActivatedPolicy {
            activation_id: "activation".to_owned(),
            activation_hash: "hash".to_owned(),
            config_hash: "hash".to_owned(),
            tuple_hash: "hash".to_owned(),
            activated_at: "2026-01-01T00:00:00Z".to_owned(),
        },
    };
    attempt.state = repo_com_delivery::DeliveryState::Accepted;
    let result = reconciler.reconcile_attempt(&attempt, "300");
    assert_eq!(
        result,
        Err(ReconciliationError::InvalidState(
            repo_com_delivery::DeliveryState::Accepted
        ))
    );
}
