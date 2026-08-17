// ABOUTME: Handlers for /coach slash commands in messaging channels
// ABOUTME: Lists available coaches as interactive cards and handles coach selection for groups
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_core::markdown::strip_emphasis;
use pierre_core::models::coaches::ListCoachesFilter;
use pierre_core::models::GroupRole;
use pierre_core::uuid_utils::parse_uuid;
use pierre_messaging::commands::{CommandAction, CommandResponse};

#[cfg(feature = "tools-groups")]
use pierre_contremaitre::messaging_strings::KEY_COACH_GROUP_CREATED;
use pierre_contremaitre::messaging_strings::KEY_COACH_GROUP_CREATION_UNAVAILABLE;
use pierre_contremaitre::messaging_strings::{
    KEY_COACH_ASSIGN_FORBIDDEN, KEY_COACH_ASSIGN_NOT_A_MEMBER, KEY_COACH_GROUP_UPDATED,
    KEY_COACH_LIST_CARD_TITLE, KEY_COACH_LIST_EMPTY, KEY_COACH_LIST_ITEM,
    KEY_COACH_MULTI_GROUP_CARD_TITLE, KEY_COACH_MULTI_GROUP_ITEM, KEY_COACH_MULTI_GROUP_PROMPT,
    KEY_COACH_NO_DESCRIPTION, KEY_COACH_USER_UPDATED,
};
#[cfg(feature = "tools-groups")]
use pierre_groups::strategies::tier::tier_strategy_for;
use pierre_services::coach_selection::{record_coach_selection, CoachSelectionSource};
use tracing::warn;

use crate::{CallerGroupStanding, CommandHandler, PlatformCommandContext};

/// Maximum number of coaches to display in a single card
const MAX_COACH_BUTTONS: usize = 8;

/// Handler for `/coach` — list available coaches as an interactive card
pub struct CoachListHandler;

#[async_trait]
impl CommandHandler for CoachListHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();
        let filter = ListCoachesFilter::with_defaults();

        let mut coaches = ctx
            .ctx
            .repos()
            .coaches
            .list(ctx.user_id, ctx.tenant_id, &filter)
            .await?;

        // Overlay per-locale translations on title/description/purpose/
        // instructions. Canonical English stays on the coaches row; missing
        // translations fall back to English automatically. Fast-path for
        // locale == "en" avoids a round-trip.
        ctx.ctx
            .repos()
            .coaches
            .apply_translations(&mut coaches, locale)
            .await?;

        if coaches.is_empty() {
            return Ok(CommandResponse::text(reg.render(
                KEY_COACH_LIST_EMPTY,
                locale,
                &[],
            )));
        }

        let no_description = reg.render(KEY_COACH_NO_DESCRIPTION, locale, &[]);
        let mut body = String::with_capacity(512);
        let actions: Vec<CommandAction> = coaches
            .iter()
            .take(MAX_COACH_BUTTONS)
            .map(|item| {
                let category = item.coach.category.display_name();
                let raw_desc = item
                    .coach
                    .description
                    .as_deref()
                    .unwrap_or(no_description.as_str());
                // Coach markdown files use CommonMark emphasis (*x*, **x**,
                // _x_, __x__). Only Slack's renderer interprets asterisks
                // natively; Telegram/Discord/WhatsApp/Messenger render them
                // literally. Strip here so every channel gets uniform plain
                // text.
                let desc = strip_emphasis(raw_desc);
                body.push_str(&reg.render(
                    KEY_COACH_LIST_ITEM,
                    locale,
                    &[&item.coach.title, category, &desc],
                ));

                CommandAction {
                    label: item.coach.title.clone(),
                    action_type: "postback".to_owned(),
                    value: format!("/coach select {}", item.coach.id),
                }
            })
            .collect();

        Ok(CommandResponse::card(
            reg.render(KEY_COACH_LIST_CARD_TITLE, locale, &[]),
            body,
            actions,
        ))
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
            .ctx
            .repos()
            .coaches
            .get_by_id(coach_id, ctx.user_id, ctx.tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Coach {coach_id}")))?;

        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();

        // DM path: coach selection is per-membership. Never auto-creates a
        // group — groups are a group-chat concept. Writes the one selection
        // pointer and renders a DM-flavored confirmation that omits any
        // "for group X" wording.
        if ctx.is_direct_message {
            ctx.ctx
                .repos()
                .tenants
                .set_selected_coach(ctx.tenant_id, ctx.user_id, Some(coach_id))
                .await?;

            record_slash_selection(ctx, coach_id).await;

            return Ok(CommandResponse::text(reg.render(
                KEY_COACH_USER_UPDATED,
                locale,
                &[&coach.title],
            )));
        }

        // Group path: fetch user's groups and filter to only those in the
        // current tenant where the user can modify settings.
        let all_groups = ctx
            .ctx
            .repos()
            .groups
            .list_groups_for_user(ctx.user_id)
            .await?;

        let tenant_groups: Vec<_> = all_groups
            .into_iter()
            .filter(|g| {
                // GroupSummary doesn't carry tenant_id, so we filter by
                // role — only owners/admins should change the group coach.
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

                    // Resolve the tenant's plan tier (mirrors the REST create
                    // path): Starter has group coaching disabled; Professional/
                    // Enterprise cap members per group. The cap is passed into
                    // GroupService::create_group, which owns the clamp and the
                    // Starter rejection. We pre-check the disabled case here to
                    // render the localized "group creation unavailable" reply
                    // rather than surfacing the service's PermissionDenied error.
                    let plan = ctx.ctx.repos().tenants.get_by_id(ctx.tenant_id).await?.plan;
                    let tier_cap = tier_strategy_for(&plan).max_members_per_group();
                    if tier_cap == 0 {
                        return Ok(CommandResponse::text(reg.render(
                            KEY_COACH_GROUP_CREATION_UNAVAILABLE,
                            locale,
                            &[&coach.title],
                        )));
                    }
                    let tier_cap = i32::try_from(tier_cap).unwrap_or(i32::MAX);

                    let request = CreateGroupRequest {
                        name: group_name.clone(),
                        description: Some(format!("Group coached by {}", coach.title)),
                        coach_id: coach.id.to_string(),
                        max_members: Some(tier_cap),
                    };

                    ctx.ctx
                        .group_service()
                        .create_group(&request, ctx.user_id, ctx.tenant_id, tier_cap)
                        .await?;

                    // Creating a group around a coach is also picking that
                    // coach — `group.created` covers the group, this covers
                    // the selection.
                    record_slash_selection(ctx, coach_id).await;

                    return Ok(CommandResponse::text(reg.render(
                        KEY_COACH_GROUP_CREATED,
                        locale,
                        &[&group_name, &coach.title],
                    )));
                }

                #[cfg(not(feature = "tools-groups"))]
                {
                    let _ = group_name;
                    Ok(CommandResponse::text(reg.render(
                        KEY_COACH_GROUP_CREATION_UNAVAILABLE,
                        locale,
                        &[&coach.title],
                    )))
                }
            }
            1 => {
                // Exactly one group — update its coach
                let group = &tenant_groups[0];
                update_group_coach(ctx, &group.id.to_string(), coach_id).await?;

                Ok(CommandResponse::text(reg.render(
                    KEY_COACH_GROUP_UPDATED,
                    locale,
                    &[&coach.title, &group.name],
                )))
            }
            _ => {
                // Multiple groups — ask the user to pick one
                let count = tenant_groups.len().to_string();
                let mut body = String::with_capacity(256);
                body.push_str(&reg.render(
                    KEY_COACH_MULTI_GROUP_PROMPT,
                    locale,
                    &[&count, &coach.title],
                ));

                let actions: Vec<CommandAction> = tenant_groups
                    .iter()
                    .take(MAX_COACH_BUTTONS)
                    .map(|g| {
                        let members = g.member_count.to_string();
                        body.push_str(&reg.render(
                            KEY_COACH_MULTI_GROUP_ITEM,
                            locale,
                            &[&g.name, &members],
                        ));
                        body.push('\n');
                        CommandAction {
                            label: g.name.clone(),
                            action_type: "postback".to_owned(),
                            value: format!("/coach assign {} {}", coach_id, g.id),
                        }
                    })
                    .collect();

                Ok(CommandResponse::card(
                    reg.render(KEY_COACH_MULTI_GROUP_CARD_TITLE, locale, &[]),
                    body,
                    actions,
                ))
            }
        }
    }
}

/// Handler for `/coach assign <coach_id> <group_id>` — bind coach to a specific group
pub struct CoachAssignHandler;

impl CoachAssignHandler {
    /// The single authority on who may assign a coach to a group, shared by
    /// `execute` and `is_available`.
    fn permits(role: GroupRole) -> bool {
        role.can_modify_settings()
    }
}

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
            .ctx
            .repos()
            .coaches
            .get_by_id(coach_id, ctx.user_id, ctx.tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Coach {coach_id}")))?;

        // Verify user has permission on this group (tenant-scoped via get_group)
        let group = ctx
            .ctx
            .repos()
            .groups
            .get_group(group_id, ctx.tenant_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("Group {group_id}")))?;

        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();

        // Verify user is admin/owner of this group
        let member = ctx
            .ctx
            .repos()
            .groups
            .get_member(group_id, ctx.user_id)
            .await?
            .ok_or_else(|| {
                AppError::not_found(reg.render(KEY_COACH_ASSIGN_NOT_A_MEMBER, locale, &[]))
            })?;

        if !Self::permits(member.role) {
            return Ok(CommandResponse::text(reg.render(
                KEY_COACH_ASSIGN_FORBIDDEN,
                locale,
                &[],
            )));
        }

        update_group_coach(ctx, group_id, coach_id).await?;

        Ok(CommandResponse::text(reg.render(
            KEY_COACH_GROUP_UPDATED,
            locale,
            &[&coach.title, &group.name],
        )))
    }

    /// Checks the group named in the arguments, not the conversation's — so
    /// holding the role in *any* group means some invocation succeeds, and
    /// filtering on the conversation's role would hide a command that works.
    fn is_available(&self, standing: &CallerGroupStanding) -> bool {
        standing.highest.is_some_and(Self::permits)
    }
}

/// Update a group's coach via `UpdateGroupRequest`
/// Record the coach selection a `/coach` command just made, emitting the
/// catalogued `coach.selected` event through the shared recorder.
///
/// `/coach select` is the messaging equivalent of the web Coaches UI, so it
/// is the same product event as `POST /api/coaches/{id}/usage` — before this
/// call existed, the slash command was the one selection surface that emitted
/// nothing, and it is the surface most Dravr users actually have.
///
/// Best-effort by design: the selection itself is already persisted by the
/// caller, so a failed usage bump must not turn a working command into an
/// error reply. The coach's visibility is verified by the caller's `get_by_id`
/// lookup, so the recorder's "not visible" branch is unreachable here.
async fn record_slash_selection(ctx: &PlatformCommandContext, coach_id: &str) {
    if let Err(e) = record_coach_selection(
        ctx.ctx.repos().coaches.as_ref(),
        coach_id,
        ctx.user_id,
        ctx.tenant_id,
        CoachSelectionSource::SlashCommand,
    )
    .await
    {
        warn!(coach_id, error = %e, "failed to record coach usage from slash command");
    }
}

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
        respond_mode: None,
        is_active: None,
    };

    ctx.ctx
        .repos()
        .groups
        .update_group(group_id, ctx.tenant_id, &update)
        .await?;

    // Shared by `/coach select` (single-group case) and `/coach assign`, so
    // both surfaces record the selection exactly once.
    record_slash_selection(ctx, coach_id).await;

    Ok(())
}
