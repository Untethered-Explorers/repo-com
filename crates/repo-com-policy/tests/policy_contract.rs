use std::path::Path;

use repo_com_config::{AutoSendEntry, RepositoryConfig, parse_config};
use repo_com_foundation::TtyMode;
use repo_com_state::{PolicyActivationInput, RepositoryInput, StateStore};

use crate::{
    OperatorConfirmation, PolicyDecision, PolicyError, PolicyHashes, PolicyRegistry, PolicyStatus,
    PolicyTuple, StaleReason, canonical_config_hash, canonical_tuple_hash, exact_policy_match,
    find_exact_policy, is_sha256_hex, status_from_state,
};

const NOW: &str = "2026-01-01T00:00:00Z";
const LATER: &str = "2026-01-01T00:01:00Z";

fn valid_source() -> &'static str {
    r#"schema_version = 1
repository_id = "acme/widgets"

[discord]
workspace_id = "123456789012345678"

[destinations.release]
channel_id = "234567890123456789"
allowed_mentions = ["oncall"]

[mentions.oncall]
target = "role:345678901234567890"

[inbound.release]
enabled = true

[retention]
content_days = 30
metadata_days = 365

[[auto_send]]
event_type = "build_failed"
destination = "release"
severity = "high"
"#
}

fn config() -> RepositoryConfig {
    parse_config(Path::new("policy-contract.toml"), valid_source()).expect("valid test config")
}

fn tuple() -> PolicyTuple {
    PolicyTuple::new("build_failed", "release", "high")
}

fn register(store: &mut StateStore, config: &RepositoryConfig) {
    store
        .upsert_repository(&RepositoryInput::new(
            config.repository_id.clone(),
            config.discord.workspace_id.clone(),
            canonical_config_hash(config),
            NOW,
        ))
        .expect("register repository");
}

fn registry(store: &mut StateStore) -> PolicyRegistry<&mut StateStore> {
    PolicyRegistry::new(store)
}

fn add_policy(config: &mut RepositoryConfig, event_type: &str, severity: &str) {
    config.auto_send.push(AutoSendEntry {
        event_type: event_type.to_owned(),
        destination: "release".to_owned(),
        severity: severity.to_owned(),
    });
}

#[test]
fn canonical_hashes_cover_normalized_complete_configuration_and_exact_tuple() {
    let first = config();
    let mut reordered = first.clone();
    reordered
        .destinations
        .get_mut("release")
        .expect("release")
        .allowed_mentions = vec!["oncall".to_owned()];
    add_policy(&mut reordered, "build_passed", "low");
    reordered.auto_send.reverse();
    let mut another_order = reordered.clone();
    another_order.auto_send.reverse();

    assert_eq!(canonical_config_hash(&first), first.canonical_hash());
    assert_eq!(
        canonical_config_hash(&reordered),
        canonical_config_hash(&another_order)
    );
    assert_eq!(tuple().canonical_hash(), canonical_tuple_hash(&tuple()));
    assert!(is_sha256_hex(&tuple().canonical_hash()));
    assert!(!is_sha256_hex("not-a-sha256"));

    let mut channel_changed = first.clone();
    channel_changed
        .destinations
        .get_mut("release")
        .expect("release")
        .channel_id = "987654321098765432".to_owned();
    assert_ne!(
        canonical_config_hash(&channel_changed),
        canonical_config_hash(&first)
    );

    let mut mention_changed = first.clone();
    mention_changed
        .destinations
        .get_mut("release")
        .expect("release")
        .allowed_mentions
        .clear();
    assert_ne!(
        canonical_config_hash(&mention_changed),
        canonical_config_hash(&first)
    );

    let mut retention_changed = first.clone();
    retention_changed.retention.content_days = 31;
    assert_ne!(
        canonical_config_hash(&retention_changed),
        canonical_config_hash(&first)
    );

    let mut tuple_changed = first;
    tuple_changed.auto_send[0].event_type = "build_passed".to_owned();
    assert_ne!(
        canonical_config_hash(&tuple_changed),
        canonical_config_hash(&config())
    );
}

#[test]
fn matching_is_exact_and_rejects_wildcards_prefixes_and_broader_severity() {
    let config = config();
    let exact = tuple();
    let matched = exact_policy_match(&config, &exact)
        .expect("matching should be side-effect free")
        .expect("exact policy");
    assert_eq!(matched.tuple, exact);
    assert!(matched.is_exact(&exact));
    assert_eq!(matched.config_hash, canonical_config_hash(&config));
    assert_eq!(matched.tuple_hash, canonical_tuple_hash(&exact));

    let cases = [
        PolicyTuple::new("build_passed", "release", "high"),
        PolicyTuple::new("build_failed", "staging", "high"),
        PolicyTuple::new("build_failed", "release", "critical"),
    ];
    for request in cases {
        assert!(
            find_exact_policy(&config, &request)
                .expect("valid request")
                .is_none()
        );
    }

    let mut prefix = config.clone();
    prefix.auto_send[0].event_type = "build".to_owned();
    assert!(
        find_exact_policy(&prefix, &exact)
            .expect("prefix is only a literal")
            .is_none()
    );

    let mut broader = config.clone();
    broader.auto_send[0].severity = "high_or_higher".to_owned();
    assert!(
        find_exact_policy(&broader, &exact)
            .expect("broader labels are not ranked")
            .is_none()
    );

    let mut wildcard = config.clone();
    wildcard.auto_send[0].event_type = "*".to_owned();
    assert!(matches!(
        find_exact_policy(&wildcard, &exact),
        Err(PolicyError::InvalidPolicyTuple)
    ));

    let mut unsupported = config;
    unsupported.schema_version = 2;
    assert!(matches!(
        find_exact_policy(&unsupported, &exact),
        Err(PolicyError::InvalidConfiguration)
    ));
}

#[test]
fn duplicate_policy_entries_are_ambiguous_and_fail_closed() {
    let mut config = config();
    add_policy(&mut config, "build_failed", "high");
    assert!(matches!(
        find_exact_policy(&config, &tuple()),
        Err(PolicyError::AmbiguousPolicy)
    ));
}

#[test]
fn tty_confirmation_persists_hashes_and_non_tty_activation_fails_closed() {
    let config = config();
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, &config);
    let mut registry = registry(&mut store);

    let denied = registry.activate(&config, &tuple(), TtyMode::NonTty, NOW);
    assert!(matches!(denied, Err(PolicyError::TtyRequired)));
    assert!(matches!(
        registry.status(&config, &tuple()).expect("status"),
        PolicyStatus::NotActivated
    ));

    let preview = registry
        .preview(&config, &tuple(), None)
        .expect("exact preview");
    assert_eq!(preview.config_hash, canonical_config_hash(&config));
    assert_eq!(preview.tuple_hash, canonical_tuple_hash(&tuple()));
    assert_eq!(preview.hashes(), preview.hashes_owned());

    let receipt = registry
        .activate(
            &config,
            &tuple(),
            OperatorConfirmation::confirmed(TtyMode::Tty).expect("TTY confirmation"),
            NOW,
        )
        .expect("activate exact policy");
    assert_eq!(receipt.config_hash, preview.config_hash);
    assert_eq!(receipt.tuple_hash, preview.tuple_hash);
    assert_eq!(receipt.activated_at, NOW);
    assert_eq!(receipt.tuple, tuple());

    let status = registry.status(&config, &tuple()).expect("active status");
    assert!(status.is_active());
    let decision = registry
        .evaluate(&config, &tuple())
        .expect("policy decision");
    assert!(decision.is_eligible());
    assert!(matches!(decision, PolicyDecision::Eligible(_)));

    let repeated = registry
        .activate(&config, &tuple(), TtyMode::Tty, LATER)
        .expect("exact replay is idempotent");
    assert_eq!(repeated, receipt);
    assert_eq!(
        registry
            .activations(&config.repository_id)
            .expect("rows")
            .len(),
        1
    );
}

#[test]
fn destination_mention_retention_and_tuple_changes_stale_prior_activation() {
    type ConfigMutation = fn(&mut RepositoryConfig);

    let base = config();
    let mutations: Vec<(&str, ConfigMutation)> = vec![
        ("destination", |config| {
            config
                .destinations
                .get_mut("release")
                .expect("release")
                .channel_id = "999999999999999999".to_owned();
        }),
        ("mentions", |config| {
            config
                .destinations
                .get_mut("release")
                .expect("release")
                .allowed_mentions
                .clear();
        }),
        ("retention", |config| config.retention.metadata_days = 364),
        ("tuple", |config| {
            config.auto_send[0].severity = "critical".to_owned()
        }),
    ];

    for (name, mutate) in mutations {
        let mut store = StateStore::open_in_memory().expect("open state");
        register(&mut store, &base);
        let mut registry = registry(&mut store);
        registry
            .activate(&base, &tuple(), TtyMode::Tty, NOW)
            .expect("activate before mutation");

        let mut changed = base.clone();
        mutate(&mut changed);
        let status = registry
            .status(&changed, &tuple())
            .expect("stale status after mutation");
        let snapshot = match status {
            PolicyStatus::Stale(snapshot) => snapshot,
            other => panic!("{name} mutation should be stale, got {other:?}"),
        };
        assert!(snapshot.stale_reason.is_some());
        assert_eq!(
            snapshot.current_config_hash,
            canonical_config_hash(&changed)
        );
        assert_ne!(snapshot.current_config_hash, snapshot.recorded_config_hash);
        if name == "tuple" {
            assert_eq!(snapshot.stale_reason, Some(StaleReason::ConfigHashChanged));
        }

        let decision = registry
            .evaluate(&changed, &tuple())
            .expect("stale policy decision");
        assert!(decision.is_stale());
        assert!(!decision.is_eligible());
    }
}

#[test]
fn shared_status_inspection_is_available_without_a_tty() {
    let config = config();
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, &config);
    {
        let mut registry = registry(&mut store);
        registry
            .activate(&config, &tuple(), TtyMode::Tty, NOW)
            .expect("activate");
    }

    let status = status_from_state(&store, &config, &tuple()).expect("shared status");
    assert!(status.is_active());
    assert!(!matches!(status, PolicyStatus::NotActivated));
}

#[test]
fn deactivation_is_permission_reducing_and_requires_no_tty() {
    let config = config();
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, &config);
    let mut registry = registry(&mut store);
    let receipt = registry
        .activate(&config, &tuple(), TtyMode::Tty, NOW)
        .expect("activate");

    let status = registry
        .status(&config, &tuple())
        .expect("status before deactivation");
    assert!(status.is_active());
    let deactivated = registry
        .deactivate(&config.repository_id, &receipt.activation_id, LATER)
        .expect("deactivate without TTY");
    assert!(!deactivated.active);
    assert_eq!(deactivated.deactivated_at.as_deref(), Some(LATER));
    assert!(matches!(
        registry
            .status(&config, &tuple())
            .expect("status after deactivation"),
        PolicyStatus::Deactivated(_)
    ));
}

#[test]
fn multiple_current_activation_rows_are_ambiguous() {
    let config = config();
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, &config);
    let mut registry = registry(&mut store);
    let first = registry
        .activate_with_id(&config, &tuple(), "activation-one", TtyMode::Tty, NOW)
        .expect("first activation");
    let second = PolicyActivationInput::new(
        config.repository_id.clone(),
        "activation-two",
        canonical_config_hash(&config),
        canonical_tuple_hash(&tuple()),
        "build_failed",
        "release",
        "high",
        LATER,
    );
    registry
        .state_mut()
        .activate_policy(&second)
        .expect("second activation");

    let status = registry
        .status(&config, &tuple())
        .expect("ambiguous status");
    assert!(matches!(status, PolicyStatus::Ambiguous { .. }));
    let decision = registry
        .evaluate(&config, &tuple())
        .expect("ambiguous decision");
    assert!(!decision.is_eligible());
    assert!(matches!(decision, PolicyDecision::Ambiguous { .. }));
    assert_ne!(first.activation_id, "activation-two");
}

#[test]
fn preview_and_receipt_carry_downstream_revalidation_hashes() {
    let config = config();
    let mut store = StateStore::open_in_memory().expect("open state");
    register(&mut store, &config);
    let registry = registry(&mut store);
    let preview = registry
        .preview(&config, &tuple(), Some("fixed-id"))
        .expect("preview");
    let expected = PolicyHashes::for_config_tuple(&config, &tuple());
    assert_eq!(preview.config_hash, expected.config_hash);
    assert_eq!(preview.tuple_hash, expected.tuple_hash);
    assert_eq!(preview.activation_id, "fixed-id");
}
