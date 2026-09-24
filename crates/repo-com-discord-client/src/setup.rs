use std::collections::{BTreeMap, BTreeSet};

use repo_com_config::{MentionKind, ResolvedConfig};
use serde::{Deserialize, Serialize};

use crate::client::{ClientError, DiscordClient, GetOutcome, RequestOperation};

/// `ADMINISTRATOR`; it bypasses channel permission overwrites.
pub const ADMINISTRATOR_PERMISSION: u64 = 1 << 3;
/// `VIEW_CHANNEL`.
pub const VIEW_CHANNEL_PERMISSION: u64 = 1 << 10;
/// `SEND_MESSAGES`.
pub const SEND_MESSAGES_PERMISSION: u64 = 1 << 11;
/// `READ_MESSAGE_HISTORY`.
pub const READ_MESSAGE_HISTORY_PERMISSION: u64 = 1 << 16;
/// Discord's role-mention bit, numerically shared with `MANAGE_ROLES` in the
/// current permission table.
pub const MENTION_ROLES_PERMISSION: u64 = 1 << 28;

const SETUP_PERMISSION_MASK: u64 = VIEW_CHANNEL_PERMISSION
    | SEND_MESSAGES_PERMISSION
    | READ_MESSAGE_HISTORY_PERMISSION
    | MENTION_ROLES_PERMISSION;

/// A channel permission checked by setup.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RequiredPermission {
    /// See configured channels.
    ViewChannel,
    /// Send the product's one text message.
    SendMessages,
    /// Read replies and mentions on enabled inbound aliases.
    ReadMessageHistory,
    /// Mention an allowlisted, mentionable role.
    MentionRoles,
}

impl RequiredPermission {
    const fn bit(self) -> u64 {
        match self {
            Self::ViewChannel => VIEW_CHANNEL_PERMISSION,
            Self::SendMessages => SEND_MESSAGES_PERMISSION,
            Self::ReadMessageHistory => READ_MESSAGE_HISTORY_PERMISSION,
            Self::MentionRoles => MENTION_ROLES_PERMISSION,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::ViewChannel => "VIEW_CHANNEL",
            Self::SendMessages => "SEND_MESSAGES",
            Self::ReadMessageHistory => "READ_MESSAGE_HISTORY",
            Self::MentionRoles => "MENTION_ROLES",
        }
    }
}

/// The configured role of one channel in setup output.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelRole {
    /// A configured outbound destination.
    Destination,
    /// An enabled inbound alias.
    EnabledInbound,
}

/// The authenticated identity proven by `GET /users/@me`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BotIdentity {
    /// Discord's numeric bot user ID.
    pub user_id: String,
    /// Whether Discord marked the identity as a bot.
    pub dedicated_bot: bool,
}

/// Workspace membership result.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceMembership {
    /// Membership was not checked because bot identity validation failed.
    Unchecked,
    /// The bot is a member of the configured workspace.
    Member,
    /// The bot is not a member of the configured workspace.
    Missing,
}

/// A safe, alias-based summary of one unique channel check.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ChannelCheck {
    /// Outbound aliases resolving to this channel.
    pub destination_aliases: Vec<String>,
    /// Enabled inbound aliases resolving to this channel.
    pub inbound_aliases: Vec<String>,
    /// Permissions required by those aliases.
    pub required_permissions: Vec<RequiredPermission>,
    /// Required permissions present in the computed bitfield.
    pub granted_permissions: Vec<RequiredPermission>,
    /// Required permissions absent from the computed bitfield.
    pub missing_permissions: Vec<RequiredPermission>,
    /// Whether Discord returned the channel and it belongs to the workspace.
    pub visible_in_workspace: bool,
    /// Whether all requirements for the aliases passed.
    pub ready: bool,
}

/// A safe result for one configured mention alias.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct MentionCheck {
    /// Destination whose allowlist contains the mention.
    pub destination_alias: String,
    /// Repository-local mention alias.
    pub mention_alias: String,
    /// Role or user target kind.
    pub target_kind: MentionKind,
    /// Whether the target can be mentioned under current guild rules.
    pub ready: bool,
}

/// A stable setup issue kind that contains aliases but no remote content.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupIssueKind {
    /// The authenticated identity is not a dedicated bot.
    IdentityNotDedicatedBot,
    /// The bot cannot access the configured workspace as a member.
    WorkspaceMembershipMissing,
    /// A configured channel cannot be read.
    ChannelUnavailable {
        /// Repository-local alias.
        alias: String,
        /// How the alias uses the channel.
        role: ChannelRole,
    },
    /// A channel belongs to another workspace.
    ChannelOutsideWorkspace {
        /// Repository-local alias.
        alias: String,
        /// How the alias uses the channel.
        role: ChannelRole,
    },
    /// A required channel permission is absent.
    MissingPermission {
        /// Repository-local alias.
        alias: String,
        /// How the alias uses the channel.
        role: ChannelRole,
        /// Missing permission.
        permission: RequiredPermission,
    },
    /// A configured mention target is absent from the workspace.
    MentionTargetUnavailable {
        /// Destination alias.
        destination_alias: String,
        /// Mention alias.
        mention_alias: String,
        /// Target kind.
        target_kind: MentionKind,
    },
    /// A role exists but is not mentionable.
    MentionRoleNotMentionable {
        /// Destination alias.
        destination_alias: String,
        /// Mention alias.
        mention_alias: String,
    },
    /// A mentionable role cannot be mentioned because the bot lacks the role
    /// mention permission on the destination.
    MentionRolePermissionMissing {
        /// Destination alias.
        destination_alias: String,
        /// Mention alias.
        mention_alias: String,
    },
    /// The bot itself was configured as a user mention target.
    MentionTargetIsBot {
        /// Destination alias.
        destination_alias: String,
        /// Mention alias.
        mention_alias: String,
    },
}

/// A machine-readable remediation action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RemediationAction {
    /// Set the dedicated token environment variable.
    SetBotTokenEnvironment,
    /// Replace the value with a raw dedicated bot token.
    UseRawDedicatedBotToken,
    /// Rotate the dedicated bot token.
    RotateBotToken,
    /// Use the fixed official Discord origin.
    UseOfficialEndpoint,
    /// Review Discord read access for a safe operation.
    ReviewRemoteReadAccess,
    /// Correct a configured alias after a missing resource.
    CorrectConfiguredResource,
    /// Retry a safe read later.
    RetrySetup,
    /// Wait for the server-provided rate-limit duration.
    RetryAfterRateLimit,
    /// Grant one missing channel permission manually.
    GrantMissingPermission,
    /// Correct or remove an unavailable mention target.
    CorrectMentionTarget,
    /// Use a Discord role whose `mentionable` flag is true.
    UseMentionableRole,
    /// Review current Discord API compatibility.
    ReviewApiCompatibility,
    /// Invite the dedicated bot to the configured workspace manually.
    InviteBotToWorkspace,
    /// Replace a user credential with a dedicated bot identity.
    UseDedicatedBotIdentity,
}

/// Structured, non-mutating operator guidance.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Remediation {
    /// Stable action code for a command or terminal layer.
    pub action: RemediationAction,
    /// Safe instruction that contains no token or response body.
    pub instruction: String,
}

impl Remediation {
    pub(crate) fn set_bot_token() -> Self {
        Self::new(
            RemediationAction::SetBotTokenEnvironment,
            "Set REPO_COM_DISCORD_TOKEN to a raw dedicated Discord bot token and retry setup.",
        )
    }

    pub(crate) fn use_raw_dedicated_bot_token() -> Self {
        Self::new(
            RemediationAction::UseRawDedicatedBotToken,
            "Replace the credential with the raw token from the dedicated Discord bot page; do not include Bot, Bearer, or Basic.",
        )
    }

    pub(crate) fn rotate_bot_token() -> Self {
        Self::new(
            RemediationAction::RotateBotToken,
            "Rotate the dedicated bot token in REPO_COM_DISCORD_TOKEN and retry setup.",
        )
    }

    pub(crate) fn use_official_endpoint() -> Self {
        Self::new(
            RemediationAction::UseOfficialEndpoint,
            "Use the fixed official Discord REST v10 endpoint; loopback test endpoints must be origin-only HTTP URLs.",
        )
    }

    pub(crate) fn review_remote_read_access(operation: RequestOperation) -> Self {
        Self::new(
            RemediationAction::ReviewRemoteReadAccess,
            match operation {
                RequestOperation::CurrentBotIdentity => {
                    "Review the dedicated bot credential and retry identity validation.".to_owned()
                }
                RequestOperation::GuildMembership => {
                    "Invite the dedicated bot to the configured workspace, then retry setup.".to_owned()
                }
                RequestOperation::Guild => {
                    "Ask a workspace administrator to verify the bot's read access, then retry setup."
                        .to_owned()
                }
                RequestOperation::Channel => {
                    "Ask a workspace administrator to restore the bot's read access to the configured channel."
                        .to_owned()
                }
                RequestOperation::GuildMember => {
                    "Ask a workspace administrator to restore read access for mention validation."
                        .to_owned()
                }
            },
        )
    }

    pub(crate) fn correct_configured_resource(operation: RequestOperation) -> Self {
        Self::new(
            RemediationAction::CorrectConfiguredResource,
            match operation {
                RequestOperation::Guild | RequestOperation::GuildMembership => {
                    "Correct the configured workspace alias or invite the bot, then retry setup."
                        .to_owned()
                }
                RequestOperation::Channel => {
                    "Correct the configured destination or inbound channel alias, then retry setup."
                        .to_owned()
                }
                RequestOperation::GuildMember => {
                    "Correct the configured user mention alias, then retry setup.".to_owned()
                }
                RequestOperation::CurrentBotIdentity => {
                    "Use a current dedicated bot identity and retry setup.".to_owned()
                }
            },
        )
    }

    pub(crate) fn retry_setup(operation: RequestOperation) -> Self {
        Self::new(
            RemediationAction::RetrySetup,
            format!(
                "Retry the read-only setup check for {operation} after the remote condition clears."
            ),
        )
    }

    pub(crate) fn retry_after_rate_limit(retry_after: std::time::Duration) -> Self {
        Self::new(
            RemediationAction::RetryAfterRateLimit,
            format!(
                "Wait at least {} seconds, as reported by Discord, then retry setup.",
                retry_after.as_secs()
            ),
        )
    }

    pub(crate) fn review_api_compatibility(operation: RequestOperation) -> Self {
        Self::new(
            RemediationAction::ReviewApiCompatibility,
            format!(
                "Review the current Discord REST v10 response contract for {operation}; no response body was retained."
            ),
        )
    }

    pub(crate) fn grant_permission(permission: RequiredPermission) -> Self {
        Self::new(
            RemediationAction::GrantMissingPermission,
            format!(
                "Ask a workspace administrator to grant {} on the configured channel; repo-com will not change permissions.",
                permission.name()
            ),
        )
    }

    pub(crate) fn invite_bot_to_workspace() -> Self {
        Self::new(
            RemediationAction::InviteBotToWorkspace,
            "Invite the dedicated bot to the configured workspace, then retry setup.",
        )
    }

    pub(crate) fn use_dedicated_bot_identity() -> Self {
        Self::new(
            RemediationAction::UseDedicatedBotIdentity,
            "Replace a user or self-bot credential with a dedicated Discord bot identity.",
        )
    }

    pub(crate) fn correct_mention_target() -> Self {
        Self::new(
            RemediationAction::CorrectMentionTarget,
            "Correct the mention alias to a workspace user, or remove it from the allowlist.",
        )
    }

    pub(crate) fn use_mentionable_role() -> Self {
        Self::new(
            RemediationAction::UseMentionableRole,
            "Choose a role whose Discord mentionable flag is true, or remove the role from the allowlist.",
        )
    }

    fn new(action: RemediationAction, instruction: impl Into<String>) -> Self {
        Self {
            action,
            instruction: instruction.into(),
        }
    }
}

/// The complete read-only setup report.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SetupReport {
    /// Identity proven with the dedicated bot credential.
    pub bot: BotIdentity,
    /// Configured workspace ID.
    pub workspace_id: String,
    /// Membership check result.
    pub workspace_membership: WorkspaceMembership,
    /// Unique channel checks with repository-local aliases only.
    pub channels: Vec<ChannelCheck>,
    /// Resolved mention checks.
    pub mentions: Vec<MentionCheck>,
    /// Structured issues and remediation.
    pub issues: Vec<SetupIssue>,
}

impl SetupReport {
    /// Returns true only when identity, membership, channels, permissions, and
    /// mentions all passed.
    #[must_use]
    pub fn ready(&self) -> bool {
        self.issues.is_empty()
    }
}

/// One setup issue with safe structured remediation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SetupIssue {
    /// Stable issue kind.
    pub kind: SetupIssueKind,
    /// Manual remediation; setup never performs it.
    pub remediation: Remediation,
}

impl DiscordClient {
    /// Performs the guided setup check using only GET requests under REST v10.
    pub async fn check_setup(&self, config: &ResolvedConfig) -> Result<SetupReport, ClientError> {
        let workspace_id = config.config.discord.workspace_id.clone();
        let current_user: CurrentUserWire = require_found(
            self.get(&["users", "@me"], RequestOperation::CurrentBotIdentity)
                .await?,
            RequestOperation::CurrentBotIdentity,
        )?;
        if !is_discord_id(&current_user.id) {
            return Err(ClientError::InvalidResponse {
                operation: RequestOperation::CurrentBotIdentity,
            });
        }

        let bot = BotIdentity {
            user_id: current_user.id.clone(),
            dedicated_bot: current_user.bot,
        };
        let mut report = SetupReport {
            bot: bot.clone(),
            workspace_id: workspace_id.clone(),
            workspace_membership: WorkspaceMembership::Unchecked,
            channels: Vec::new(),
            mentions: Vec::new(),
            issues: Vec::new(),
        };
        if !bot.dedicated_bot {
            report.issues.push(SetupIssue {
                kind: SetupIssueKind::IdentityNotDedicatedBot,
                remediation: Remediation::use_dedicated_bot_identity(),
            });
            return Ok(report);
        }

        let membership: GuildMemberWire = match self
            .get(
                &["users", "@me", "guilds", &workspace_id, "member"],
                RequestOperation::GuildMembership,
            )
            .await?
        {
            GetOutcome::Found(membership) => membership,
            GetOutcome::NotFound => {
                report.workspace_membership = WorkspaceMembership::Missing;
                report.issues.push(SetupIssue {
                    kind: SetupIssueKind::WorkspaceMembershipMissing,
                    remediation: Remediation::invite_bot_to_workspace(),
                });
                return Ok(report);
            }
            GetOutcome::Forbidden => {
                return Err(ClientError::PermissionDenied {
                    operation: RequestOperation::GuildMembership,
                });
            }
        };
        report.workspace_membership = WorkspaceMembership::Member;

        let guild: GuildWire = require_found(
            self.get(&["guilds", &workspace_id], RequestOperation::Guild)
                .await?,
            RequestOperation::Guild,
        )?;
        if guild.id != workspace_id {
            return Err(ClientError::InvalidResponse {
                operation: RequestOperation::Guild,
            });
        }
        let context = GuildPermissionContext::new(&workspace_id, &bot.user_id, membership, guild)?;

        let channel_uses = channel_uses(config);
        let mut channel_permissions = BTreeMap::new();
        for (channel_id, uses) in channel_uses {
            let channel_outcome: GetOutcome<ChannelWire> = self
                .get(&["channels", &channel_id], RequestOperation::Channel)
                .await?;
            match channel_outcome {
                GetOutcome::NotFound => {
                    report.channels.push(uses.check(None, false));
                    add_unavailable_channel_issues(&mut report.issues, &uses, false);
                }
                GetOutcome::Forbidden => {
                    report.channels.push(uses.check(None, false));
                    add_unavailable_channel_issues(&mut report.issues, &uses, true);
                }
                GetOutcome::Found(channel) => {
                    if channel.id != channel_id
                        || channel.guild_id.as_deref() != Some(&workspace_id)
                    {
                        report.channels.push(uses.check(None, false));
                        add_unavailable_channel_issues(&mut report.issues, &uses, false);
                        continue;
                    }
                    let permissions = context.permissions_for(&channel)?;
                    let visible = has_permission(permissions, RequiredPermission::ViewChannel);
                    let channel_check = uses.check(Some(permissions), visible);
                    let channel_ready = channel_check.ready;
                    report.channels.push(channel_check);
                    for (alias, role, permission) in uses.missing_entries(permissions) {
                        report.issues.push(SetupIssue {
                            kind: SetupIssueKind::MissingPermission {
                                alias,
                                role,
                                permission,
                            },
                            remediation: Remediation::grant_permission(permission),
                        });
                    }
                    if channel_ready {
                        channel_permissions.insert(channel_id, permissions);
                    }
                }
            }
        }

        let mut user_membership = BTreeMap::new();
        for destination in config.destinations.values() {
            let Some(permissions) = channel_permissions.get(&destination.channel_id).copied()
            else {
                continue;
            };
            for mention in &destination.allowed_mentions {
                let mut ready = true;
                match mention.target.kind() {
                    MentionKind::Role => {
                        let Some(role) = context.roles.get(mention.target.id()) else {
                            ready = false;
                            report.issues.push(SetupIssue {
                                kind: SetupIssueKind::MentionTargetUnavailable {
                                    destination_alias: destination.alias.clone(),
                                    mention_alias: mention.alias.clone(),
                                    target_kind: MentionKind::Role,
                                },
                                remediation: Remediation::correct_mention_target(),
                            });
                            report.mentions.push(MentionCheck {
                                destination_alias: destination.alias.clone(),
                                mention_alias: mention.alias.clone(),
                                target_kind: MentionKind::Role,
                                ready,
                            });
                            continue;
                        };
                        if !role.mentionable {
                            ready = false;
                            report.issues.push(SetupIssue {
                                kind: SetupIssueKind::MentionRoleNotMentionable {
                                    destination_alias: destination.alias.clone(),
                                    mention_alias: mention.alias.clone(),
                                },
                                remediation: Remediation::use_mentionable_role(),
                            });
                        }
                        if !has_permission(permissions, RequiredPermission::MentionRoles) {
                            ready = false;
                            report.issues.push(SetupIssue {
                                kind: SetupIssueKind::MentionRolePermissionMissing {
                                    destination_alias: destination.alias.clone(),
                                    mention_alias: mention.alias.clone(),
                                },
                                remediation: Remediation::grant_permission(
                                    RequiredPermission::MentionRoles,
                                ),
                            });
                        }
                    }
                    MentionKind::User => {
                        if mention.target.id() == bot.user_id {
                            ready = false;
                            report.issues.push(SetupIssue {
                                kind: SetupIssueKind::MentionTargetIsBot {
                                    destination_alias: destination.alias.clone(),
                                    mention_alias: mention.alias.clone(),
                                },
                                remediation: Remediation::correct_mention_target(),
                            });
                        } else {
                            let member_exists = if let Some(exists) =
                                user_membership.get(mention.target.id()).copied()
                            {
                                exists
                            } else {
                                let member_outcome: GetOutcome<GuildMemberWire> = self
                                    .get(
                                        &["guilds", &workspace_id, "members", mention.target.id()],
                                        RequestOperation::GuildMember,
                                    )
                                    .await?;
                                let exists = match member_outcome {
                                    GetOutcome::Found(_user) => true,
                                    GetOutcome::NotFound | GetOutcome::Forbidden => false,
                                };
                                user_membership.insert(mention.target.id().to_owned(), exists);
                                exists
                            };
                            if !member_exists {
                                ready = false;
                                report.issues.push(SetupIssue {
                                    kind: SetupIssueKind::MentionTargetUnavailable {
                                        destination_alias: destination.alias.clone(),
                                        mention_alias: mention.alias.clone(),
                                        target_kind: MentionKind::User,
                                    },
                                    remediation: Remediation::correct_mention_target(),
                                });
                            }
                        }
                    }
                }
                report.mentions.push(MentionCheck {
                    destination_alias: destination.alias.clone(),
                    mention_alias: mention.alias.clone(),
                    target_kind: mention.target.kind(),
                    ready,
                });
            }
        }

        Ok(report)
    }
}

fn require_found<T>(outcome: GetOutcome<T>, operation: RequestOperation) -> Result<T, ClientError> {
    match outcome {
        GetOutcome::Found(value) => Ok(value),
        GetOutcome::Forbidden => Err(ClientError::PermissionDenied { operation }),
        GetOutcome::NotFound => Err(ClientError::NotFound { operation }),
    }
}

fn add_unavailable_channel_issues(
    issues: &mut Vec<SetupIssue>,
    uses: &ChannelUses,
    permission_denied: bool,
) {
    let remediation = if permission_denied {
        Remediation::grant_permission(RequiredPermission::ViewChannel)
    } else {
        Remediation::correct_configured_resource(RequestOperation::Channel)
    };
    for (alias, role) in uses.aliases() {
        let kind = if permission_denied {
            SetupIssueKind::MissingPermission {
                alias,
                role,
                permission: RequiredPermission::ViewChannel,
            }
        } else {
            SetupIssueKind::ChannelUnavailable { alias, role }
        };
        issues.push(SetupIssue {
            kind,
            remediation: remediation.clone(),
        });
    }
}

#[derive(Default)]
struct ChannelUses {
    destination_aliases: Vec<String>,
    inbound_aliases: Vec<String>,
}

impl ChannelUses {
    fn required_permissions(&self) -> Vec<RequiredPermission> {
        let mut permissions = BTreeSet::new();
        if !self.destination_aliases.is_empty() {
            permissions.insert(RequiredPermission::ViewChannel);
            permissions.insert(RequiredPermission::SendMessages);
        }
        if !self.inbound_aliases.is_empty() {
            permissions.insert(RequiredPermission::ViewChannel);
            permissions.insert(RequiredPermission::ReadMessageHistory);
        }
        permissions.into_iter().collect()
    }

    fn missing_entries(&self, permissions: u64) -> Vec<(String, ChannelRole, RequiredPermission)> {
        let mut missing = Vec::new();
        for alias in &self.destination_aliases {
            for permission in [
                RequiredPermission::ViewChannel,
                RequiredPermission::SendMessages,
            ] {
                if !has_permission(permissions, permission) {
                    missing.push((alias.clone(), ChannelRole::Destination, permission));
                }
            }
        }
        for alias in &self.inbound_aliases {
            for permission in [
                RequiredPermission::ViewChannel,
                RequiredPermission::ReadMessageHistory,
            ] {
                if !has_permission(permissions, permission) {
                    missing.push((alias.clone(), ChannelRole::EnabledInbound, permission));
                }
            }
        }
        missing
    }

    fn aliases(&self) -> Vec<(String, ChannelRole)> {
        let mut aliases = self
            .destination_aliases
            .iter()
            .cloned()
            .map(|alias| (alias, ChannelRole::Destination))
            .collect::<Vec<_>>();
        aliases.extend(
            self.inbound_aliases
                .iter()
                .cloned()
                .map(|alias| (alias, ChannelRole::EnabledInbound)),
        );
        aliases
    }

    fn check(&self, permissions: Option<u64>, visible_in_workspace: bool) -> ChannelCheck {
        let required_permissions = self.required_permissions();
        let missing_permissions = match permissions {
            Some(bits) => required_permissions
                .iter()
                .copied()
                .filter(|permission| !has_permission(bits, *permission))
                .collect(),
            None => required_permissions.clone(),
        };
        let granted_permissions = required_permissions
            .iter()
            .copied()
            .filter(|permission| !missing_permissions.contains(permission))
            .collect();
        ChannelCheck {
            destination_aliases: self.destination_aliases.clone(),
            inbound_aliases: self.inbound_aliases.clone(),
            ready: visible_in_workspace && missing_permissions.is_empty(),
            required_permissions,
            granted_permissions,
            missing_permissions,
            visible_in_workspace,
        }
    }
}

fn channel_uses(config: &ResolvedConfig) -> BTreeMap<String, ChannelUses> {
    let mut uses = BTreeMap::<String, ChannelUses>::new();
    for destination in config.destinations.values() {
        uses.entry(destination.channel_id.clone())
            .or_default()
            .destination_aliases
            .push(destination.alias.clone());
    }
    for inbound in config.inbound.values() {
        if inbound.enabled {
            uses.entry(inbound.channel_id.clone())
                .or_default()
                .inbound_aliases
                .push(inbound.alias.clone());
        }
    }
    uses
}

struct RoleState {
    permissions: u64,
    mentionable: bool,
}

struct GuildPermissionContext {
    guild_id: String,
    bot_user_id: String,
    member_roles: Vec<String>,
    roles: BTreeMap<String, RoleState>,
    base_permissions: u64,
}

impl GuildPermissionContext {
    fn new(
        guild_id: &str,
        bot_user_id: &str,
        member: GuildMemberWire,
        guild: GuildWire,
    ) -> Result<Self, ClientError> {
        let mut roles = BTreeMap::new();
        for role in guild.roles {
            let state = RoleState {
                permissions: parse_permissions(&role.permissions, RequestOperation::Guild)?,
                mentionable: role.mentionable,
            };
            if roles.insert(role.id, state).is_some() {
                return Err(ClientError::InvalidResponse {
                    operation: RequestOperation::Guild,
                });
            }
        }
        let everyone = roles.get(guild_id).ok_or(ClientError::InvalidResponse {
            operation: RequestOperation::Guild,
        })?;
        let mut base_permissions = everyone.permissions;
        for role_id in &member.roles {
            let role = roles.get(role_id).ok_or(ClientError::InvalidResponse {
                operation: RequestOperation::GuildMembership,
            })?;
            base_permissions |= role.permissions;
        }
        Ok(Self {
            guild_id: guild_id.to_owned(),
            bot_user_id: bot_user_id.to_owned(),
            member_roles: member.roles,
            roles,
            base_permissions,
        })
    }

    fn permissions_for(&self, channel: &ChannelWire) -> Result<u64, ClientError> {
        if has_raw_permission(self.base_permissions, ADMINISTRATOR_PERMISSION) {
            return Ok(self.base_permissions | SETUP_PERMISSION_MASK);
        }
        let mut permissions = self.base_permissions;
        apply_overwrites(
            &mut permissions,
            &self.guild_id,
            &self.bot_user_id,
            &self.member_roles,
            &channel.permission_overwrites,
        )?;
        Ok(permissions)
    }
}

fn apply_overwrites(
    permissions: &mut u64,
    guild_id: &str,
    bot_user_id: &str,
    member_roles: &[String],
    raw_overwrites: &[PermissionOverwriteWire],
) -> Result<(), ClientError> {
    let mut overwrites = BTreeMap::new();
    for overwrite in raw_overwrites {
        let parsed = ParsedOverwrite {
            kind: match overwrite.kind {
                0 => OverwriteKind::Role,
                1 => OverwriteKind::Member,
                _ => {
                    return Err(ClientError::InvalidResponse {
                        operation: RequestOperation::Channel,
                    });
                }
            },
            allow: parse_permissions(&overwrite.allow, RequestOperation::Channel)?,
            deny: parse_permissions(&overwrite.deny, RequestOperation::Channel)?,
        };
        if overwrites.insert(overwrite.id.as_str(), parsed).is_some() {
            return Err(ClientError::InvalidResponse {
                operation: RequestOperation::Channel,
            });
        }
    }

    if let Some(overwrite) = overwrites.get(guild_id) {
        if !matches!(overwrite.kind, OverwriteKind::Role) {
            return Err(ClientError::InvalidResponse {
                operation: RequestOperation::Channel,
            });
        }
        apply_one(permissions, overwrite);
    }

    let mut role_allow = 0_u64;
    let mut role_deny = 0_u64;
    for role_id in member_roles {
        if let Some(overwrite) = overwrites.get(role_id.as_str()) {
            if !matches!(overwrite.kind, OverwriteKind::Role) {
                return Err(ClientError::InvalidResponse {
                    operation: RequestOperation::Channel,
                });
            }
            role_allow |= overwrite.allow;
            role_deny |= overwrite.deny;
        }
    }
    *permissions &= !role_deny;
    *permissions |= role_allow;

    if let Some(overwrite) = overwrites.get(bot_user_id) {
        if !matches!(overwrite.kind, OverwriteKind::Member) {
            return Err(ClientError::InvalidResponse {
                operation: RequestOperation::Channel,
            });
        }
        apply_one(permissions, overwrite);
    }
    Ok(())
}

fn apply_one(permissions: &mut u64, overwrite: &ParsedOverwrite) {
    *permissions &= !overwrite.deny;
    *permissions |= overwrite.allow;
}

struct ParsedOverwrite {
    kind: OverwriteKind,
    allow: u64,
    deny: u64,
}

enum OverwriteKind {
    Role,
    Member,
}

fn parse_permissions(value: &str, operation: RequestOperation) -> Result<u64, ClientError> {
    value
        .parse::<u64>()
        .map_err(|_| ClientError::InvalidResponse { operation })
}

fn has_permission(bits: u64, permission: RequiredPermission) -> bool {
    has_raw_permission(bits, permission.bit())
}

fn has_raw_permission(bits: u64, permission: u64) -> bool {
    bits & permission == permission
}

fn is_discord_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 20 && value.bytes().all(|byte| byte.is_ascii_digit())
}

#[derive(Deserialize)]
struct CurrentUserWire {
    id: String,
    #[serde(default)]
    bot: bool,
}

#[derive(Deserialize)]
struct GuildMemberWire {
    #[serde(default)]
    roles: Vec<String>,
}

#[derive(Deserialize)]
struct GuildWire {
    id: String,
    roles: Vec<RoleWire>,
}

#[derive(Deserialize)]
struct RoleWire {
    id: String,
    permissions: String,
    mentionable: bool,
}

#[derive(Deserialize)]
struct ChannelWire {
    id: String,
    guild_id: Option<String>,
    #[serde(default)]
    permission_overwrites: Vec<PermissionOverwriteWire>,
}

#[derive(Deserialize)]
struct PermissionOverwriteWire {
    id: String,
    #[serde(rename = "type")]
    kind: u8,
    allow: String,
    deny: String,
}
