// ABOUTME: Handlers for /group slash commands in messaging
// ABOUTME: List groups, show stats, list members, generate invites, leave group
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Write;

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_messaging::commands::CommandResponse;

use super::{CommandHandler, PlatformCommandContext};

/// Handler for `/group` — list user's groups
pub struct GroupListHandler;

#[async_trait]
impl CommandHandler for GroupListHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let groups = ctx
            .resources
            .repos
            .groups
            .list_groups_for_user(ctx.user_id)
            .await?;

        if groups.is_empty() {
            return Ok(CommandResponse::text(
                "You are not a member of any groups.\nCreate or join a group via the web or mobile app.",
            ));
        }

        let mut text = String::with_capacity(256);
        let _ = writeln!(text, "Your Groups ({}):\n", groups.len());
        for g in &groups {
            let role = g.my_role.as_str();
            let _ = writeln!(text, "- {} ({} members) [{}]", g.name, g.member_count, role);
        }

        Ok(CommandResponse::text(text))
    }
}

/// Handler for `/group status` — aggregate stats
pub struct GroupStatusHandler;

#[async_trait]
impl CommandHandler for GroupStatusHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let groups = ctx
            .resources
            .repos
            .groups
            .list_groups_for_user(ctx.user_id)
            .await?;

        let group = groups
            .first()
            .ok_or_else(|| AppError::not_found("You are not a member of any group"))?;

        let member_count = ctx
            .resources
            .repos
            .groups
            .count_members(&group.id.to_string(), ctx.tenant_id)
            .await
            .unwrap_or(0);

        let text = format!(
            "{} Stats:\n\
             - Members: {}\n\
             - Active: {}\n\
             - Peer sharing: {}",
            group.name,
            member_count,
            group.member_count,
            if group.peer_data_sharing { "on" } else { "off" }
        );

        Ok(CommandResponse::text(text))
    }
}

/// Handler for `/group members` — list members
pub struct GroupMembersHandler;

#[async_trait]
impl CommandHandler for GroupMembersHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let groups = ctx
            .resources
            .repos
            .groups
            .list_groups_for_user(ctx.user_id)
            .await?;

        let group = groups
            .first()
            .ok_or_else(|| AppError::not_found("You are not a member of any group"))?;

        let members = ctx
            .resources
            .repos
            .groups
            .list_members(&group.id.to_string(), ctx.tenant_id)
            .await?;

        let mut text = String::with_capacity(256);
        let _ = writeln!(text, "{} Members ({}):\n", group.name, members.len());
        for m in &members {
            let name = m.display_name.as_deref().unwrap_or("Unknown");
            let role = m.role.as_str();
            let _ = writeln!(text, "- {name} [{role}]");
        }

        Ok(CommandResponse::text(text))
    }
}

/// Handler for `/group invite` — generate invite link
pub struct GroupInviteHandler;

#[async_trait]
impl CommandHandler for GroupInviteHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let groups = ctx
            .resources
            .repos
            .groups
            .list_groups_for_user(ctx.user_id)
            .await?;

        let group = groups
            .first()
            .ok_or_else(|| AppError::not_found("You are not a member of any group"))?;

        // Check admin role
        let member = ctx
            .resources
            .repos
            .groups
            .get_member(&group.id.to_string(), ctx.user_id, ctx.tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found("Membership not found"))?;

        if !member.role.can_manage_members() {
            return Ok(CommandResponse::text(
                "Only admins and owners can generate invite links.",
            ));
        }

        #[cfg(feature = "tools-groups")]
        {
            let invite = ctx
                .resources
                .group_service()
                .create_invite(group.id, ctx.user_id, ctx.tenant_id, Some(7), None)
                .await?;

            let text = format!(
                "Invite link for {}:\nhttps://app.dravr.ai/groups/join/{}\n\nCode: {}\nValid for 7 days.",
                group.name, invite.code, invite.code
            );

            return Ok(CommandResponse::text(text));
        }

        #[cfg(not(feature = "tools-groups"))]
        Ok(CommandResponse::text("Group invites not available."))
    }
}

/// Handler for `/group leave` — leave the group
pub struct GroupLeaveHandler;

#[async_trait]
impl CommandHandler for GroupLeaveHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let groups = ctx
            .resources
            .repos
            .groups
            .list_groups_for_user(ctx.user_id)
            .await?;

        let group = groups
            .first()
            .ok_or_else(|| AppError::not_found("You are not a member of any group"))?;

        let text = format!(
            "Are you sure you want to leave {}?\n\
             Type \"YES\" to confirm.",
            group.name
        );

        Ok(CommandResponse::with_confirmation(text))
    }
}
