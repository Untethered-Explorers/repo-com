#![forbid(unsafe_code)]
#![doc = "Dedicated-bot Discord REST v10 authentication and read-only setup diagnostics."]

#[cfg(test)]
#[path = "../tests/discord_client_contract.rs"]
mod discord_client_contract;

mod auth;
mod client;
mod setup;

pub use auth::BOT_TOKEN_ENV;
pub use client::{
    ClientError, DISCORD_API_VERSION, DISCORD_BASE_URL, DiscordClient, RateLimitInfo,
    RateLimitScope, RequestOperation, token_environment_variable,
};
pub use setup::{
    ADMINISTRATOR_PERMISSION, BotIdentity, ChannelCheck, ChannelRole, MENTION_ROLES_PERMISSION,
    MentionCheck, READ_MESSAGE_HISTORY_PERMISSION, Remediation, RemediationAction,
    RequiredPermission, SEND_MESSAGES_PERMISSION, SetupIssue, SetupIssueKind, SetupReport,
    VIEW_CHANNEL_PERMISSION, WorkspaceMembership,
};
