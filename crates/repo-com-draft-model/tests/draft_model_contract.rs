use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use repo_com_config::{
    DestinationConfig, DiscordConfig, MentionConfig, RepositoryConfig, ResolvedConfig,
    RetentionConfig, resolve_model,
};

use crate::{
    AuthorizedReplyReference, BasisAuthority, BasisStatus, DEFAULT_EXPIRY_SECONDS, DecisionBases,
    DestinationAlias, DraftBody, DraftError, DraftMetadata, DraftModel, DraftRequest, EventType,
    ExactTextSource, MAX_EXPIRY_SECONDS, SendBlocker, SendDecision, Severity,
    UnresolvedBasisReason,
};

const SNAPSHOT_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn resolved_config(
    repository_id: &str,
    release_channel: &str,
    include_staging: bool,
    include_mentions: bool,
) -> ResolvedConfig {
    let mut destinations = BTreeMap::new();
    destinations.insert(
        "release".to_owned(),
        DestinationConfig {
            channel_id: release_channel.to_owned(),
            allowed_mentions: if include_mentions {
                vec!["oncall".to_owned()]
            } else {
                Vec::new()
            },
        },
    );
    if include_staging {
        destinations.insert(
            "staging".to_owned(),
            DestinationConfig {
                channel_id: release_channel.to_owned(),
                allowed_mentions: Vec::new(),
            },
        );
    }

    let mut mentions = BTreeMap::new();
    if include_mentions {
        mentions.insert(
            "oncall".to_owned(),
            MentionConfig {
                target: "role:300".to_owned(),
            },
        );
    }

    resolve_model(
        &RepositoryConfig {
            schema_version: 1,
            repository_id: repository_id.to_owned(),
            discord: DiscordConfig {
                workspace_id: "100".to_owned(),
            },
            destinations,
            mentions,
            inbound: BTreeMap::new(),
            retention: RetentionConfig::default(),
            auto_send: Vec::new(),
        },
        Path::new("/test/.repo-com.toml"),
    )
    .expect("the synthetic test configuration is valid")
}

fn request() -> DraftRequest {
    DraftRequest::new(
        "draft-1",
        "release",
        "  Build failed.\nReview the logs.  ",
        "build_failed",
        "high",
    )
    .expect("the baseline request is valid")
}

fn metadata() -> DraftMetadata {
    DraftMetadata::new()
        .with_repository_label("Widgets")
        .expect("repository label is valid")
        .with_branch("main")
        .expect("branch is valid")
        .with_commit("abc1234")
        .expect("commit is valid")
}

fn reply_reference() -> AuthorizedReplyReference {
    AuthorizedReplyReference::from_validated_target(
        "acme/widgets",
        "100",
        "200",
        "inbound-1",
        "400",
        "authorization-1",
        SNAPSHOT_HASH,
    )
    .expect("the reply target evidence is complete")
}

fn create_draft(request: DraftRequest, config: &ResolvedConfig, created_at: u64) -> DraftModel {
    DraftModel::create(request, config, created_at).expect("the draft request is valid")
}

#[test]
fn valid_boundaries_default_expiry_and_exact_seven_day_limit() {
    let config = resolved_config("acme/widgets", "200", false, false);
    let draft = create_draft(request(), &config, 1_000);

    assert_eq!(draft.revisions().len(), 1);
    assert_eq!(draft.current_revision().number(), 1);
    assert_eq!(
        draft.current_revision().exact_text(),
        "  Build failed.\nReview the logs.  "
    );
    assert_eq!(
        draft.current_revision().expiry().created_at_unix_seconds(),
        1_000
    );
    assert_eq!(
        draft.current_revision().expiry().expires_at_unix_seconds(),
        1_000 + DEFAULT_EXPIRY_SECONDS
    );
    assert_eq!(draft.current_revision().content_hash().len(), 64);

    let maximum_request = request().with_expiry_seconds(MAX_EXPIRY_SECONDS);
    let maximum = create_draft(maximum_request, &config, 1_000);
    assert_eq!(
        maximum
            .current_revision()
            .expiry()
            .expires_at_unix_seconds(),
        1_000 + MAX_EXPIRY_SECONDS
    );

    let over_limit = request().with_expiry_seconds(MAX_EXPIRY_SECONDS + 1);
    assert_eq!(
        DraftModel::create(over_limit, &config, 1_000).unwrap_err(),
        DraftError::ExpiryTooLong {
            requested_seconds: MAX_EXPIRY_SECONDS + 1,
            maximum_seconds: MAX_EXPIRY_SECONDS,
        }
    );
    assert!(matches!(
        DraftModel::create(request().with_expiry_seconds(0), &config, 1_000),
        Err(DraftError::ExpiryMustBePositive)
    ));
}

#[test]
fn strict_requests_reject_empty_invalid_multiple_broadcast_and_unknown_inputs() {
    let invalid_json = [
        r#"{"draft_id":"draft-1","destination_alias":"release","text":"","event_type":"build_failed","severity":"high"}"#,
        r#"{"draft_id":"draft-1","destination_alias":"release","text":"   ","event_type":"build_failed","severity":"high"}"#,
        r#"{"draft_id":"draft-1","destination_alias":"release","text":"body","event_type":"build/*","severity":"high"}"#,
        r#"{"draft_id":"draft-1","destination_alias":"release","text":"body","event_type":"build_failed","severity":"high or higher"}"#,
        r#"{"draft_id":"draft-1","destination_alias":["release","staging"],"text":"body","event_type":"build_failed","severity":"high"}"#,
        r#"{"draft_id":"draft-1","destination_alias":"*","text":"body","event_type":"build_failed","severity":"high"}"#,
        r#"{"draft_id":"draft-1","destination_alias":"200","text":"body","event_type":"build_failed","severity":"high"}"#,
        r#"{"draft_id":"draft-1","destination_alias":"release","text":"body","event_type":"build_failed","severity":"high","attachment":"file.txt"}"#,
        r#"{"draft_id":"draft-1","destination_alias":"release","text":"body","event_type":"build_failed","severity":"high","metadata":{"attachments":[]}}"#,
        r#"{"draft_id":"draft-1","destination_alias":"release","text":"body","event_type":"build_failed","severity":"high","reply_reference":"400"}"#,
        r#"{"draft_id":"draft-1","destination_alias":"release","text":"body","event_type":"build_failed","severity":"high","reply_reference":{"message_id":"400"}}"#,
    ];

    for json in invalid_json {
        assert!(
            serde_json::from_str::<DraftRequest>(json).is_err(),
            "request unexpectedly decoded: {json}"
        );
    }

    assert!(matches!(
        DraftBody::new(" \n\t"),
        Err(DraftError::InvalidText)
    ));
    assert!(matches!(
        EventType::new("build/*"),
        Err(DraftError::InvalidEventType)
    ));
    assert!(matches!(
        Severity::new("high or higher"),
        Err(DraftError::InvalidSeverity)
    ));
    assert!(matches!(
        DestinationAlias::new("200"),
        Err(DraftError::InvalidDestinationAlias)
    ));
    assert!(matches!(
        DraftMetadata::new().with_branch("x".repeat(257)),
        Err(DraftError::InvalidMetadataValue { field: "branch" })
    ));
}

#[test]
fn only_an_authorized_opaque_reply_reference_is_carried_as_metadata() {
    let config = resolved_config("acme/widgets", "200", false, false);
    let reference = reply_reference();
    let draft = create_draft(
        request()
            .with_metadata(metadata())
            .with_reply_reference(reference.clone()),
        &config,
        1_000,
    );
    let revision = draft.current_revision();

    assert_eq!(revision.reply_reference(), Some(&reference));
    assert_eq!(revision.exact_text(), request().text().as_str());
    assert_eq!(reference.inbound_item_id(), "inbound-1");
    assert_eq!(reference.message_id(), "400");
    assert_eq!(reference.authorization_reference(), "authorization-1");
    assert_eq!(reference.validated_snapshot_hash(), SNAPSHOT_HASH);

    let other_repository = resolved_config("other/widgets", "200", false, false);
    assert_eq!(
        DraftModel::create(
            request().with_reply_reference(reference.clone()),
            &other_repository,
            1_000
        )
        .unwrap_err(),
        DraftError::ReplyRepositoryMismatch
    );

    let mut other_workspace = config.config.clone();
    other_workspace.discord.workspace_id = "101".to_owned();
    let other_workspace = resolve_model(&other_workspace, Path::new("/test/other.toml"))
        .expect("the alternate workspace configuration is valid");
    assert_eq!(
        DraftModel::create(
            request().with_reply_reference(reference.clone()),
            &other_workspace,
            1_000
        )
        .unwrap_err(),
        DraftError::ReplyWorkspaceMismatch
    );

    let other_channel = resolved_config("acme/widgets", "201", false, false);
    assert_eq!(
        DraftModel::create(
            request().with_reply_reference(reference),
            &other_channel,
            1_000
        )
        .unwrap_err(),
        DraftError::ReplyChannelMismatch
    );
}

#[test]
fn each_change_appends_a_revision_and_never_mutates_prior_snapshots() {
    let release = resolved_config("acme/widgets", "200", false, false);
    let mut draft = create_draft(request(), &release, 1_000);
    let first = draft.current_revision().clone();

    draft
        .revise(
            DraftRequest::new("draft-1", "release", "Body changed", "build_failed", "high")
                .unwrap(),
            &release,
            1_000,
        )
        .unwrap();
    draft
        .revise(
            request().with_metadata(DraftMetadata::new().with_branch("release/1.0").unwrap()),
            &release,
            1_000,
        )
        .unwrap();
    draft
        .revise(
            DraftRequest::new(
                "draft-1",
                "staging",
                request().text().as_str(),
                "build_failed",
                "high",
            )
            .unwrap(),
            &resolved_config("acme/widgets", "200", true, false),
            1_000,
        )
        .unwrap();
    draft
        .revise(
            request(),
            &resolved_config("acme/widgets", "201", false, false),
            1_000,
        )
        .unwrap();
    draft
        .revise(
            request(),
            &resolved_config("other/widgets", "200", false, false),
            1_000,
        )
        .unwrap();
    draft
        .revise(request().with_expiry_seconds(3_600), &release, 1_000)
        .unwrap();

    assert_eq!(draft.revisions().len(), 7);
    assert_eq!(draft.current_revision().number(), 7);
    assert_eq!(draft.revision(1), Some(&first));
    assert_eq!(first.exact_text(), request().text().as_str());
    assert_eq!(
        first.expiry().expires_at_unix_seconds(),
        1_000 + DEFAULT_EXPIRY_SECONDS
    );
    assert_eq!(
        draft
            .revisions()
            .iter()
            .map(|revision| revision.content_hash())
            .collect::<BTreeSet<_>>()
            .len(),
        7
    );

    let wrong_draft =
        DraftRequest::new("draft-2", "release", "body", "build_failed", "high").unwrap();
    assert_eq!(
        draft.revise(wrong_draft, &release, 1_000).unwrap_err(),
        DraftError::DraftIdMismatch
    );
    assert_eq!(draft.revisions().len(), 7);
}

#[test]
fn canonical_hash_is_stable_across_metadata_order_and_covers_approval_fields() {
    let config = resolved_config("acme/widgets", "200", true, false);
    let first_json = r#"{
        "draft_id":"draft-1",
        "destination_alias":"release",
        "text":"Build failed",
        "event_type":"build_failed",
        "severity":"high",
        "metadata":{"repository_label":"Widgets","branch":"main","commit":"abc1234"}
    }"#;
    let second_json = r#"{
        "metadata":{"commit":"abc1234","branch":"main","repository_label":"Widgets"},
        "severity":"high",
        "event_type":"build_failed",
        "text":"Build failed",
        "destination_alias":"release",
        "draft_id":"draft-1"
    }"#;

    let first = create_draft(
        serde_json::from_str(first_json).expect("first metadata order is valid"),
        &config,
        1_000,
    );
    let second = create_draft(
        serde_json::from_str(second_json).expect("second metadata order is valid"),
        &config,
        1_000,
    );
    assert_eq!(
        first.current_revision().canonical_json(),
        second.current_revision().canonical_json()
    );
    assert_eq!(
        first.current_revision().canonical_hash(),
        second.current_revision().canonical_hash()
    );

    let base = first.current_revision().content_hash().to_owned();
    let mut changed_hashes = BTreeSet::new();
    changed_hashes.insert(base.clone());

    let variants = [
        create_draft(
            DraftRequest::new("draft-1", "release", "Changed body", "build_failed", "high")
                .unwrap(),
            &config,
            1_000,
        ),
        create_draft(
            request().with_metadata(DraftMetadata::new().with_branch("feature/send").unwrap()),
            &config,
            1_000,
        ),
        create_draft(
            DraftRequest::new(
                "draft-1",
                "release",
                request().text().as_str(),
                "build_passed",
                "high",
            )
            .unwrap(),
            &config,
            1_000,
        ),
        create_draft(
            DraftRequest::new(
                "draft-1",
                "release",
                request().text().as_str(),
                "build_failed",
                "critical",
            )
            .unwrap(),
            &config,
            1_000,
        ),
        create_draft(
            DraftRequest::new(
                "draft-1",
                "staging",
                request().text().as_str(),
                "build_failed",
                "high",
            )
            .unwrap(),
            &config,
            1_000,
        ),
        create_draft(
            request(),
            &resolved_config("acme/widgets", "201", true, false),
            1_000,
        ),
        create_draft(
            request(),
            &resolved_config("other/widgets", "200", true, false),
            1_000,
        ),
        create_draft(request().with_expiry_seconds(3_600), &config, 1_000),
        create_draft(
            request(),
            &resolved_config("acme/widgets", "200", true, true),
            1_000,
        ),
        create_draft(
            request().with_reply_reference(reply_reference()),
            &resolved_config("acme/widgets", "200", true, false),
            1_000,
        ),
    ];

    for draft in variants {
        let hash = draft.current_revision().content_hash().to_owned();
        assert_ne!(hash, base, "approval-bound field did not change the hash");
        changed_hashes.insert(hash);
    }
    assert_eq!(changed_hashes.len(), 11);
}

#[test]
fn preview_returns_exact_snapshot_and_unresolved_bases_without_mutation() {
    let config = resolved_config("acme/widgets", "200", false, false);
    let draft = create_draft(
        request()
            .with_metadata(metadata())
            .with_expiry_seconds(100)
            .with_reply_reference(reply_reference()),
        &config,
        1_000,
    );
    let before = draft.clone();

    let preview = draft.preview(1, 1_050).expect("revision one exists");
    assert_eq!(preview.draft_id(), "draft-1");
    assert_eq!(preview.revision(), 1);
    assert_eq!(preview.repository_id(), "acme/widgets");
    assert_eq!(preview.destination_alias().as_str(), "release");
    assert_eq!(preview.resolved_destination().workspace_id, "100");
    assert_eq!(preview.resolved_destination().channel_id, "200");
    assert_eq!(preview.exact_text(), before.current_revision().exact_text());
    assert_eq!(preview.text_source(), ExactTextSource::StoredChannelText);
    assert_eq!(preview.metadata(), before.current_revision().metadata());
    assert_eq!(preview.expiry(), before.current_revision().expiry());
    assert_eq!(preview.bases(), DecisionBases::unresolved());
    assert_eq!(
        preview.bases().approval().authority(),
        BasisAuthority::HumanApproval
    );
    assert_eq!(
        preview.bases().approval().status(),
        BasisStatus::Unresolved {
            reason: UnresolvedBasisReason::NotEvaluatedByDraftModel
        }
    );
    assert_eq!(
        preview.bases().policy().authority(),
        BasisAuthority::ActivatedPolicy
    );
    assert_eq!(
        preview.bases().safety().authority(),
        BasisAuthority::SecretScanner
    );
    assert_eq!(
        preview.send_decision(),
        &SendDecision::Unresolved {
            blockers: Vec::new()
        }
    );
    assert_eq!(draft.preview(1, 1_050).unwrap(), preview);
    assert_eq!(draft, before);

    let rendered = draft
        .preview_with_rendered_text(1, "Build failed\n-- delivery-nonce", 1_050)
        .expect("the caller-rendered text is valid");
    assert_eq!(rendered.exact_text(), "Build failed\n-- delivery-nonce");
    assert_eq!(rendered.text_source(), ExactTextSource::CallerRenderedText);
    assert_eq!(draft, before);

    let expired = draft
        .preview(1, 1_100)
        .expect("expired previews remain inspectable");
    assert_eq!(
        expired.send_decision(),
        &SendDecision::Blocked {
            blocker: SendBlocker::RevisionExpired
        }
    );
    assert!(matches!(
        draft.ensure_current_unexpired(1_100),
        Err(DraftError::RevisionExpired { revision: 1, .. })
    ));
    assert_eq!(draft, before);
    assert!(matches!(
        draft.preview(2, 1_050),
        Err(DraftError::RevisionNotFound { revision: 2, .. })
    ));
}
