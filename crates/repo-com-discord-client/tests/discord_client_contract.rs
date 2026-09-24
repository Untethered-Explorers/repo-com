use std::{path::Path, process::Command};

use repo_com_config::{ResolvedConfig, parse_config, resolve_model};
use serde_json::{Value, json};
use tokio::runtime::Builder;
use wiremock::{
    Mock, MockServer, Request, ResponseTemplate,
    matchers::{method, path},
};

use crate::{
    BOT_TOKEN_ENV, ChannelRole, ClientError, DiscordClient, RemediationAction, RequiredPermission,
    SetupIssueKind, SetupReport, WorkspaceMembership,
};

const CASE_ENV: &str = "REPO_COM_DISCORD_CLIENT_CONTRACT_CASE";
const BASE_URL_ENV: &str = "REPO_COM_DISCORD_CLIENT_CONTRACT_BASE_URL";
const SYNTHETIC_BOT_TOKEN: &str = "c3ludGhldGljLWNvbnRyYWN0LWJvdC10b2tlbi5zeW50aGV0aWMtc2lnbmF0dXJl.c3ludGhldGljLWNvbnRyYWN0.c2lnbmF0dXJl";
const SENSITIVE_RESPONSE_MARKER: &str =
    "synthetic-sensitive-response-body-must-never-enter-diagnostics";
const GUILD_ID: &str = "123456789012345678";
const CHANNEL_ID: &str = "234567890123456789";
const BOT_USER_ID: &str = "345678901234567890";
const USER_ID: &str = "567890123456789012";
const ROLE_ID: &str = "456789012345678901";
const VIEW_CHANNEL: u64 = 1 << 10;
const SEND_MESSAGES: u64 = 1 << 11;
const READ_MESSAGE_HISTORY: u64 = 1 << 16;
const MENTION_ROLES: u64 = 1 << 28;
const FULL_SETUP_PERMISSIONS: u64 =
    VIEW_CHANNEL | SEND_MESSAGES | READ_MESSAGE_HISTORY | MENTION_ROLES;

const CONFIG_TOML: &str = r#"
schema_version = 1
repository_id = "acme/widgets"

[discord]
workspace_id = "123456789012345678"

[destinations.release]
channel_id = "234567890123456789"
allowed_mentions = ["oncall"]

[mentions.oncall]
target = "role:456789012345678901"

[inbound.release]
enabled = true

[retention]
content_days = 30
metadata_days = 365

[[auto_send]]
event_type = "build_failed"
destination = "release"
severity = "high"
"#;

const USER_CONFIG_TOML: &str = r#"
schema_version = 1
repository_id = "acme/widgets"

[discord]
workspace_id = "123456789012345678"

[destinations.release]
channel_id = "234567890123456789"
allowed_mentions = ["owner"]

[mentions.owner]
target = "user:567890123456789012"

[inbound.release]
enabled = true

[retention]
content_days = 30
metadata_days = 365

[[auto_send]]
event_type = "build_failed"
destination = "release"
severity = "high"
"#;

#[test]
fn complete_setup_success_uses_only_v10_get_requests_with_bot_auth() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        mount_complete_setup(&server, FULL_SETUP_PERMISSIONS, true).await;

        run_child(&server, "success");

        assert_read_only_requests(&server).await;
    });
}

#[test]
fn non_bot_identity_is_rejected_as_a_dedicated_bot_requirement() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        mount_identity(&server, false).await;

        run_child(&server, "identity-not-bot");

        assert_read_only_requests(&server).await;
    });
}

#[test]
fn missing_workspace_membership_returns_manual_remediation() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        mount_identity(&server, true).await;
        mount_status(
            &server,
            &membership_path(),
            404,
            json!({"message": "synthetic missing guild"}),
        )
        .await;

        run_child(&server, "missing-guild");

        assert_read_only_requests(&server).await;
    });
}

#[test]
fn missing_configured_channel_is_reported_without_mutation() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        mount_identity(&server, true).await;
        mount_membership(&server).await;
        mount_guild(&server, FULL_SETUP_PERMISSIONS, true).await;
        mount_status(
            &server,
            &channel_path(),
            404,
            json!({"message": "synthetic missing channel"}),
        )
        .await;

        run_child(&server, "missing-channel");

        assert_read_only_requests(&server).await;
    });
}

#[test]
fn missing_view_channel_permission_is_reported() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        mount_complete_setup(&server, FULL_SETUP_PERMISSIONS & !VIEW_CHANNEL, true).await;

        run_child(&server, "missing-view");

        assert_read_only_requests(&server).await;
    });
}

#[test]
fn hidden_channel_403_is_reported_as_missing_visibility() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        mount_identity(&server, true).await;
        mount_membership(&server).await;
        mount_guild(&server, FULL_SETUP_PERMISSIONS, true).await;
        mount_status(
            &server,
            &channel_path(),
            403,
            json!({"message": "synthetic hidden channel"}),
        )
        .await;

        run_child(&server, "hidden-channel");

        assert_read_only_requests(&server).await;
    });
}

#[test]
fn missing_send_messages_permission_is_reported() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        mount_complete_setup(&server, FULL_SETUP_PERMISSIONS & !SEND_MESSAGES, true).await;

        run_child(&server, "missing-send");

        assert_read_only_requests(&server).await;
    });
}

#[test]
fn missing_read_message_history_permission_is_reported_for_inbound() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        mount_complete_setup(
            &server,
            FULL_SETUP_PERMISSIONS & !READ_MESSAGE_HISTORY,
            true,
        )
        .await;

        run_child(&server, "missing-read");

        assert_read_only_requests(&server).await;
    });
}

#[test]
fn mention_denial_is_reported_separately_from_channel_permissions() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        mount_complete_setup(&server, FULL_SETUP_PERMISSIONS & !MENTION_ROLES, true).await;

        run_child(&server, "mention-denied");

        assert_read_only_requests(&server).await;
    });
}

#[test]
fn unmentionable_role_is_reported_separately_from_role_permission() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        mount_complete_setup(&server, FULL_SETUP_PERMISSIONS, false).await;

        run_child(&server, "role-unmentionable");

        assert_read_only_requests(&server).await;
    });
}

#[test]
fn missing_user_mention_member_is_reported_without_exposing_member_content() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        mount_complete_setup(&server, FULL_SETUP_PERMISSIONS, true).await;
        mount_status(
            &server,
            &user_member_path(),
            404,
            json!({"message": "synthetic missing mention member"}),
        )
        .await;

        run_child(&server, "user-mention-missing");

        assert_read_only_requests(&server).await;
    });
}

#[test]
fn authentication_failure_requires_rotation_and_redacts_token_and_body() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        mount_status(
            &server,
            "/api/v10/users/@me",
            401,
            json!({
                "message": SENSITIVE_RESPONSE_MARKER,
                "credential_echo": SENSITIVE_RESPONSE_MARKER
            }),
        )
        .await;

        run_child(&server, "authentication-failure");

        assert_read_only_requests(&server).await;
    });
}

#[test]
fn bearer_user_credentials_are_rejected_before_http_io() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        run_child_with_token("bearer-user-rejected", "Bearer user-token-value");
        assert!(
            server
                .received_requests()
                .await
                .expect("request journal")
                .is_empty()
        );
    });
}

#[test]
fn missing_environment_token_is_typed_and_contains_no_credential() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        run_child_without_token("missing-token");
        assert!(
            server
                .received_requests()
                .await
                .expect("request journal")
                .is_empty()
        );
    });
}

#[test]
fn unpinned_api_versions_and_non_loopback_test_endpoints_are_rejected() {
    runtime().block_on(async {
        let server = MockServer::start().await;
        run_child_with_base("unpinned-version", "https://discord.com/api/v9");
        run_child_with_base("non-loopback-endpoint", "https://example.com");
        assert!(
            server
                .received_requests()
                .await
                .expect("request journal")
                .is_empty()
        );
    });
}

#[test]
#[ignore = "subprocess fixture for environment-only credential tests"]
fn contract_child_process() {
    let Ok(case) = std::env::var(CASE_ENV) else {
        return;
    };
    let base_url = std::env::var(BASE_URL_ENV).expect("child base URL");
    runtime().block_on(execute_child_case(&case, &base_url));
}

async fn execute_child_case(case: &str, base_url: &str) {
    match case {
        "success" => {
            let report = run_setup().await;
            assert!(report.ready());
            assert_eq!(report.bot.user_id, BOT_USER_ID);
            assert!(report.bot.dedicated_bot);
            assert_eq!(report.workspace_membership, WorkspaceMembership::Member);
            assert_eq!(report.channels.len(), 1);
            assert!(report.channels[0].ready);
            assert!(report.channels[0].missing_permissions.is_empty());
            assert_eq!(report.mentions.len(), 1);
            assert!(report.mentions[0].ready);
        }
        "identity-not-bot" => {
            let report = run_setup().await;
            assert!(!report.ready());
            assert!(!report.bot.dedicated_bot);
            assert!(
                report
                    .issues
                    .iter()
                    .any(|issue| { issue.kind == SetupIssueKind::IdentityNotDedicatedBot })
            );
        }
        "missing-guild" => {
            let report = run_setup().await;
            assert!(!report.ready());
            assert_eq!(report.workspace_membership, WorkspaceMembership::Missing);
            assert!(
                report
                    .issues
                    .iter()
                    .any(|issue| { issue.kind == SetupIssueKind::WorkspaceMembershipMissing })
            );
        }
        "missing-channel" => {
            let report = run_setup().await;
            assert!(!report.ready());
            assert!(report.issues.iter().any(|issue| {
                matches!(
                    issue.kind,
                    SetupIssueKind::ChannelUnavailable { ref role, .. }
                        if role == &ChannelRole::Destination
                )
            }));
        }
        "missing-view" => {
            assert_missing_permission("missing-view", RequiredPermission::ViewChannel).await;
        }
        "hidden-channel" => {
            assert_missing_permission("hidden-channel", RequiredPermission::ViewChannel).await;
        }
        "missing-send" => {
            assert_missing_permission("missing-send", RequiredPermission::SendMessages).await;
        }
        "missing-read" => {
            assert_missing_permission("missing-read", RequiredPermission::ReadMessageHistory).await;
        }
        "mention-denied" => {
            let report = run_setup().await;
            assert!(!report.ready());
            assert!(report.channels[0].ready);
            assert!(!report.mentions[0].ready);
            assert!(report.issues.iter().any(|issue| {
                matches!(
                    issue.kind,
                    SetupIssueKind::MentionRolePermissionMissing { .. }
                )
            }));
        }
        "role-unmentionable" => {
            let report = run_setup().await;
            assert!(!report.ready());
            assert!(report.channels[0].ready);
            assert!(!report.mentions[0].ready);
            assert!(report.issues.iter().any(|issue| {
                matches!(issue.kind, SetupIssueKind::MentionRoleNotMentionable { .. })
            }));
        }
        "user-mention-missing" => {
            let report = run_setup_with(&user_config()).await;
            assert!(!report.ready());
            assert!(report.channels[0].ready);
            assert!(!report.mentions[0].ready);
            assert!(report.issues.iter().any(|issue| {
                matches!(
                    issue.kind,
                    SetupIssueKind::MentionTargetUnavailable {
                        target_kind: repo_com_config::MentionKind::User,
                        ..
                    }
                )
            }));
        }
        "authentication-failure" => {
            let client = test_client();
            let error = client
                .check_setup(&config())
                .await
                .expect_err("synthetic authentication failure");
            assert_eq!(error, ClientError::AuthenticationFailed);
            let remediation = error.remediation();
            assert_eq!(remediation.action, RemediationAction::RotateBotToken);
            assert!(remediation.instruction.contains(BOT_TOKEN_ENV));
            let foundation = error.to_repo_com_error();
            let diagnostics = format!(
                "{client:?} {error:?} {error} {remediation:?} {foundation:?} {}",
                serde_json::to_string(&remediation).expect("safe remediation JSON")
            );
            assert!(!diagnostics.contains(SYNTHETIC_BOT_TOKEN));
            assert!(!diagnostics.contains(SENSITIVE_RESPONSE_MARKER));
        }
        "bearer-user-rejected" => {
            let error = DiscordClient::from_environment_for_test_server(base_url)
                .expect_err("Bearer credential must fail before transport use");
            assert_eq!(error, ClientError::InvalidBotToken);
            assert_eq!(
                error.remediation().action,
                RemediationAction::UseRawDedicatedBotToken
            );
        }
        "missing-token" => {
            let error = DiscordClient::from_environment_for_test_server(base_url)
                .expect_err("missing environment token must fail before transport use");
            assert_eq!(error, ClientError::MissingBotToken);
            assert_eq!(
                error.remediation().action,
                RemediationAction::SetBotTokenEnvironment
            );
        }
        "unpinned-version" | "non-loopback-endpoint" => {
            let error = DiscordClient::from_environment_for_test_server(base_url)
                .expect_err("unapproved endpoint must fail before transport use");
            assert_eq!(error, ClientError::InvalidTestBaseUrl);
            assert_eq!(
                error.remediation().action,
                RemediationAction::UseOfficialEndpoint
            );
        }
        other => panic!("unknown child case {other}"),
    }
}

async fn assert_missing_permission(case: &str, permission: RequiredPermission) {
    let report = run_setup().await;
    assert!(!report.ready(), "{case}");
    assert!(report.issues.iter().any(|issue| {
        matches!(
            issue.kind,
            SetupIssueKind::MissingPermission {
                permission: actual,
                ..
            } if actual == permission
        )
    }));
    assert!(report.channels[0].missing_permissions.contains(&permission));
}

async fn run_setup() -> SetupReport {
    run_setup_with(&config()).await
}

async fn run_setup_with(config: &ResolvedConfig) -> SetupReport {
    test_client()
        .check_setup(config)
        .await
        .expect("synthetic setup response")
}

fn test_client() -> DiscordClient {
    let base_url = std::env::var(BASE_URL_ENV).expect("child base URL");
    DiscordClient::from_environment_for_test_server(&base_url).expect("synthetic test client")
}

fn config() -> ResolvedConfig {
    resolve_config_source(CONFIG_TOML)
}

fn user_config() -> ResolvedConfig {
    resolve_config_source(USER_CONFIG_TOML)
}

fn resolve_config_source(source: &str) -> ResolvedConfig {
    let parsed = parse_config(Path::new("discord-client-contract.toml"), source)
        .expect("synthetic valid config");
    resolve_model(&parsed, Path::new("discord-client-contract.toml"))
        .expect("synthetic resolved config")
}

fn runtime() -> tokio::runtime::Runtime {
    Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test Tokio runtime")
}

fn run_child(server: &MockServer, case: &str) {
    let output = child_command(case)
        .env(BASE_URL_ENV, server.uri())
        .output()
        .expect("run setup child");
    assert_child_success(output);
}

async fn mount_complete_setup(server: &MockServer, permissions: u64, mentionable: bool) {
    mount_identity(server, true).await;
    mount_membership(server).await;
    mount_guild(server, permissions, mentionable).await;
    mount_channel(server).await;
}

async fn mount_identity(server: &MockServer, bot: bool) {
    mount_status(
        server,
        "/api/v10/users/@me",
        200,
        json!({
            "id": BOT_USER_ID,
            "username": "synthetic-repo-com-bot",
            "bot": bot
        }),
    )
    .await;
}

async fn mount_membership(server: &MockServer) {
    mount_status(
        server,
        &membership_path(),
        200,
        json!({"roles": [], "joined_at": "2026-01-01T00:00:00Z"}),
    )
    .await;
}

async fn mount_guild(server: &MockServer, permissions: u64, mentionable: bool) {
    mount_status(
        server,
        &guild_path(),
        200,
        json!({
            "id": GUILD_ID,
            "name": "synthetic-workspace",
            "roles": [
                {
                    "id": GUILD_ID,
                    "name": "@everyone",
                    "permissions": permissions.to_string(),
                    "mentionable": false
                },
                {
                    "id": ROLE_ID,
                    "name": "oncall",
                    "permissions": "0",
                    "mentionable": mentionable
                }
            ]
        }),
    )
    .await;
}

async fn mount_channel(server: &MockServer) {
    mount_status(
        server,
        &channel_path(),
        200,
        json!({
            "id": CHANNEL_ID,
            "guild_id": GUILD_ID,
            "type": 0,
            "permission_overwrites": []
        }),
    )
    .await;
}

async fn mount_status(server: &MockServer, request_path: &str, status: u16, body: Value) {
    Mock::given(method("GET"))
        .and(path(request_path))
        .respond_with(ResponseTemplate::new(status).set_body_json(body))
        .mount(server)
        .await;
}

async fn assert_read_only_requests(server: &MockServer) {
    let requests = server.received_requests().await.expect("request journal");
    assert!(!requests.is_empty());
    for request in requests {
        assert_allowed_request(&request);
    }
}

fn assert_allowed_request(request: &Request) {
    assert_eq!(request.method, reqwest::Method::GET);
    let request_path = request.url.path();
    assert!(request_path.starts_with("/api/v10/"), "{request_path}");
    assert!(matches!(
        request_path,
        "/api/v10/users/@me"
            | "/api/v10/users/@me/guilds/123456789012345678/member"
            | "/api/v10/guilds/123456789012345678"
            | "/api/v10/channels/234567890123456789"
            | "/api/v10/guilds/123456789012345678/members/567890123456789012"
    ));
    assert_eq!(
        request
            .headers
            .get("authorization")
            .and_then(|value| value.to_str().ok()),
        Some(format!("Bot {SYNTHETIC_BOT_TOKEN}").as_str())
    );
    assert!(
        request
            .headers
            .get("user-agent")
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("DiscordBot "))
    );
    assert!(request.body.is_empty());
}

fn membership_path() -> String {
    format!("/api/v10/users/@me/guilds/{GUILD_ID}/member")
}

fn guild_path() -> String {
    format!("/api/v10/guilds/{GUILD_ID}")
}

fn channel_path() -> String {
    format!("/api/v10/channels/{CHANNEL_ID}")
}

fn user_member_path() -> String {
    format!("/api/v10/guilds/{GUILD_ID}/members/{USER_ID}")
}

fn child_command(case: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("current test executable"));
    command
        .arg("--ignored")
        .arg("--nocapture")
        .arg("contract_child_process")
        .env(CASE_ENV, case)
        .env(BOT_TOKEN_ENV, SYNTHETIC_BOT_TOKEN);
    command
}

fn run_child_with_token(case: &str, token: &str) {
    let output = child_command(case)
        .env(BASE_URL_ENV, "http://127.0.0.1:1")
        .env(BOT_TOKEN_ENV, token)
        .output()
        .expect("run credential child");
    assert_child_success(output);
}

fn run_child_without_token(case: &str) {
    let output = child_command(case)
        .env_remove(BOT_TOKEN_ENV)
        .env(BASE_URL_ENV, "http://127.0.0.1:1")
        .output()
        .expect("run missing-token child");
    assert_child_success(output);
}

fn run_child_with_base(case: &str, base_url: &str) {
    let output = child_command(case)
        .env(BASE_URL_ENV, base_url)
        .output()
        .expect("run endpoint child");
    assert_child_success(output);
}

fn assert_child_success(output: std::process::Output) {
    assert!(
        output.status.success(),
        "child failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
