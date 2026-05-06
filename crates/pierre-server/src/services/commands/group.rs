// ABOUTME: Handlers for /group slash commands in messaging
// ABOUTME: List groups, show stats, list members, generate invites, leave group
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_messaging::commands::CommandResponse;

#[cfg(feature = "tools-groups")]
use crate::contremaitre::messaging_strings::KEY_GROUP_INVITE_BODY;
#[cfg(not(feature = "tools-groups"))]
use crate::contremaitre::messaging_strings::KEY_GROUP_INVITE_UNAVAILABLE;
use crate::contremaitre::messaging_strings::{
    KEY_GROUP_CONSENT_UPDATED, KEY_GROUP_CONSENT_USAGE, KEY_GROUP_INVITE_FORBIDDEN,
    KEY_GROUP_LEAVE_PROMPT, KEY_GROUP_LIST_EMPTY, KEY_GROUP_LIST_HEADER, KEY_GROUP_LIST_ITEM,
    KEY_GROUP_MEMBERS_HEADER, KEY_GROUP_MEMBERS_ITEM, KEY_GROUP_MEMBERS_UNKNOWN,
    KEY_GROUP_NOT_A_MEMBER, KEY_GROUP_PEER_SHARING_OFF, KEY_GROUP_PEER_SHARING_ON,
    KEY_GROUP_STATUS_SUMMARY,
};

use super::{CommandHandler, PlatformCommandContext};

/// Handler for `/group` — list user's groups
pub struct GroupListHandler;

#[async_trait]
impl CommandHandler for GroupListHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = &ctx.resources.messaging_strings_registry;
        let locale = ctx.locale.as_str();

        let groups = ctx
            .resources
            .repos
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
            let role = g.my_role.as_str();
            text.push_str(&reg.render(
                KEY_GROUP_LIST_ITEM,
                locale,
                &[&g.name, &member_count, role],
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
        let reg = &ctx.resources.messaging_strings_registry;
        let locale = ctx.locale.as_str();

        let groups = ctx
            .resources
            .repos
            .groups
            .list_groups_for_user(ctx.user_id)
            .await?;

        let group = groups
            .first()
            .ok_or_else(|| AppError::not_found(reg.render(KEY_GROUP_NOT_A_MEMBER, locale, &[])))?;

        let member_count = ctx
            .resources
            .repos
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
        let reg = &ctx.resources.messaging_strings_registry;
        let locale = ctx.locale.as_str();

        let groups = ctx
            .resources
            .repos
            .groups
            .list_groups_for_user(ctx.user_id)
            .await?;

        let group = groups
            .first()
            .ok_or_else(|| AppError::not_found(reg.render(KEY_GROUP_NOT_A_MEMBER, locale, &[])))?;

        let members = ctx
            .resources
            .repos
            .groups
            .list_members(&group.id.to_string())
            .await?;

        let mut text = String::with_capacity(256);
        let count = members.len().to_string();
        text.push_str(&reg.render(KEY_GROUP_MEMBERS_HEADER, locale, &[&group.name, &count]));
        let unknown = reg.render(KEY_GROUP_MEMBERS_UNKNOWN, locale, &[]);
        for m in &members {
            let name = m.display_name.as_deref().unwrap_or(unknown.as_str());
            let role = m.role.as_str();
            text.push_str(&reg.render(KEY_GROUP_MEMBERS_ITEM, locale, &[name, role]));
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
        let reg = &ctx.resources.messaging_strings_registry;
        let locale = ctx.locale.as_str();

        let groups = ctx
            .resources
            .repos
            .groups
            .list_groups_for_user(ctx.user_id)
            .await?;

        let group = groups
            .first()
            .ok_or_else(|| AppError::not_found(reg.render(KEY_GROUP_NOT_A_MEMBER, locale, &[])))?;

        // Check admin role
        let member = ctx
            .resources
            .repos
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
            let invite = ctx
                .resources
                .group_service()
                .create_invite(group.id, ctx.user_id, ctx.tenant_id, Some(7), None)
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

/// Handler for `/group leave` — leave the group
pub struct GroupLeaveHandler;

#[async_trait]
impl CommandHandler for GroupLeaveHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = &ctx.resources.messaging_strings_registry;
        let locale = ctx.locale.as_str();

        let groups = ctx
            .resources
            .repos
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
/// in their first listed group. The privacy gate in
/// `pierre_groups::GroupService::inject_group_context` honors this flag:
/// even when the group has `peer_data_sharing = true`, only members who
/// have set their consent to `true` will have their training summaries
/// rendered to peers.
pub struct GroupConsentHandler;

#[async_trait]
impl CommandHandler for GroupConsentHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = &ctx.resources.messaging_strings_registry;
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

        let groups = ctx
            .resources
            .repos
            .groups
            .list_groups_for_user(ctx.user_id)
            .await?;

        let group = groups
            .first()
            .ok_or_else(|| AppError::not_found(reg.render(KEY_GROUP_NOT_A_MEMBER, locale, &[])))?;

        ctx.resources
            .repos
            .groups
            .update_peer_sharing_consent(&group.id.to_string(), ctx.user_id, consent_choice)
            .await?;

        let state_key = if consent_choice {
            KEY_GROUP_PEER_SHARING_ON
        } else {
            KEY_GROUP_PEER_SHARING_OFF
        };
        let state = reg.render(state_key, locale, &[]);
        let body = reg.render(KEY_GROUP_CONSENT_UPDATED, locale, &[&state, &group.name]);

        Ok(CommandResponse::text(body))
    }
}
