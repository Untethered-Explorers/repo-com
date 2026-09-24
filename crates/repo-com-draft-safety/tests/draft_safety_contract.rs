use repo_com_draft_model::DraftMetadata;

use crate::{MatchSource, MetadataField, SecretReasonCode, SecretScanStatus, SecretScanner};

fn synthetic_discord_token() -> String {
    format!("{}.{}.{}", "A".repeat(24), "B".repeat(6), "C".repeat(27))
}

#[test]
fn high_confidence_patterns_have_stable_reason_codes() {
    let scanner = SecretScanner::new();
    let cases = [
        (synthetic_discord_token(), SecretReasonCode::DiscordBotToken),
        (
            "Authorization: Bearer synthetic-authorization-value".to_owned(),
            SecretReasonCode::AuthorizationValue,
        ),
        (
            "-----BEGIN PRIVATE KEY-----".to_owned(),
            SecretReasonCode::PrivateKeyMarker,
        ),
        (
            "https://user:synthetic-url-password@example.invalid/resource".to_owned(),
            SecretReasonCode::CredentialUrl,
        ),
        (
            "api_key = \"synthetic-assignment-value\"".to_owned(),
            SecretReasonCode::SecretAssignment,
        ),
    ];

    for (value, expected) in cases {
        let result = scanner.scan(&value, &DraftMetadata::new());
        assert!(result.is_blocked());
        assert!(result.has_reason(expected));
        assert_eq!(result.status(), SecretScanStatus::Finding);
        assert_eq!(result.findings()[0].reason_code().code(), expected.code());
        assert_eq!(SecretReasonCode::from_code(expected.code()), Some(expected));
    }
}

#[test]
fn findings_expose_only_redacted_location_and_reason_metadata() {
    let value = synthetic_discord_token();
    let result = SecretScanner::new().scan(&value, &DraftMetadata::new());
    let finding = result.findings().first().expect("a token finding");
    let location = finding.location();

    assert_eq!(location.source(), MatchSource::RenderedText);
    assert_eq!(location.field(), None);
    assert!(location.start() < location.end());
    assert_eq!(location.byte_range().start, location.start());
    assert_eq!(location.byte_range().end, location.end());

    let debug = format!("{result:?}");
    let display = result.to_string();
    let serialized = serde_json::to_string(&result).expect("the safe result serializes");
    assert!(!debug.contains(&value));
    assert!(!display.contains(&value));
    assert!(!serialized.contains(&value));
    assert!(!debug.contains("A".repeat(24).as_str()));
}

#[test]
fn metadata_values_are_scanned_with_a_safe_field_label() {
    let metadata = DraftMetadata::new()
        .with_repository_label("release-bot/api-key=synthetic-metadata-value")
        .expect("the synthetic repository label is bounded");
    let result = SecretScanner::new().scan_text("ordinary rendered text");
    let metadata_result = SecretScanner::new().scan("", &metadata);

    assert!(result.is_clear());
    assert!(metadata_result.has_reason(SecretReasonCode::SecretAssignment));
    let finding = metadata_result
        .findings()
        .first()
        .expect("a metadata assignment finding");
    assert_eq!(finding.location().source(), MatchSource::Metadata);
    assert_eq!(
        finding.location().field(),
        Some(MetadataField::RepositoryLabel)
    );
}

#[test]
fn clean_repository_text_branches_commits_and_ordinary_urls_are_clear() {
    let commit = "0123456789abcdef0123456789abcdef01234567";
    let metadata = DraftMetadata::new()
        .with_repository_label("https://example.invalid/org/repository")
        .expect("the synthetic label is bounded")
        .with_branch("feature/api-key-parser")
        .expect("the synthetic branch is bounded")
        .with_commit(commit)
        .expect("the synthetic commit is bounded");
    let text =
        "See https://example.invalid/docs?ref=readme and compare branch feature/api-key-parser.";

    let result = SecretScanner::new().scan(text, &metadata);

    assert!(result.is_clear());
    assert!(!result.has_findings());
    assert_eq!(result.finding_count(), 0);
    assert_eq!(result.status(), SecretScanStatus::Clear);
    assert_eq!(result.reason_codes(), Vec::<SecretReasonCode>::new());
}

#[test]
fn repeated_scans_are_deterministic_and_inputs_remain_unchanged() {
    let text = "Authorization: Bearer synthetic-repeat-value";
    let metadata = DraftMetadata::new()
        .with_branch("feature/repeatability")
        .expect("the synthetic branch is bounded");
    let original_text = text.to_owned();
    let original_metadata = metadata.clone();
    let scanner = SecretScanner::new();

    let first = scanner.scan(text, &metadata);
    let second = scanner.scan(text, &metadata);

    assert_eq!(first, second);
    assert_eq!(first.reason_codes(), second.reason_codes());
    assert_eq!(text, original_text);
    assert_eq!(metadata, original_metadata);
    assert_eq!(std::mem::size_of::<SecretScanner>(), 0);
}

#[test]
fn overlapping_specific_patterns_do_not_duplicate_a_single_secret() {
    let value = format!("api_key={}", synthetic_discord_token());
    let result = SecretScanner::new().scan(&value, &DraftMetadata::new());

    assert_eq!(result.finding_count(), 1);
    assert_eq!(
        result.findings()[0].reason_code(),
        SecretReasonCode::DiscordBotToken
    );
}

#[test]
fn common_assignment_variants_and_non_http_credential_urls_are_detected() {
    let scanner = SecretScanner::new();
    let assignments = [
        "API-KEY=\"synthetic-value\"",
        "client_secret: 123456",
        "token=main",
    ];
    assert!(
        scanner
            .scan(assignments[0], &DraftMetadata::new())
            .has_reason(SecretReasonCode::SecretAssignment)
    );
    assert!(
        scanner
            .scan(assignments[1], &DraftMetadata::new())
            .has_reason(SecretReasonCode::SecretAssignment)
    );
    assert!(
        scanner
            .scan(assignments[2], &DraftMetadata::new())
            .is_clear()
    );
    assert!(
        scanner
            .scan("password: \"x\"", &DraftMetadata::new())
            .has_reason(SecretReasonCode::SecretAssignment)
    );

    let url = "postgres://user:synthetic-password@db.invalid/app#access_token=synthetic-fragment";
    let result = scanner.scan(url, &DraftMetadata::new());
    assert!(result.has_reason(SecretReasonCode::CredentialUrl));
    assert!(!format!("{result:?}").contains("synthetic-password"));
    assert!(!format!("{result:?}").contains("synthetic-fragment"));

    let query_authorization =
        "https://example.invalid/callback?authorization=synthetic-query-value";
    assert!(
        scanner
            .scan(query_authorization, &DraftMetadata::new())
            .has_reason(SecretReasonCode::CredentialUrl)
    );
}

#[test]
fn empty_and_unicode_inputs_are_safe_and_deterministic() {
    let scanner = SecretScanner::new();
    let empty = scanner.scan("", &DraftMetadata::new());
    let unicode = scanner.scan("こんにちは — café 🚀", &DraftMetadata::new());
    let unicode_again = scanner.scan("こんにちは — café 🚀", &DraftMetadata::new());

    assert!(empty.is_clear());
    assert!(unicode.is_clear());
    assert_eq!(unicode, unicode_again);
}
