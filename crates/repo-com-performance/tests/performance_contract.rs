use std::time::Duration;

use crate::{
    DOCUMENTED_NO_NETWORK_COMMANDS, Distribution, Fixture, HarnessConfig, P95_THRESHOLD,
    SAMPLE_COUNT, ensure_final_binary, report_json, run_harness,
};

#[test]
fn performance_contract_collects_exactly_one_hundred_warm_samples_per_class() {
    let binary = ensure_final_binary().expect("final repo-com binary is prepared before timing");
    let fixture = Fixture::create().expect("isolated performance fixture is created");
    let report = run_harness(&binary, &fixture, &HarnessConfig::default())
        .expect("all local command classes complete");

    assert_eq!(report.classes.len(), DOCUMENTED_NO_NETWORK_COMMANDS.len());
    assert_eq!(report.sample_count_per_class, SAMPLE_COUNT);
    assert_eq!(report.threshold_ms, 500);
    assert!(report.exclusions.first_compilation_excluded);
    assert!(report.exclusions.network_io_excluded);
    assert!(report.exclusions.process_start_included_in_wall_time);
    assert!(!report.exclusions.internal_function_timer_used);
    assert_eq!(report.network.external_request_count, 0);
    assert_eq!(report.network.network_capable_commands_selected, 0);
    assert!(!report.network.discord_token_present);
    assert!(report.passed, "performance report: {report:?}");

    for class in &report.classes {
        assert_eq!(class.sample_count, SAMPLE_COUNT);
        assert_eq!(class.warmup_sample_count, 1);
        assert_eq!(class.complete_process_count, SAMPLE_COUNT + 1);
        assert_eq!(class.distribution.sample_count, SAMPLE_COUNT);
        assert_eq!(class.samples_ms.len(), SAMPLE_COUNT);
        assert!(class.passed, "{} exceeded the p95 budget", class.class);
        assert_eq!(class.network.network_capable_commands_selected, 0);
        assert!(!class.network.discord_token_present);
    }

    let serialized = report_json(&report).expect("machine-readable report");
    assert!(serialized.contains("\"schema_version\": 1"));
    assert!(serialized.contains("\"first_compilation_excluded\": true"));
    assert!(serialized.contains("\"network_io_excluded\": true"));
    assert!(serialized.contains("\"external_request_count\": 0"));
    assert!(serialized.contains("\"p50_ms\""));
    assert!(serialized.contains("\"p95_ms\""));
    assert!(serialized.contains("\"max_ms\""));
    assert!(!serialized.contains("Authorization"));
    assert!(!serialized.contains("REPO_COM_DISCORD_TOKEN"));
}

#[test]
fn performance_threshold_is_inclusive_and_rejects_injected_over_budget_p95() {
    let at_threshold = vec![P95_THRESHOLD; SAMPLE_COUNT];
    let at_threshold = Distribution::from_samples(&at_threshold).expect("threshold distribution");
    assert_eq!(at_threshold.p95_duration(), Duration::from_millis(500));
    assert!(at_threshold.passes(P95_THRESHOLD));

    let mut over_budget = vec![Duration::from_millis(499); SAMPLE_COUNT];
    for sample in over_budget.iter_mut().skip(94) {
        *sample = Duration::from_millis(501);
    }
    let over_budget = Distribution::from_samples(&over_budget).expect("over-budget distribution");
    assert_eq!(over_budget.p95_duration(), Duration::from_millis(501));
    assert!(!over_budget.passes(P95_THRESHOLD));
}

#[test]
fn fixture_isolated_and_allowlist_contains_no_network_command() {
    let fixture = Fixture::create().expect("isolated fixture");
    assert!(fixture.root().starts_with(std::env::temp_dir()));
    assert!(fixture.config_path().starts_with(fixture.root()));
    assert!(fixture.state_path().starts_with(fixture.root()));

    for spec in fixture.command_specs() {
        assert!(!spec.network, "{} is network-capable", spec.class);
        assert!(!spec.args.iter().any(|arg| arg == "send"));
        assert!(!spec.args.iter().any(|arg| arg == "setup-check"));
        assert!(!spec.args.iter().any(|arg| arg == "fetch"));
        assert!(!spec.stdin.contains("token"));
        assert!(!spec.stdin.contains("Authorization"));
    }
}
