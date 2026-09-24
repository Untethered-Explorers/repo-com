use std::collections::BTreeMap;
use std::path::Path;

use repo_com_config::{
    DestinationConfig, DiscordConfig, MentionConfig, MentionKind, RepositoryConfig, ResolvedConfig,
    RetentionConfig, resolve_model,
};

use crate::{
    ContentRenderer, DecisionBases, DeliveryNonce, ExactTextSource, MAX_DISCORD_MESSAGE_CHARACTERS,
    MentionError, NONCE_FOOTER_PREFIX, RenderError, contains_nonce_marker, normalize_text,
};

const CREATED_AT: u64 = 1_000;
const REPOSITORY_ID: &str = "acme/widgets";
const WORKSPACE_ID: &str = "100";
const CHANNEL_ID: &str = "200";
const ROLE_ID: &str = "300";
const USER_ID: &str = "400";
const BOT_USER_ID: &str = "999";
const SNAPSHOT_HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn resolved_config(include_user: bool, include_bot: bool) -> ResolvedConfig {
    let mut allowed_mentions = vec!["oncall".to_owned()];
    if include_user {
        allowed_mentions.push("alice".to_owned());
    }
    if include_bot {
        allowed_mentions.push("bot".to_owned());
    }

    let mut destinations = BTreeMap::new();
    destinations.insert(
        "release".to_owned(),
        DestinationConfig {
            channel_id: CHANNEL_ID.to_owned(),
            allowed_mentions,
        },
    );

    let mut mentions = BTreeMap::new();
    mentions.insert(
        "oncall".to_owned(),
        MentionConfig {
            target: format!("role:{ROLE_ID}"),
        },
    );
    if include_user {
        mentions.insert(
            "alice".to_owned(),
            MentionConfig {
                target: format!("user:{USER_ID}"),
            },
        );
    }
    if include_bot {
        mentions.insert(
            "bot".to_owned(),
            MentionConfig {
                target: format!("user:{BOT_USER_ID}"),
            },
        );
    }

    resolve_model(
        &RepositoryConfig {
            schema_version: 1,
            repository_id: REPOSITORY_ID.to_owned(),
            discord: DiscordConfig {
                workspace_id: WORKSPACE_ID.to_owned(),
            },
            destinations,
            mentions,
            inbound: BTreeMap::new(),
            retention: RetentionConfig::default(),
            auto_send: Vec::new(),
        },
        Path::new("/test/.repo-com.toml"),
    )
    .expect("the synthetic configuration is valid")
}

fn request(text: &str) -> repo_com_draft_model::DraftRequest {
    repo_com_draft_model::DraftRequest::new("draft-1", "release", text, "build_failed", "high")
        .expect("the synthetic draft request is valid")
}

fn create_draft(text: &str, config: &ResolvedConfig) -> repo_com_draft_model::DraftModel {
    repo_com_draft_model::DraftModel::create(request(text), config, CREATED_AT)
        .expect("the synthetic draft is valid")
}

fn reply_reference() -> repo_com_draft_model::AuthorizedReplyReference {
    repo_com_draft_model::AuthorizedReplyReference::from_validated_target(
        REPOSITORY_ID,
        WORKSPACE_ID,
        CHANNEL_ID,
        "inbound-1",
        "450",
        "authorization-1",
        SNAPSHOT_HASH,
    )
    .expect("the reply evidence is valid")
}

#[test]
fn identical_revision_renders_byte_identical_text_and_nonce() {
    let config = resolved_config(true, false);
    let draft = create_draft("Build failed", &config);
    let before = draft.clone();
    let renderer = ContentRenderer::new();

    let first = renderer
        .render_revision(draft.current_revision(), CREATED_AT)
        .expect("the first render succeeds");
    let second = renderer
        .render_revision(draft.current_revision(), CREATED_AT)
        .expect("the second render succeeds");

    assert_eq!(first, second);
    assert_eq!(first.exact_text(), second.exact_text());
    assert_eq!(first.nonce(), second.nonce());
    assert_eq!(first.nonce(), draft.current_revision().content_hash());
    assert!(first.exact_text().ends_with(&first.nonce_footer()));
    let preview = first
        .preview(&draft, CREATED_AT)
        .expect("the rendered revision can be projected into preview");
    assert_eq!(preview.exact_text(), first.exact_text());
    assert_eq!(preview.text_source(), ExactTextSource::CallerRenderedText);
    assert_eq!(preview.bases(), DecisionBases::unresolved());
    assert_eq!(draft, before);
}

#[test]
fn named_role_and_user_aliases_resolve_only_from_the_allowlist() {
    let config = resolved_config(true, false);
    let draft = create_draft("@oncall @alice", &config);
    let rendered = ContentRenderer::new()
        .render_current(&draft, CREATED_AT)
        .expect("allowlisted mentions resolve");

    assert!(rendered.exact_text().starts_with("<@&300> <@400>\n\n"));
    let punctuated = create_draft("@oncall.", &config);
    let punctuated_rendered = ContentRenderer::new()
        .render_current(&punctuated, CREATED_AT)
        .expect("a sentence-ending period remains outside the alias");
    assert!(punctuated_rendered.exact_text().starts_with("<@&300>."));
    assert_eq!(rendered.mentions().len(), 2);
    assert_eq!(rendered.mentions()[0].alias(), "oncall");
    assert_eq!(rendered.mentions()[0].kind(), MentionKind::Role);
    assert_eq!(rendered.mentions()[1].alias(), "alice");
    assert_eq!(rendered.mentions()[1].kind(), MentionKind::User);
    assert_eq!(rendered.allowed_mentions().len(), 2);
    assert_eq!(rendered.allowed_mention_targets().len(), 2);
    assert!(rendered.message_reference().is_none());
}

#[test]
fn raw_unlisted_malformed_and_user_bot_mentions_fail_closed() {
    let config = resolved_config(true, true);
    let renderer = ContentRenderer::new();

    let unlisted = create_draft("@unknown", &config);
    assert!(matches!(
        renderer.render_current(&unlisted, CREATED_AT),
        Err(RenderError::Mention(MentionError::UnlistedAlias { .. }))
    ));

    for body in [
        "<@300>", "<@!300>", "<@&300>", "<#200>", "role:300", "user:400",
    ] {
        let draft = create_draft(body, &config);
        assert!(
            matches!(
                renderer.render_current(&draft, CREATED_AT),
                Err(RenderError::Mention(MentionError::RawDiscordMention))
            ),
            "raw mention unexpectedly rendered: {body}"
        );
    }

    for body in ["@oncall!", "@oncallé", "{{oncall}}"] {
        let draft = create_draft(body, &config);
        assert!(renderer.render_current(&draft, CREATED_AT).is_err());
    }

    let bot_renderer =
        ContentRenderer::for_bot_user_id(BOT_USER_ID).expect("the synthetic bot ID is valid");
    let bot_draft = create_draft("@bot", &config);
    assert!(matches!(
        bot_renderer.render_current(&bot_draft, CREATED_AT),
        Err(RenderError::Mention(MentionError::UserBotMention { .. }))
    ));
}

#[test]
fn normalization_handles_crlf_unicode_and_trailing_whitespace_without_copy_expansion() {
    let config = resolved_config(false, false);
    let input = "  Cafe\u{301} 🚀\r\nsecond\t \r\n";
    let normalized = normalize_text(input).expect("the body has semantic text");
    assert_eq!(normalized, "  Café 🚀\nsecond");

    let draft = create_draft(input, &config);
    let rendered = ContentRenderer::new()
        .render_current(&draft, CREATED_AT)
        .expect("normalized text renders");
    assert_eq!(rendered.normalized_body(), normalized);
    assert!(rendered.exact_text().starts_with(&normalized));
    assert!(!rendered.exact_text().contains("repository"));
    assert!(!rendered.exact_text().contains("branch"));
    assert!(!rendered.exact_text().contains("commit"));

    let email_draft = create_draft("Write to dev@example.com", &config);
    let email = ContentRenderer::new()
        .render_current(&email_draft, CREATED_AT)
        .expect("ordinary text remains text");
    assert!(email.exact_text().starts_with("Write to dev@example.com"));
}

#[test]
fn already_rendered_nonce_text_is_rejected_and_never_rendered_twice() {
    let config = resolved_config(false, false);
    let draft = create_draft("Build failed", &config);
    let renderer = ContentRenderer::new();
    let first = renderer
        .render_current(&draft, CREATED_AT)
        .expect("the first render succeeds");

    assert!(contains_nonce_marker(first.exact_text()));
    let second_draft = create_draft(first.exact_text(), &config);
    assert_eq!(
        renderer
            .render_current(&second_draft, CREATED_AT)
            .unwrap_err(),
        RenderError::NonceAlreadyRendered
    );
    assert_eq!(first.nonce_footer().matches(NONCE_FOOTER_PREFIX).count(), 1);
}

#[test]
fn exact_discord_boundary_is_inclusive_and_footer_is_counted() {
    let config = resolved_config(false, false);
    let overhead = NONCE_FOOTER_PREFIX.chars().count() + 64;
    let maximum_body = "a".repeat(MAX_DISCORD_MESSAGE_CHARACTERS - overhead);
    let maximum = create_draft(&maximum_body, &config);
    let rendered = ContentRenderer::new()
        .render_current(&maximum, CREATED_AT)
        .expect("the exact Discord maximum succeeds");
    assert_eq!(
        rendered.exact_text().chars().count(),
        MAX_DISCORD_MESSAGE_CHARACTERS
    );

    let over_body = "a".repeat(MAX_DISCORD_MESSAGE_CHARACTERS - overhead + 1);
    let over = create_draft(&over_body, &config);
    assert_eq!(
        ContentRenderer::new()
            .render_current(&over, CREATED_AT)
            .unwrap_err(),
        RenderError::MessageTooLong {
            length: MAX_DISCORD_MESSAGE_CHARACTERS + 1,
            maximum: MAX_DISCORD_MESSAGE_CHARACTERS,
        }
    );
}

#[test]
fn validated_reply_reference_is_metadata_not_body_text() {
    let config = resolved_config(false, false);
    let request = request("A normal reply").with_reply_reference(reply_reference());
    let draft = repo_com_draft_model::DraftModel::create(request, &config, CREATED_AT)
        .expect("the reply draft is valid");
    let rendered = ContentRenderer::new()
        .render_current(&draft, CREATED_AT)
        .expect("the reply draft renders");

    assert!(!rendered.exact_text().contains("450"));
    assert_eq!(
        rendered
            .message_reference()
            .expect("the reply reference is carried")
            .message_id(),
        "450"
    );
    assert_eq!(
        rendered
            .request_metadata()
            .message_reference()
            .expect("request metadata carries the reference")
            .message_id(),
        "450"
    );
}

#[test]
fn expired_revisions_are_rejected_before_rendering() {
    let config = resolved_config(false, false);
    let request = request("Expires").with_expiry_seconds(1);
    let draft = repo_com_draft_model::DraftModel::create(request, &config, CREATED_AT)
        .expect("the expiring draft is valid");
    let renderer = ContentRenderer::new();

    assert!(renderer.render_current(&draft, CREATED_AT).is_ok());
    assert_eq!(
        renderer.render_current(&draft, CREATED_AT + 1).unwrap_err(),
        RenderError::RevisionExpired { revision: 1 }
    );
}

#[test]
fn nonce_is_a_validated_immutable_projection() {
    let config = resolved_config(false, false);
    let draft = create_draft("Body", &config);
    let nonce = DeliveryNonce::from_revision(draft.current_revision())
        .expect("the revision hash is a valid nonce");
    assert_eq!(nonce.as_str(), draft.current_revision().content_hash());
    assert_eq!(
        nonce.footer(),
        format!("{NONCE_FOOTER_PREFIX}{}", nonce.as_str())
    );
    assert!(DeliveryNonce::new("not-a-hash").is_err());
}
