use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::{
    CONFIG_FILE_NAME, ConfigError, ConfigResolver, LoadedConfig, WorkspaceReferenceIndex,
    discover_config_path, load_config_file, parse_config, resolve_loaded,
};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new() -> Self {
        let sequence = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "repo-com-config-contract-{}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&path).expect("create temporary test directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn make_repository() -> TempDir {
    let temp = TempDir::new();
    fs::create_dir_all(temp.path().join(".git")).expect("create repository marker");
    temp
}

fn write_config(root: &Path, relative: &str, source: &str) -> PathBuf {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create config parent");
    }
    fs::write(&path, source).expect("write config");
    path
}

fn valid_source() -> String {
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
    .to_owned()
}

fn with_extra_top_level(extra: &str) -> String {
    format!("{}\n{extra}\n", valid_source())
}

#[test]
fn example_parses_and_resolves_without_credentials() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate is under crates/");
    let example_path = workspace_root.join("examples/repo-com.example.toml");
    let source = fs::read_to_string(&example_path).expect("example must exist");
    assert!(!source.to_ascii_lowercase().contains("token"));
    assert!(!source.to_ascii_lowercase().contains("password"));
    assert!(!source.to_ascii_lowercase().contains("authorization"));

    let config = parse_config(&example_path, &source).expect("example must be valid");
    assert_eq!(config.schema_version, 1);
    let resolved = crate::resolve_model(&config, &example_path).expect("resolve example");
    let release = resolved
        .destination("release")
        .expect("release destination");
    assert_eq!(release.workspace_id, "123456789012345678");
    assert_eq!(release.channel_id, "234567890123456789");
    assert_eq!(release.allowed_mentions.len(), 1);
    assert_eq!(release.allowed_mentions[0].alias, "oncall");
    assert_eq!(
        release.allowed_mentions[0].target.as_prefixed(),
        "role:345678901234567890"
    );
    assert!(
        resolved
            .inbound("release")
            .expect("release inbound")
            .enabled
    );
}

#[test]
fn explicit_path_and_bounded_ancestor_discovery_are_normalized() {
    let temp = make_repository();
    let nested = temp.path().join("src/deep");
    fs::create_dir_all(&nested).expect("create nested directory");
    let root_config = write_config(temp.path(), CONFIG_FILE_NAME, &valid_source());
    let explicit = write_config(temp.path(), "config/custom.toml", &valid_source());

    let discovered = discover_config_path(None, &nested, temp.path()).expect("ancestor discovery");
    assert_eq!(
        discovered,
        root_config.canonicalize().expect("canonical root config")
    );

    let explicit_discovered = discover_config_path(
        Some(Path::new("../../config/custom.toml")),
        &nested,
        temp.path(),
    )
    .expect("explicit discovery");
    assert_eq!(
        explicit_discovered,
        explicit.canonicalize().expect("canonical explicit config")
    );
    assert!(explicit_discovered.is_absolute());

    let loaded = ConfigResolver::load_explicit(&explicit).expect("explicit load");
    assert_eq!(loaded.config.repository_id, "acme/widgets");
}

#[test]
fn discovery_reports_zero_and_multiple_candidates_and_never_crosses_root() {
    let outer = TempDir::new();
    let repository = outer.path().join("repository");
    fs::create_dir_all(repository.join(".git")).expect("create nested repository");
    let nested = repository.join("src");
    fs::create_dir_all(&nested).expect("create nested source directory");
    write_config(outer.path(), CONFIG_FILE_NAME, &valid_source());

    let zero = discover_config_path(None, &nested, &repository)
        .expect_err("configuration above repository root must not be searched");
    assert!(matches!(zero, ConfigError::NoCandidates { .. }));
    assert!(zero.to_string().contains("config-not-found"));

    let root_config = write_config(repository.as_path(), CONFIG_FILE_NAME, &valid_source());
    let nested_config = write_config(&nested, CONFIG_FILE_NAME, &valid_source());
    let multiple = discover_config_path(None, &nested, &repository)
        .expect_err("both bounded candidates must be reported");
    assert!(matches!(multiple, ConfigError::MultipleCandidates { .. }));
    let candidates = match &multiple {
        ConfigError::MultipleCandidates { candidates, .. } => candidates.clone(),
        _ => unreachable!(),
    };
    assert_eq!(candidates.len(), 2);
    assert!(candidates.contains(&root_config.canonicalize().expect("canonical root")));
    assert!(candidates.contains(&nested_config.canonicalize().expect("canonical nested")));
}

#[test]
fn valid_schema_resolves_named_mentions_and_inbound_alias() {
    let temp = make_repository();
    let path = write_config(temp.path(), CONFIG_FILE_NAME, &valid_source());
    let loaded = load_config_file(&path).expect("load valid config");
    let resolved = resolve_loaded(&loaded, None).expect("resolve valid config");

    assert_eq!(resolved.config.repository_id, "acme/widgets");
    assert_eq!(resolved.destinations.len(), 1);
    assert_eq!(resolved.inbound.len(), 1);
    assert_eq!(
        resolved.destination("release").unwrap().allowed_mentions[0]
            .target
            .kind(),
        crate::MentionKind::Role
    );
    assert_eq!(
        resolved.inbound("release").unwrap().channel_id,
        "234567890123456789"
    );
}

#[test]
fn missing_required_schema_sections_fail_closed() {
    let source = r#"schema_version = 1
repository_id = "acme/widgets"

[discord]
workspace_id = "123456789012345678"
"#;
    let error = parse_config(Path::new("minimal.toml"), source).expect_err("sections are required");
    assert!(matches!(error, ConfigError::MissingField { .. }));
    assert_eq!(error.field_path(), Some("destinations"));
}

#[test]
fn strict_schema_rejects_unknown_fields_and_future_versions() {
    let temp = TempDir::new();
    let path = temp.path().join("config.toml");
    let unknown = with_extra_top_level("mystery_field = \"UNSAFE_SENTINEL\"");
    let error = parse_config(&path, &unknown).expect_err("unknown key must fail");
    assert!(matches!(error, ConfigError::UnknownField { .. }));
    assert!(!error.to_string().contains("UNSAFE_SENTINEL"));

    for version in [0_i64, 2_i64, 99_i64] {
        let source =
            valid_source().replace("schema_version = 1", &format!("schema_version = {version}"));
        let error = parse_config(&path, &source).expect_err("unsupported version must fail");
        assert!(matches!(
            error,
            ConfigError::UnsupportedSchemaVersion { .. }
        ));
    }
}

#[test]
fn duplicate_aliases_and_exact_policy_entries_are_rejected() {
    let temp = TempDir::new();
    let path = temp.path().join("config.toml");

    let duplicate_mention = valid_source().replace(
        "allowed_mentions = [\"oncall\"]",
        "allowed_mentions = [\"oncall\", \"oncall\"]",
    );
    let error = parse_config(&path, &duplicate_mention).expect_err("duplicate alias must fail");
    assert!(matches!(error, ConfigError::DuplicateAlias { .. }));

    let duplicate_policy = format!(
        "{}\n[[auto_send]]\nevent_type = \"build_failed\"\ndestination = \"release\"\nseverity = \"high\"\n",
        valid_source()
    );
    let error = parse_config(&path, &duplicate_policy).expect_err("duplicate policy must fail");
    assert!(matches!(error, ConfigError::DuplicateAutoSend { .. }));

    let wildcard = valid_source().replace("event_type = \"build_failed\"", "event_type = \"*\"");
    let error = parse_config(&path, &wildcard).expect_err("wildcard policy must fail");
    assert!(matches!(error, ConfigError::InvalidAutoSend { .. }));
}

#[test]
fn mention_targets_must_exist_and_use_only_supported_prefixes() {
    let temp = TempDir::new();
    let path = temp.path().join("config.toml");

    let missing = valid_source().replace("target = \"role:345678901234567890\"", "");
    let error = parse_config(&path, &missing).expect_err("missing target must fail");
    assert!(matches!(error, ConfigError::MissingMentionTarget { .. }));

    let invalid = valid_source().replace("role:345678901234567890", "channel:345678901234567890");
    let error = parse_config(&path, &invalid).expect_err("invalid target must fail");
    assert!(matches!(error, ConfigError::InvalidMentionTarget { .. }));

    let missing_alias = valid_source().replace(
        "allowed_mentions = [\"oncall\"]",
        "allowed_mentions = [\"missing\"]",
    );
    let error = parse_config(&path, &missing_alias).expect_err("missing mention alias must fail");
    assert!(matches!(error, ConfigError::MissingMentionAlias { .. }));
}

#[test]
fn cross_workspace_references_fail_closed_when_local_evidence_is_supplied() {
    let temp = make_repository();
    let path = write_config(temp.path(), CONFIG_FILE_NAME, &valid_source());
    let loaded: LoadedConfig = load_config_file(&path).expect("load config");
    let references =
        WorkspaceReferenceIndex::new().with_channel("234567890123456789", "other-workspace");
    let error = resolve_loaded(&loaded, Some(&references)).expect_err("cross-workspace channel");
    assert!(matches!(error, ConfigError::CrossWorkspaceReference { .. }));
    assert!(!error.to_string().contains("other-workspace"));
}

#[test]
fn secret_and_raw_destination_fields_fail_without_echoing_values() {
    let temp = TempDir::new();
    let path = temp.path().join("config.toml");
    let cases = [
        "token = \"SECRET_SENTINEL\"",
        "password = \"SECRET_SENTINEL\"",
        "private_key = \"SECRET_SENTINEL\"",
        "authorization = \"SECRET_SENTINEL\"",
        "channel_id = \"RAW_SENTINEL\"",
    ];

    for extra in cases {
        let source = format!("{}\n{extra}\n", valid_source());
        let error = parse_config(&path, &source).expect_err("forbidden field must fail");
        assert!(matches!(
            error,
            ConfigError::SecretField { .. } | ConfigError::RawDestinationField { .. }
        ));
        let rendered = format!("{error} {}", error.to_protocol_json().expect("safe JSON"));
        assert!(!rendered.contains("SECRET_SENTINEL"));
        assert!(!rendered.contains("RAW_SENTINEL"));
    }

    let raw_inbound = valid_source().replace(
        "[inbound.release]\nenabled = true",
        "[inbound.release]\nenabled = true\nchannel_id = \"RAW_INBOUND_SENTINEL\"",
    );
    let error = parse_config(&path, &raw_inbound).expect_err("raw inbound channel must fail");
    assert!(matches!(error, ConfigError::RawDestinationField { .. }));
    assert!(!error.to_string().contains("RAW_INBOUND_SENTINEL"));
}

#[test]
fn canonical_hash_is_order_independent_and_protocol_errors_are_safe() {
    let first = parse_config(Path::new("first.toml"), &valid_source()).expect("first config");
    let reordered = valid_source().replace(
        "[destinations.release]\nchannel_id = \"234567890123456789\"\nallowed_mentions = [\"oncall\"]",
        "[destinations.release]\nallowed_mentions = [\"oncall\"]\nchannel_id = \"234567890123456789\"",
    );
    let second = parse_config(Path::new("second.toml"), &reordered).expect("second config");
    assert_eq!(first.canonical_hash(), second.canonical_hash());
    assert_eq!(first.canonical_hash_bytes(), second.canonical_hash_bytes());

    let error =
        parse_config(Path::new("error.toml"), "schema_version = 2\n").expect_err("version error");
    let value = error.to_protocol_value().expect("protocol value");
    assert_eq!(value["code"], "unsupported-schema-version");
    assert!(value["message"].as_str().is_some());
    let json = error.to_protocol_json().expect("protocol JSON");
    assert!(json.contains("unsupported-schema-version"));
    assert!(!json.contains("schema_version = 2"));
}
