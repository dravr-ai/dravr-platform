// ABOUTME: Handlers for /group slash commands in messaging
// ABOUTME: List groups, show stats, list members, generate invites, leave group
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_messaging::commands::CommandResponse;
use tracing::info;

#[cfg(feature = "tools-groups")]
use pierre_contremaitre::messaging_strings::KEY_GROUP_INVITE_BODY;
#[cfg(not(feature = "tools-groups"))]
use pierre_contremaitre::messaging_strings::KEY_GROUP_INVITE_UNAVAILABLE;
use pierre_contremaitre::messaging_strings::{
    KEY_GROUP_CONSENT_UPDATED, KEY_GROUP_CONSENT_USAGE, KEY_GROUP_INVITE_FORBIDDEN,
    KEY_GROUP_LEAVE_PROMPT, KEY_GROUP_LIST_EMPTY, KEY_GROUP_LIST_HEADER, KEY_GROUP_LIST_ITEM,
    KEY_GROUP_MEMBERS_HEADER, KEY_GROUP_MEMBERS_ITEM, KEY_GROUP_MEMBERS_UNKNOWN,
    KEY_GROUP_NOT_A_MEMBER, KEY_GROUP_PEER_SHARING_OFF, KEY_GROUP_PEER_SHARING_ON,
    KEY_GROUP_ROLE_ADMIN, KEY_GROUP_ROLE_MEMBER, KEY_GROUP_ROLE_OWNER, KEY_GROUP_STATUS_SUMMARY,
};
use pierre_core::models::coaches::ListCoachesFilter;
#[cfg(feature = "tools-groups")]
use pierre_core::models::groups::GroupInviteKind;
use pierre_core::models::groups::{GroupRole, UpdateGroupRequest};

use crate::{CommandHandler, PlatformCommandContext};

/// Map a [`GroupRole`] to its localized messaging-string key so the role
/// label renders in the user's locale instead of leaking the raw English
/// enum value into an otherwise-translated reply.
const fn role_label_key(role: GroupRole) -> &'static str {
    match role {
        GroupRole::Owner => KEY_GROUP_ROLE_OWNER,
        GroupRole::Admin => KEY_GROUP_ROLE_ADMIN,
        GroupRole::Member => KEY_GROUP_ROLE_MEMBER,
    }
}

/// Handler for `/group` — list user's groups
pub struct GroupListHandler;

#[async_trait]
impl CommandHandler for GroupListHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();

        let groups = ctx
            .ctx
            .repos()
            .groups
            .list_groups_for_user(ctx.user_id)
            .await?;

        if groups.is_empty() {
            return Ok(CommandResponse::text(reg.render(
                KEY_GROUP_LIST_EMPTY,
                locale,
                &[],
            )));
        }

        let mut text = String::with_capacity(256);
        let count = groups.len().to_string();
        text.push_str(&reg.render(KEY_GROUP_LIST_HEADER, locale, &[&count]));
        for g in &groups {
            let member_count = g.member_count.to_string();
            let role = reg.render(role_label_key(g.my_role), locale, &[]);
            text.push_str(&reg.render(
                KEY_GROUP_LIST_ITEM,
                locale,
                &[&g.name, &member_count, &role],
            ));
            text.push('\n');
        }

        Ok(CommandResponse::text(text))
    }
}

/// Handler for `/group status` — aggregate stats
pub struct GroupStatusHandler;

#[async_trait]
impl CommandHandler for GroupStatusHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();

        let groups = ctx
            .ctx
            .repos()
            .groups
            .list_groups_for_user(ctx.user_id)
            .await?;

        let group = groups
            .first()
            .ok_or_else(|| AppError::not_found(reg.render(KEY_GROUP_NOT_A_MEMBER, locale, &[])))?;

        let member_count = ctx
            .ctx
            .repos()
            .groups
            .count_members(&group.id.to_string())
            .await
            .unwrap_or(0);

        let peer_sharing_key = if group.peer_data_sharing {
            KEY_GROUP_PEER_SHARING_ON
        } else {
            KEY_GROUP_PEER_SHARING_OFF
        };
        let peer_sharing = reg.render(peer_sharing_key, locale, &[]);
        let mc = member_count.to_string();
        let active = group.member_count.to_string();

        let text = reg.render(
            KEY_GROUP_STATUS_SUMMARY,
            locale,
            &[&group.name, &mc, &active, &peer_sharing],
        );

        Ok(CommandResponse::text(text))
    }
}

/// Handler for `/group members` — list members
pub struct GroupMembersHandler;

#[async_trait]
impl CommandHandler for GroupMembersHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();

        let groups = ctx
            .ctx
            .repos()
            .groups
            .list_groups_for_user(ctx.user_id)
            .await?;

        let group = groups
            .first()
            .ok_or_else(|| AppError::not_found(reg.render(KEY_GROUP_NOT_A_MEMBER, locale, &[])))?;

        let members = ctx
            .ctx
            .repos()
            .groups
            .list_members(&group.id.to_string())
            .await?;

        let mut text = String::with_capacity(256);
        let count = members.len().to_string();
        text.push_str(&reg.render(KEY_GROUP_MEMBERS_HEADER, locale, &[&group.name, &count]));
        let unknown = reg.render(KEY_GROUP_MEMBERS_UNKNOWN, locale, &[]);
        for m in &members {
            let name = m.display_name.as_deref().unwrap_or(unknown.as_str());
            let role = reg.render(role_label_key(m.role), locale, &[]);
            text.push_str(&reg.render(KEY_GROUP_MEMBERS_ITEM, locale, &[name, &role]));
            text.push('\n');
        }

        Ok(CommandResponse::text(text))
    }
}

/// Handler for `/group invite` — generate invite link
pub struct GroupInviteHandler;

#[async_trait]
impl CommandHandler for GroupInviteHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();

        let groups = ctx
            .ctx
            .repos()
            .groups
            .list_groups_for_user(ctx.user_id)
            .await?;

        let group = groups
            .first()
            .ok_or_else(|| AppError::not_found(reg.render(KEY_GROUP_NOT_A_MEMBER, locale, &[])))?;

        // Check admin role
        let member = ctx
            .ctx
            .repos()
            .groups
            .get_member(&group.id.to_string(), ctx.user_id)
            .await?
            .ok_or_else(|| AppError::not_found("Membership not found"))?;

        if !member.role.can_manage_members() {
            return Ok(CommandResponse::text(reg.render(
                KEY_GROUP_INVITE_FORBIDDEN,
                locale,
                &[],
            )));
        }

        #[cfg(feature = "tools-groups")]
        {
            // `/group invite coach` issues a coach invite (attaches the
            // redeemer as the group's human coach); any other arg defaults to
            // a standard athlete-membership invite.
            let kind = if ctx
                .args
                .first()
                .is_some_and(|a| a.trim().eq_ignore_ascii_case("coach"))
            {
                GroupInviteKind::Coach
            } else {
                GroupInviteKind::Member
            };

            let invite = ctx
                .ctx
                .group_service()
                .create_invite(group.id, ctx.user_id, ctx.tenant_id, Some(7), None, kind)
                .await?;

            let text = reg.render(
                KEY_GROUP_INVITE_BODY,
                locale,
                &[&group.name, &invite.code, &invite.code],
            );

            return Ok(CommandResponse::text(text));
        }

        #[cfg(not(feature = "tools-groups"))]
        {
            Ok(CommandResponse::text(reg.render(
                KEY_GROUP_INVITE_UNAVAILABLE,
                locale,
                &[],
            )))
        }
    }
}

/// Handler for `/group coach <name>` — set the group's AI coach persona.
///
/// Owner/admin only. Resolves a Dravr coach by (case-insensitive) title from
/// the coaches visible to the caller and points the group's `coach_id` at it,
/// so that persona answers in the group thereafter. Distinct from
/// `/group invite coach`, which attaches a *human* coach (`coach_user_id`).
/// Replies in plain text so it does not depend on the `tools-groups`-gated
/// messaging strings.
pub struct GroupCoachHandler;

#[async_trait]
impl CommandHandler for GroupCoachHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();

        // Coach name argument — joined so multi-word names like "5K Marathon"
        // arrive intact.
        let name = ctx.args.join(" ");
        let name = name.trim();
        if name.is_empty() {
            return Ok(CommandResponse::text(
                "Usage: /group coach <name> — set this group's Dravr coach (e.g. /group coach 5K Marathon)."
                    .to_owned(),
            ));
        }

        // Resolve the caller's group (mirrors /group invite).
        let groups = ctx
            .ctx
            .repos()
            .groups
            .list_groups_for_user(ctx.user_id)
            .await?;
        let group = groups
            .first()
            .ok_or_else(|| AppError::not_found(reg.render(KEY_GROUP_NOT_A_MEMBER, locale, &[])))?;

        // Owner/admin only — changing the group's coach is a settings change.
        let member = ctx
            .ctx
            .repos()
            .groups
            .get_member(&group.id.to_string(), ctx.user_id)
            .await?
            .ok_or_else(|| AppError::not_found("Membership not found"))?;
        if !member.role.can_manage_members() {
            return Ok(CommandResponse::text(reg.render(
                KEY_GROUP_INVITE_FORBIDDEN,
                locale,
                &[],
            )));
        }

        // Find a visible coach whose title matches — exact (case-insensitive)
        // first, otherwise the first whose title contains the text.
        let filter = ListCoachesFilter::with_defaults();
        let coaches = ctx
            .ctx
            .repos()
            .coaches
            .list(ctx.user_id, ctx.tenant_id, &filter)
            .await?;
        let needle = name.to_lowercase();
        let matched = coaches
            .iter()
            .find(|c| c.coach.title.eq_ignore_ascii_case(name))
            .or_else(|| {
                coaches
                    .iter()
                    .find(|c| c.coach.title.to_lowercase().contains(&needle))
            });
        let Some(found) = matched else {
            return Ok(CommandResponse::text(format!(
                "No coach matching \"{name}\" found. Use /coach to see the coaches available to you."
            )));
        };

        // Point the group at the chosen coach persona.
        let request = UpdateGroupRequest {
            name: None,
            description: None,
            coach_id: Some(found.coach.id.to_string()),
            max_members: None,
            peer_data_sharing: None,
            is_active: None,
        };
        let updated = ctx
            .ctx
            .group_service()
            .update_group(&group.id.to_string(), ctx.tenant_id, &request)
            .await?
            .ok_or_else(|| AppError::not_found("Group not found"))?;

        info!(
            group_id = %updated.id,
            coach_id = %found.coach.id,
            "Group AI coach updated via /group coach"
        );

        Ok(CommandResponse::text(format!(
            "{}'s coach is now {}.",
            updated.name, found.coach.title
        )))
    }
}

/// Handler for `/group leave` — leave the group
pub struct GroupLeaveHandler;

#[async_trait]
impl CommandHandler for GroupLeaveHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();

        let groups = ctx
            .ctx
            .repos()
            .groups
            .list_groups_for_user(ctx.user_id)
            .await?;

        let group = groups
            .first()
            .ok_or_else(|| AppError::not_found(reg.render(KEY_GROUP_NOT_A_MEMBER, locale, &[])))?;

        let text = reg.render(KEY_GROUP_LEAVE_PROMPT, locale, &[&group.name]);

        Ok(CommandResponse::with_confirmation(text))
    }
}

/// Handler for `/group consent yes|no` — toggle peer-sharing consent.
///
/// Updates `coaching_group_members.peer_sharing_consent` for the requester
/// in the group bound to the current chat conversation. Resolution order:
///
///   1. `chat_conversations.group_id` for `ctx.conversation_id` — used by
///      Telegram/Slack/Discord group chats (auto-bound) and any web/mobile
///      conversation explicitly created against a group.
///   2. The user's most recently updated group from `list_groups_for_user`
///      — fallback for chat surfaces that haven't propagated a
///      conversation id (notably Slack `block_actions` buttons).
///
/// The privacy gate in `pierre_groups::GroupService::inject_group_context`
/// honors this flag: even when the group has `peer_data_sharing = true`,
/// only members who have set their consent to `true` will have their
/// training summaries rendered to peers.
pub struct GroupConsentHandler;

#[async_trait]
impl CommandHandler for GroupConsentHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();

        let arg = ctx.args.first().map_or("", String::as_str).trim();
        let consent_choice = match arg.to_lowercase().as_str() {
            "yes" | "on" | "true" | "1" => true,
            "no" | "off" | "false" | "0" => false,
            _ => {
                return Ok(CommandResponse::text(reg.render(
                    KEY_GROUP_CONSENT_USAGE,
                    locale,
                    &[],
                )));
            }
        };

        let user_id_str = ctx.user_id.to_string();
        let conversation_group = if let Some(conv_id) = ctx.conversation_id.as_deref() {
            ctx.ctx
                .repos()
                .chat
                .get_conversation(conv_id, &user_id_str, ctx.tenant_id)
                .await?
                .and_then(|c| c.group_id)
        } else {
            None
        };

        let (group_id_str, group_name) = if let Some(gid) = conversation_group {
            let group = ctx
                .ctx
                .repos()
                .groups
                .get_group(&gid, ctx.tenant_id)
                .await?
                .ok_or_else(|| {
                    AppError::not_found(reg.render(KEY_GROUP_NOT_A_MEMBER, locale, &[]))
                })?;
            (group.id.to_string(), group.name)
        } else {
            let groups = ctx
                .ctx
                .repos()
                .groups
                .list_groups_for_user(ctx.user_id)
                .await?;
            let group = groups.first().ok_or_else(|| {
                AppError::not_found(reg.render(KEY_GROUP_NOT_A_MEMBER, locale, &[]))
            })?;
            (group.id.to_string(), group.name.clone())
        };

        let rows_affected = ctx
            .ctx
            .repos()
            .groups
            .update_peer_sharing_consent(&group_id_str, ctx.user_id, consent_choice)
            .await?;

        info!(
            user_id = %ctx.user_id,
            group_id = %group_id_str,
            group_name = %group_name,
            consent_choice,
            rows_affected,
            source = if ctx.conversation_id.is_some() { "conversation_group_id" } else { "list_groups_for_user_first" },
            "Applied /group consent — peer_sharing_consent updated"
        );

        if !rows_affected {
            return Err(AppError::not_found(reg.render(
                KEY_GROUP_NOT_A_MEMBER,
                locale,
                &[],
            )));
        }

        let state_key = if consent_choice {
            KEY_GROUP_PEER_SHARING_ON
        } else {
            KEY_GROUP_PEER_SHARING_OFF
        };
        let state = reg.render(state_key, locale, &[]);
        let body = reg.render(KEY_GROUP_CONSENT_UPDATED, locale, &[&state, &group_name]);

        Ok(CommandResponse::text(body))
    }
}
