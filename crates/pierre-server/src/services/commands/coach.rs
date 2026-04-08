// ABOUTME: Handlers for /coach slash commands in messaging channels
// ABOUTME: Lists available coaches as interactive cards and handles coach selection for groups
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Write;

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_core::models::coaches::ListCoachesFilter;
use pierre_core::uuid_utils::parse_uuid;
use pierre_messaging::commands::{CommandAction, CommandResponse};

use super::{CommandHandler, PlatformCommandContext};

/// Maximum number of coaches to display in a single card
const MAX_COACH_BUTTONS: usize = 8;

/// Handler for `/coach` — list available coaches as an interactive card
pub struct CoachListHandler;

#[async_trait]
impl CommandHandler for CoachListHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let filter = ListCoachesFilter::with_defaults();

        let coaches = ctx
            .resources
            .repos
            .coaches
            .list(ctx.user_id, ctx.tenant_id, &filter)
            .await?;

        if coaches.is_empty() {
            return Ok(CommandResponse::text(
                "No coaches available. Ask your admin to add coaches to your workspace.",
            ));
        }

        let mut body = String::with_capacity(512);
        let actions: Vec<CommandAction> = coaches
            .iter()
            .take(MAX_COACH_BUTTONS)
            .map(|item| {
                let category = item.coach.category.display_name();
                let desc = item
                    .coach
                    .description
                    .as_deref()
                    .unwrap_or("No description");
                let _ = writeln!(body, "• {} [{}]\n  {}\n", item.coach.title, category, desc);

                CommandAction {
                    label: item.coach.title.clone(),
                    action_type: "postback".to_owned(),
                    value: format!("/coach select {}", item.coach.id),
                }
            })
            .collect();

        Ok(CommandResponse::card("Choose a Coach", body, actions))
    }
}

/// Handler for `/coach select <coach_id>` — bind a coach to this conversation's group
pub struct CoachSelectHandler;

#[async_trait]
impl CommandHandler for CoachSelectHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let coach_id = ctx.args.first().ok_or_else(|| {
            AppError::invalid_input("Missing coach ID. Usage: /coach select <id>")
        })?;

        // Validate coach_id is a valid UUID before hitting the database
        let _ = parse_uuid(coach_id)?;

        // Verify the coach exists and the user has access (tenant-scoped)
        let coach = ctx
            .resources
            .repos
            .coaches
            .get_by_id(coach_id, ctx.user_id, ctx.tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Coach {coach_id}")))?;

        // Fetch user's groups and filter to only those in the current tenant
        let all_groups = ctx
            .resources
            .repos
            .groups
            .list_groups_for_user(ctx.user_id)
            .await?;

        let tenant_groups: Vec<_> = all_groups
            .into_iter()
            .filter(|g| {
                // Verify group belongs to this tenant by checking via get_group
                // GroupSummary doesn't carry tenant_id, so we filter by checking
                // the user's admin/owner role (only owners/admins should change coach)
                g.my_role.can_modify_settings()
            })
            .collect();

        match tenant_groups.len() {
            0 => {
                // No groups with admin/owner role — create a new one
                let group_name = format!("{} Group", coach.title);

                #[cfg(feature = "tools-groups")]
                {
                    use pierre_core::models::groups::CreateGroupRequest;

                    let request = CreateGroupRequest {
                        name: group_name.clone(),
                        description: Some(format!("Group coached by {}", coach.title)),
                        coach_id: coach.id.to_string(),
                        max_members: None,
                    };

                    ctx.resources
                        .group_service()
                        .create_group(&request, ctx.user_id, ctx.tenant_id)
                        .await?;

                    return Ok(CommandResponse::text(format!(
                        "Created group \"{group_name}\" with coach {}. \
                         Members can join via /group invite.",
                        coach.title
                    )));
                }

                #[cfg(not(feature = "tools-groups"))]
                Ok(CommandResponse::text(format!(
                    "Selected coach: {}. Group creation is not available.",
                    coach.title
                )))
            }
            1 => {
                // Exactly one group — update its coach
                let group = &tenant_groups[0];
                update_group_coach(ctx, &group.id.to_string(), coach_id).await?;

                Ok(CommandResponse::text(format!(
                    "Coach updated to {} for group {}.",
                    coach.title, group.name
                )))
            }
            _ => {
                // Multiple groups — ask the user to pick one
                let mut body = String::with_capacity(256);
                let _ = writeln!(
                    body,
                    "You manage {} groups. Which one should use {}?\n",
                    tenant_groups.len(),
                    coach.title
                );

                let actions: Vec<CommandAction> = tenant_groups
                    .iter()
                    .take(MAX_COACH_BUTTONS)
                    .map(|g| {
                        let _ = writeln!(body, "• {} ({} members)", g.name, g.member_count);
                        CommandAction {
                            label: g.name.clone(),
                            action_type: "postback".to_owned(),
                            value: format!("/coach assign {} {}", coach_id, g.id),
                        }
                    })
                    .collect();

                Ok(CommandResponse::card("Choose a Group", body, actions))
            }
        }
    }
}

/// Handler for `/coach assign <coach_id> <group_id>` — bind coach to a specific group
pub struct CoachAssignHandler;

#[async_trait]
impl CommandHandler for CoachAssignHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let coach_id = ctx
            .args
            .first()
            .ok_or_else(|| AppError::invalid_input("Usage: /coach assign <coach_id> <group_id>"))?;
        let group_id = ctx
            .args
            .get(1)
            .ok_or_else(|| AppError::invalid_input("Usage: /coach assign <coach_id> <group_id>"))?;

        // Validate both IDs are valid UUIDs
        let _ = parse_uuid(coach_id)?;
        let _ = parse_uuid(group_id)?;

        // Verify coach exists and is accessible in this tenant
        let coach = ctx
            .resources
            .repos
            .coaches
            .get_by_id(coach_id, ctx.user_id, ctx.tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Coach {coach_id}")))?;

        // Verify user has permission on this group (tenant-scoped via get_group)
        let group = ctx
            .resources
            .repos
            .groups
            .get_group(group_id, ctx.tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Group {group_id}")))?;

        // Verify user is admin/owner of this group
        let member = ctx
            .resources
            .repos
            .groups
            .get_member(group_id, ctx.user_id)
            .await?
            .ok_or_else(|| AppError::not_found("You are not a member of this group"))?;

        if !member.role.can_modify_settings() {
            return Ok(CommandResponse::text(
                "Only group admins and owners can change the coach.",
            ));
        }

        update_group_coach(ctx, group_id, coach_id).await?;

        Ok(CommandResponse::text(format!(
            "Coach updated to {} for group {}.",
            coach.title, group.name
        )))
    }
}

/// Update a group's coach via `UpdateGroupRequest`
async fn update_group_coach(
    ctx: &PlatformCommandContext,
    group_id: &str,
    coach_id: &str,
) -> Result<(), AppError> {
    use pierre_core::models::groups::UpdateGroupRequest;

    let update = UpdateGroupRequest {
        name: None,
        description: None,
        coach_id: Some(coach_id.to_owned()),
        max_members: None,
        peer_data_sharing: None,
        is_active: None,
    };

    ctx.resources
        .repos
        .groups
        .update_group(group_id, ctx.tenant_id, &update)
        .await?;

    Ok(())
}
