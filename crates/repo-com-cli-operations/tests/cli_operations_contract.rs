use std::collections::BTreeMap;
use std::path::Path;

use repo_com_config::{
    DestinationConfig, DiscordConfig, InboundConfig, RepositoryConfig, ResolvedConfig,
    RetentionConfig, resolve_model,
};
use repo_com_foundation::{CommandOutcome, RepoComError, TtyMode};
use repo_com_policy::{ActivationReceipt, PolicyTuple};
use repo_com_purge::{PurgeCounts, PurgeCutoff, PurgeExecution, PurgePlan, PurgeScope};
use repo_com_terminal_operations::render::RenderOptions;
use repo_com_terminal_operations::{
    ActivationConfirmationIdentity, ActivationPreviewView, AuditView, CheckState, ConfigStatus,
    ConfigStatusView, DetailField, ExactConfirmation, LifecyclePageView, LifecycleView,
    PolicyActivationSnapshotView, PolicyState, PolicyStatusView, PromptAction, PromptResult,
    Provenance, PurgeConfirmationIdentity, PurgePlanView, StateCheckView, StateVerificationView,
};
use serde_json::{Value, json};

use crate::{
    AuditQueryRequest, AuditService, ConfigService, ConfigValidationRequest, HandlerContext,
    LifecycleInspectionRequest, LifecycleInspectionResult, OperationsOutput, OperationsResult,
    OperationsUi, PolicyActivationRequest, PolicyService, PolicyStatusRequest,
    PurgeExecutionRequest, PurgePlanRequest, PurgeService, StateService, StateVerificationRequest,
    dispatch_json, human_output_fits, machine_output_streams, render_human_outcome,
};

const REPOSITORY: &str = "acme/widgets";
const WORKSPACE: &str = "100000000000000001";
const CHANNEL: &str = "200000000000000001";
const CONFIG_HASH: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const TUPLE_HASH: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const PLAN_HASH: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const NOW: u64 = 1_767_225_600;
const ACTIVATED_AT: &str = "2026-01-01T00:00:00Z";

fn tuple() -> PolicyTuple {
    PolicyTuple::new("build_failed", "release", "high")
}

fn config() -> ResolvedConfig {
    let mut destinations = BTreeMap::new();
    destinations.insert(
        "release".to_owned(),
        DestinationConfig {
            channel_id: CHANNEL.to_owned(),
            allowed_mentions: Vec::new(),
        },
    );
    let mut inbound = BTreeMap::new();
    inbound.insert("release".to_owned(), InboundConfig { enabled: true });
    resolve_model(
        &RepositoryConfig {
            schema_version: 1,
            repository_id: REPOSITORY.to_owned(),
            discord: DiscordConfig {
                workspace_id: WORKSPACE.to_owned(),
            },
            destinations,
            mentions: BTreeMap::new(),
            inbound,
            retention: RetentionConfig::default(),
            auto_send: vec![repo_com_config::AutoSendEntry {
                event_type: "build_failed".to_owned(),
                destination: "release".to_owned(),
                severity: "high".to_owned(),
            }],
        },
        Path::new("/repo-com-cli-operations/.repo-com.toml"),
    )
    .expect("synthetic operations configuration is valid")
}

fn config_view() -> ConfigStatusView {
    ConfigStatusView {
        repository_id: Some(REPOSITORY.to_owned()),
        config_path: Some("/repo-com-cli-operations/.repo-com.toml".to_owned()),
        schema_version: Some(1),
        workspace_id: Some(WORKSPACE.to_owned()),
        config_hash: Some(CONFIG_HASH.to_owned()),
        destination_aliases: vec!["release".to_owned()],
        inbound_aliases: vec!["release".to_owned()],
        auto_send_entries: 1,
        status: ConfigStatus::Valid,
        validation_code: Some("config-valid".to_owned()),
        detail: Some("configuration resolved and validated".to_owned()),
        provenance: Provenance::LocalDecision,
        next_action: "Use the exact repository and alias identifiers.".to_owned(),
    }
}

fn policy_view() -> PolicyStatusView {
    PolicyStatusView {
        repository_id: REPOSITORY.to_owned(),
        tuple: Some(tuple()),
        state: PolicyState::NotActivated,
        config_hash: Some(CONFIG_HASH.to_owned()),
        tuple_hash: Some(TUPLE_HASH.to_owned()),
        activation_id: None,
        activations: Vec::<PolicyActivationSnapshotView>::new(),
        basis: "configured policy has no activation".to_owned(),
        provenance: Provenance::LocalState,
        next_action: "Request an exact TTY activation if policy use is intended.".to_owned(),
    }
}

fn activation_preview() -> ActivationPreviewView {
    ActivationPreviewView {
        repository_id: REPOSITORY.to_owned(),
        tuple: tuple(),
        config_hash: CONFIG_HASH.to_owned(),
        tuple_hash: TUPLE_HASH.to_owned(),
        activation_id: "activation-ops-1".to_owned(),
        provenance: Provenance::LocalDecision,
        next_action: "Confirm the exact repository, tuple, and hashes on a TTY.".to_owned(),
    }
}

fn activation_receipt() -> ActivationReceipt {
    ActivationReceipt {
        activation_id: "activation-ops-1".to_owned(),
        repository_id: REPOSITORY.to_owned(),
        tuple: tuple(),
        config_hash: CONFIG_HASH.to_owned(),
        tuple_hash: TUPLE_HASH.to_owned(),
        activated_at: ACTIVATED_AT.to_owned(),
    }
}

fn state_view() -> StateVerificationView {
    let check = |code: &str| StateCheckView {
        status: CheckState::Passed,
        passed: true,
        code: code.to_owned(),
        details: Vec::new(),
    };
    StateVerificationView {
        repository_id: REPOSITORY.to_owned(),
        healthy: true,
        read_only: true,
        connection_read_only: true,
        privacy_disclosure: "local state is not encrypted at rest".to_owned(),
        quick_check: check("quick-check-passed"),
        foreign_keys: check("foreign-keys-passed"),
        migration: check("migration-passed"),
        repository_scope: check("repository-scope-passed"),
        filesystem_permissions: check("permissions-passed"),
        issues: Vec::new(),
        remediation: Vec::new(),
        provenance: Provenance::LocalState,
        next_action: "Use read-only inspection; no migration or repair was attempted.".to_owned(),
    }
}

fn audit_view() -> AuditView {
    AuditView {
        repository_id: REPOSITORY.to_owned(),
        page_size: 10,
        event_count: 0,
        has_more: false,
        next_cursor: None,
        events: Vec::new(),
        provenance: Provenance::LocalState,
        remote_fetch_performed: false,
        next_action: "This bounded local audit page is complete.".to_owned(),
    }
}

fn lifecycle_view() -> LifecycleView {
    LifecycleView {
        repository_id: REPOSITORY.to_owned(),
        object_type: "draft".to_owned(),
        object_id: "draft-1".to_owned(),
        revision: Some(1),
        state: "created".to_owned(),
        local_state: "local draft state".to_owned(),
        last_fetched_remote: None,
        untrusted: false,
        read_receipt: repo_com_terminal_operations::ReadReceiptStatus::Unavailable,
        reply_claim: repo_com_terminal_operations::ReplyClaimStatus::NotClaimed,
        remote_fetch_performed: false,
        details: vec![DetailField::new("Inspection", "bounded local projection")],
        provenance: Provenance::LocalState,
        next_action: "Inspect local state only; no remote mutation is implied.".to_owned(),
    }
}

fn lifecycle_page() -> LifecyclePageView {
    LifecyclePageView::new(REPOSITORY, "draft", vec![lifecycle_view()], 10, false, None)
}

fn purge_plan() -> PurgePlan {
    PurgePlan {
        schema_version: 1,
        repository_id: REPOSITORY.to_owned(),
        scope: PurgeScope::Content,
        cutoff: PurgeCutoff::from_unix_seconds(NOW).expect("test cutoff"),
        config_hash: CONFIG_HASH.to_owned(),
        counts: PurgeCounts::default(),
        state_fingerprint: "4444444444444444444444444444444444444444444444444444444444444444"
            .to_owned(),
        plan_hash: PLAN_HASH.to_owned(),
    }
}

fn purge_execution() -> PurgeExecution {
    PurgeExecution {
        repository_id: REPOSITORY.to_owned(),
        plan_hash: PLAN_HASH.to_owned(),
        counts: PurgeCounts::default(),
        audit_event_id: "audit-purge-1".to_owned(),
        executed_at: ACTIVATED_AT.to_owned(),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum UiMode {
    Confirm,
    Cancel,
    Mismatch,
}

struct FakeUi {
    mode: UiMode,
    activation_calls: usize,
    purge_calls: usize,
}

impl FakeUi {
    fn new(mode: UiMode) -> Self {
        Self {
            mode,
            activation_calls: 0,
            purge_calls: 0,
        }
    }
}

impl OperationsUi for FakeUi {
    fn request_activation(&mut self, preview: ActivationPreviewView) -> PromptResult {
        self.activation_calls += 1;
        if self.mode == UiMode::Cancel {
            return PromptResult::Cancelled;
        }
        let identity = ActivationConfirmationIdentity::from_preview(&preview);
        let tuple_hash = if self.mode == UiMode::Mismatch {
            "0".repeat(64)
        } else {
            identity.tuple_hash.clone()
        };
        PromptResult::Confirmed(ExactConfirmation {
            action: PromptAction::ActivatePolicy,
            repository_id: identity.repository_id,
            object_id: identity.activation_id,
            scope: Some(identity.tuple.to_string()),
            config_hash: Some(identity.config_hash),
            tuple_hash: Some(tuple_hash),
            plan_hash: None,
        })
    }

    fn request_purge(&mut self, plan: PurgePlanView) -> PromptResult {
        self.purge_calls += 1;
        if self.mode == UiMode::Cancel {
            return PromptResult::Cancelled;
        }
        let identity = PurgeConfirmationIdentity {
            repository_id: plan.repository_id.clone(),
            scope: plan.scope,
            cutoff_unix_seconds: plan.cutoff_unix_seconds,
            cutoff_utc: plan.cutoff_utc.clone(),
            config_hash: plan.config_hash.clone(),
            plan_hash: plan.plan_hash.clone(),
        };
        let plan_hash = if self.mode == UiMode::Mismatch {
            "0".repeat(64)
        } else {
            identity.plan_hash.clone()
        };
        PromptResult::Confirmed(ExactConfirmation {
            action: PromptAction::ConfirmPurge,
            repository_id: identity.repository_id,
            object_id: "purge-plan".to_owned(),
            scope: Some(identity.scope.as_str().to_owned()),
            config_hash: Some(identity.config_hash),
            tuple_hash: None,
            plan_hash: Some(plan_hash),
        })
    }
}

#[derive(Default)]
struct FakeService {
    calls: Vec<String>,
    last_activation: Option<PolicyActivationRequest>,
    last_purge_execution: Option<PurgeExecutionRequest>,
    last_audit: Option<AuditQueryRequest>,
    error: Option<RepoComError>,
}

impl FakeService {
    fn record(&mut self, call: &str) {
        self.calls.push(call.to_owned());
    }
}

impl ConfigService for FakeService {
    fn validate(
        &mut self,
        _config: &ResolvedConfig,
        _request: ConfigValidationRequest,
    ) -> OperationsResult<ConfigStatusView> {
        self.record("config.validate");
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(config_view())
    }
}

impl PolicyService for FakeService {
    fn status(
        &mut self,
        _config: &ResolvedConfig,
        _request: PolicyStatusRequest,
    ) -> OperationsResult<PolicyStatusView> {
        self.record("policy.status");
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(policy_view())
    }

    fn activation_preview(
        &mut self,
        _config: &ResolvedConfig,
        request: PolicyActivationRequest,
    ) -> OperationsResult<ActivationPreviewView> {
        self.record("policy.activation-preview");
        self.last_activation = Some(request);
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(activation_preview())
    }

    fn activate(
        &mut self,
        _config: &ResolvedConfig,
        request: PolicyActivationRequest,
        _confirmation: ExactConfirmation,
        _tty_mode: TtyMode,
    ) -> OperationsResult<ActivationReceipt> {
        self.record("policy.activate");
        self.last_activation = Some(request);
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(activation_receipt())
    }
}

impl StateService for FakeService {
    fn verify(
        &mut self,
        _request: StateVerificationRequest,
    ) -> OperationsResult<StateVerificationView> {
        self.record("state.verify");
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(state_view())
    }

    fn inspect(
        &mut self,
        _request: LifecycleInspectionRequest,
    ) -> OperationsResult<LifecycleInspectionResult> {
        self.record("lifecycle.inspect");
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(LifecycleInspectionResult::Page(lifecycle_page()))
    }
}

impl AuditService for FakeService {
    fn query(&mut self, request: AuditQueryRequest) -> OperationsResult<AuditView> {
        self.record("audit.query");
        self.last_audit = Some(request);
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(audit_view())
    }
}

impl PurgeService for FakeService {
    fn plan(&mut self, _request: PurgePlanRequest) -> OperationsResult<PurgePlan> {
        self.record("purge.plan");
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(purge_plan())
    }

    fn execute(&mut self, request: PurgeExecutionRequest) -> OperationsResult<PurgeExecution> {
        self.record("purge.execute");
        self.last_purge_execution = Some(request);
        if let Some(error) = &self.error {
            return Err(error.clone());
        }
        Ok(purge_execution())
    }
}

fn context(tty_mode: TtyMode) -> HandlerContext {
    HandlerContext::new(tty_mode, NOW)
}

fn envelope(command: &str, input: Value) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "protocol_version": 1,
        "command": command,
        "input": input,
    }))
    .expect("test envelope serializes")
}

fn config_input() -> Value {
    json!({ "repository_id": REPOSITORY })
}

fn policy_input() -> Value {
    json!({
        "repository_id": REPOSITORY,
        "event_type": "build_failed",
        "destination_alias": "release",
        "severity": "high",
    })
}

fn activation_input() -> Value {
    json!({
        "repository_id": REPOSITORY,
        "event_type": "build_failed",
        "destination_alias": "release",
        "severity": "high",
        "activated_at": ACTIVATED_AT,
    })
}

fn state_input() -> Value {
    json!({
        "repository_id": REPOSITORY,
        "database_path": "/tmp/repo-com-state.db",
        "expected_migration": 1,
    })
}

fn lifecycle_input() -> Value {
    json!({
        "repository_id": REPOSITORY,
        "object_type": "draft",
        "object_id": "draft-1",
        "page_size": 10,
    })
}

fn audit_input() -> Value {
    json!({
        "repository_id": REPOSITORY,
        "page_size": 10,
    })
}

fn purge_input() -> Value {
    json!({
        "repository_id": REPOSITORY,
        "scope": "content",
        "cutoff": ACTIVATED_AT,
    })
}

fn purge_execute_input() -> Value {
    json!({
        "repository_id": REPOSITORY,
        "scope": "content",
        "cutoff": ACTIVATED_AT,
        "config_hash": CONFIG_HASH,
        "plan_hash": PLAN_HASH,
        "executed_at": ACTIVATED_AT,
    })
}

#[test]
fn every_operations_command_dispatches_with_protocol_version_one_and_explicit_scope() {
    let config = config();
    let cases = [
        ("config.validate", config_input(), "config"),
        ("policy.status", policy_input(), "policy"),
        ("policy.activate", activation_input(), "activation"),
        ("state.verify", state_input(), "state"),
        ("lifecycle.inspect", lifecycle_input(), "lifecycle-page"),
        ("audit.query", audit_input(), "audit"),
        ("purge.plan", purge_input(), "purge-plan"),
    ];
    for (command, input, view) in cases {
        let mut service = FakeService::default();
        let mut ui = FakeUi::new(UiMode::Confirm);
        let outcome = dispatch_json(
            &mut service,
            &mut ui,
            context(TtyMode::Tty),
            &config,
            &envelope(command, input),
        );
        let json = outcome.to_json().expect("one protocol object");
        let value: Value = serde_json::from_str(&json).expect("valid JSON object");
        assert_eq!(value["protocol_version"], 1, "{command}");
        assert_eq!(value["status"], "success", "{command}: {value}");
        assert_eq!(value["data"]["view"], view, "{command}");
        assert!(human_output_fits(
            &render_human_outcome(&outcome, RenderOptions::plain_text()),
            80
        ));
    }
}

#[test]
fn malformed_unknown_and_invalid_inputs_are_one_failure_object_without_dispatch() {
    let config = config();
    let cases = [
        b"{not-json".to_vec(),
        envelope("unknown.command", config_input()),
        serde_json::to_vec(&json!({
            "protocol_version": 2,
            "command": "config.validate",
            "input": config_input(),
        }))
        .expect("wrong version"),
        serde_json::to_vec(&json!({
            "protocol_version": 1,
            "command": "config.validate",
            "input": config_input(),
            "extra": true,
        }))
        .expect("unknown envelope field"),
        envelope(
            "config.validate",
            json!({ "repository_id": REPOSITORY, "unexpected": true }),
        ),
        envelope(
            "audit.query",
            json!({ "repository_id": REPOSITORY, "page_size": 0 }),
        ),
        envelope(
            "audit.query",
            json!({
                "repository_id": REPOSITORY,
                "page_size": 10,
                "cursor": "other|2026-01-01T00:00:00Z|1",
            }),
        ),
    ];
    for bytes in cases {
        let mut service = FakeService::default();
        let mut ui = FakeUi::new(UiMode::Confirm);
        let outcome = dispatch_json(
            &mut service,
            &mut ui,
            context(TtyMode::NonTty),
            &config,
            &bytes,
        );
        let streams = machine_output_streams(&outcome, None).expect("failure streams");
        let value: Value = serde_json::from_str(streams.stdout()).expect("one JSON object");
        assert_eq!(streams.stderr(), "");
        assert_eq!(value["protocol_version"], 1);
        assert_eq!(value["status"], "error");
        assert_eq!(value["error"]["code"], "usage-schema");
        assert_eq!(outcome.exit_code(), 2);
        assert!(service.calls.is_empty());
    }
}

#[test]
fn identifiers_and_page_bounds_are_explicit_and_no_destination_defaults_are_implied() {
    let config = config();
    let invalid = [
        ("config.validate", json!({})),
        (
            "policy.status",
            json!({ "repository_id": REPOSITORY, "destination_alias": "release" }),
        ),
        (
            "lifecycle.inspect",
            json!({ "repository_id": REPOSITORY, "object_type": "draft", "page_size": 10 }),
        ),
        (
            "lifecycle.inspect",
            json!({
                "repository_id": REPOSITORY,
                "object_type": "draft_revision",
                "object_id": "draft-1",
                "revision": 0,
                "page_size": 10,
            }),
        ),
        (
            "audit.query",
            json!({ "repository_id": REPOSITORY, "page_size": 101 }),
        ),
        (
            "purge.plan",
            json!({ "repository_id": REPOSITORY, "scope": "content" }),
        ),
    ];
    for (command, input) in invalid {
        let mut service = FakeService::default();
        let mut ui = FakeUi::new(UiMode::Confirm);
        let outcome = dispatch_json(
            &mut service,
            &mut ui,
            context(TtyMode::NonTty),
            &config,
            &envelope(command, input),
        );
        assert_eq!(outcome.exit_code(), 2, "{command}");
        assert!(service.calls.is_empty(), "{command}");
    }
}

#[test]
fn activation_and_purge_fail_closed_in_non_tty_without_ui_or_domain_calls() {
    let config = config();
    for (command, input) in [
        ("policy.activate", activation_input()),
        ("purge.execute", purge_execute_input()),
    ] {
        let mut service = FakeService::default();
        let mut ui = FakeUi::new(UiMode::Confirm);
        let outcome = dispatch_json(
            &mut service,
            &mut ui,
            context(TtyMode::NonTty),
            &config,
            &envelope(command, input),
        );
        assert_eq!(outcome.exit_code(), 3, "{command}");
        assert_eq!(ui.activation_calls, 0, "{command}");
        assert_eq!(ui.purge_calls, 0, "{command}");
        assert!(service.calls.is_empty(), "{command}");
        let streams = machine_output_streams(&outcome, None).expect("typed failure");
        let value: Value = serde_json::from_str(streams.stdout()).expect("failure object");
        assert_eq!(value["error"]["code"], "operator-action-required");
    }
}

#[test]
fn tty_confirmation_routes_through_ui_before_domain_authority() {
    let config = config();
    let mut service = FakeService::default();
    let mut ui = FakeUi::new(UiMode::Confirm);
    let outcome = dispatch_json(
        &mut service,
        &mut ui,
        context(TtyMode::Tty),
        &config,
        &envelope("policy.activate", activation_input()),
    );
    assert!(outcome.is_success());
    assert_eq!(ui.activation_calls, 1);
    assert_eq!(
        service.calls,
        vec!["policy.activation-preview", "policy.activate"]
    );

    let mut service = FakeService::default();
    let mut ui = FakeUi::new(UiMode::Confirm);
    let outcome = dispatch_json(
        &mut service,
        &mut ui,
        context(TtyMode::Tty),
        &config,
        &envelope("purge.execute", purge_execute_input()),
    );
    assert!(outcome.is_success());
    assert_eq!(ui.purge_calls, 1);
    assert_eq!(service.calls, vec!["purge.plan", "purge.execute"]);
    let request = service.last_purge_execution.expect("purge request");
    assert_eq!(request.tty_mode, TtyMode::Tty);
    assert_eq!(request.confirmation.plan_hash(), PLAN_HASH);
}

#[test]
fn mismatched_exact_confirmation_never_reaches_mutation_port() {
    let config = config();
    let mut service = FakeService::default();
    let mut ui = FakeUi::new(UiMode::Mismatch);
    let outcome = dispatch_json(
        &mut service,
        &mut ui,
        context(TtyMode::Tty),
        &config,
        &envelope("policy.activate", activation_input()),
    );
    assert_eq!(outcome.exit_code(), 2);
    assert_eq!(ui.activation_calls, 1);
    assert!(!service.calls.iter().any(|call| call == "policy.activate"));

    let mut service = FakeService::default();
    let mut ui = FakeUi::new(UiMode::Mismatch);
    let outcome = dispatch_json(
        &mut service,
        &mut ui,
        context(TtyMode::Tty),
        &config,
        &envelope("purge.execute", purge_execute_input()),
    );
    assert_eq!(outcome.exit_code(), 2);
    assert_eq!(ui.purge_calls, 1);
    assert!(!service.calls.iter().any(|call| call == "purge.execute"));
}

#[test]
fn read_only_and_planning_ports_do_not_dispatch_mutation_operations() {
    let config = config();
    for (command, input, expected) in [
        ("state.verify", state_input(), "state.verify"),
        ("lifecycle.inspect", lifecycle_input(), "lifecycle.inspect"),
        ("audit.query", audit_input(), "audit.query"),
        ("purge.plan", purge_input(), "purge.plan"),
    ] {
        let mut service = FakeService::default();
        let mut ui = FakeUi::new(UiMode::Confirm);
        let outcome = dispatch_json(
            &mut service,
            &mut ui,
            context(TtyMode::NonTty),
            &config,
            &envelope(command, input),
        );
        assert!(outcome.is_success(), "{command}");
        assert_eq!(service.calls, vec![expected], "{command}");
        assert_eq!(ui.activation_calls, 0, "{command}");
        assert_eq!(ui.purge_calls, 0, "{command}");
    }
}

#[test]
fn machine_streams_keep_diagnostics_separate_and_human_output_is_labeled() {
    let config = config();
    let mut service = FakeService::default();
    let mut ui = FakeUi::new(UiMode::Confirm);
    let outcome: CommandOutcome<OperationsOutput> = dispatch_json(
        &mut service,
        &mut ui,
        context(TtyMode::NonTty),
        &config,
        &envelope("state.verify", state_input()),
    );
    let streams = machine_output_streams(&outcome, Some("operator diagnostic".to_owned()))
        .expect("machine streams");
    let value: Value = serde_json::from_str(streams.stdout()).expect("one JSON object");
    assert_eq!(value["protocol_version"], 1);
    assert_eq!(streams.stderr(), "operator diagnostic");
    assert!(!streams.stdout().contains("operator diagnostic"));
    let human = render_human_outcome(&outcome, RenderOptions::plain_text());
    assert!(human.contains("Local state verification"));
    assert!(human.contains("Read only: true"));
    assert!(human_output_fits(&human, 80));
}

#[test]
fn domain_failures_preserve_stable_categories_in_one_json_object() {
    let config = config();
    let mut service = FakeService {
        error: Some(RepoComError::policy_blocked("local policy is blocked")),
        ..FakeService::default()
    };
    let mut ui = FakeUi::new(UiMode::Confirm);
    let outcome = dispatch_json(
        &mut service,
        &mut ui,
        context(TtyMode::NonTty),
        &config,
        &envelope("policy.status", policy_input()),
    );
    assert_eq!(outcome.exit_code(), 4);
    assert_eq!(service.calls, vec!["policy.status"]);
    let streams = machine_output_streams(&outcome, None).expect("domain failure streams");
    let value: Value = serde_json::from_str(streams.stdout()).expect("one failure object");
    assert_eq!(value["error"]["code"], "policy-blocked");
    assert!(value["data"].is_null());
    let human = render_human_outcome(&outcome, RenderOptions::plain_text());
    assert!(human.contains("Local operations error"));
    assert!(human.contains("Error category: policy-blocked"));
    assert!(human_output_fits(&human, 80));
}

#[test]
fn operation_output_views_have_stable_names_and_serializable_data() {
    let output = OperationsOutput::Audit(audit_view());
    assert_eq!(output.view_name(), "audit");
    let json = output.to_protocol_json().expect("output JSON");
    let value: Value = serde_json::from_str(&json).expect("one output object");
    assert_eq!(value["data"]["view"], "audit");
    assert_eq!(value["data"]["remote_fetch_performed"], false);

    let input = crate::input::parse(&envelope("config.validate", config_input()))
        .expect("explicit config input parses");
    assert_eq!(input.command().as_str(), "config.validate");
}
