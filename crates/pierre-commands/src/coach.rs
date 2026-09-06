// ABOUTME: Handlers for the /agent command tree — list installed agents, add one to a conversation, remove it, assign to a group
// ABOUTME: One binding implementation behind /agent add and the confirm step of /agent create
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_core::markdown::strip_emphasis;
use pierre_core::models::coaches::{Coach, CoachHandle, ListCoachesFilter};
use pierre_core::models::groups::GroupInviteKind;
use pierre_core::models::{GroupRole, TenantId};
use pierre_core::uuid_utils::parse_uuid;
use pierre_messaging::commands::{CommandAction, CommandResponse};

use pierre_contremaitre::messaging_strings::{
    KEY_COACH_ADD_UNKNOWN, KEY_COACH_ADD_USAGE, KEY_COACH_ASSIGN_FORBIDDEN,
    KEY_COACH_ASSIGN_NOT_A_MEMBER, KEY_COACH_GROUP_UPDATED, KEY_COACH_LIST_CARD_TITLE,
    KEY_COACH_LIST_EMPTY, KEY_COACH_LIST_FOOTER, KEY_COACH_LIST_ITEM,
    KEY_COACH_LIST_ITEM_NO_HANDLE, KEY_COACH_NO_DESCRIPTION, KEY_COACH_REMOVED,
    KEY_COACH_REMOVE_GROUP_THREAD, KEY_COACH_REMOVE_NOTHING, KEY_COACH_USER_UPDATED,
};
use pierre_services::coach_selection::{record_coach_selection, CoachSelectionSource};
use tracing::warn;

use crate::group::{issue_group_invite, resolve_target_group, GroupInviteHandler};
use crate::{CallerGroupStanding, CommandHandler, PlatformCommandContext};

/// Maximum number of coaches offered as buttons on the list card.
const MAX_COACH_BUTTONS: usize = 8;

/// Handler for `/agent` (also `/agent list`, and the legacy `/coach` spellings) — the caller's own
/// coach list as an interactive card.
///
/// The list is what `find_installed_by_handle` resolves against: the coaches
/// the caller created and the ones they installed from Discover. System
/// coaches they never installed are deliberately absent — `/discover` is the
/// catalogue, this is the shelf — so every entry here can be added to a
/// conversation, and each carries the `@handle` that does it.
pub struct CoachListHandler;

#[async_trait]
impl CommandHandler for CoachListHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();
        let mut coaches = ctx
            .ctx
            .repos()
            .coaches
            .list(ctx.user_id, ctx.tenant_id, &installed_filter())
            .await?;

        // Overlay per-locale translations on title/description. Canonical
        // English stays on the coaches row; missing translations fall back to
        // English automatically. Fast-path for locale == "en" avoids a
        // round-trip.
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
        for item in &coaches {
            let raw_desc = item
                .coach
                .description
                .as_deref()
                .unwrap_or(no_description.as_str());
            // Coach markdown files use CommonMark emphasis (*x*, **x**, _x_,
            // __x__). Only Slack's renderer interprets asterisks natively;
            // Telegram/Discord/WhatsApp/Messenger render them literally. Strip
            // here so every channel gets uniform plain text.
            let desc = strip_emphasis(raw_desc);
            let line = item.coach.handle.as_deref().map_or_else(
                || {
                    reg.render(
                        KEY_COACH_LIST_ITEM_NO_HANDLE,
                        locale,
                        &[&item.coach.title, &desc],
                    )
                },
                |handle| {
                    reg.render(
                        KEY_COACH_LIST_ITEM,
                        locale,
                        &[&item.coach.title, handle, &desc],
                    )
                },
            );
            body.push_str(&line);
        }
        body.push('\n');
        body.push_str(&reg.render(KEY_COACH_LIST_FOOTER, locale, &[]));

        let actions: Vec<CommandAction> = coaches
            .iter()
            .take(MAX_COACH_BUTTONS)
            .map(|item| CommandAction {
                label: item.coach.title.clone(),
                action_type: "postback".to_owned(),
                value: add_postback(&item.coach),
            })
            .collect();

        Ok(CommandResponse::card(
            reg.render(KEY_COACH_LIST_CARD_TITLE, locale, &[]),
            body,
            actions,
        ))
    }
}

/// The `/agent add` text that names `coach`: its handle when it owns one,
/// its id otherwise — an agent created through the editor owns none until
/// the Store approves it.
///
/// Bounded by construction: `/agent add @` plus a handle of at most
/// [`CoachHandle::MAX_LEN`] characters is 52 bytes, and the id form is 47 —
/// both under the 64-byte ceiling Telegram puts on a button's callback data.
fn add_postback(coach: &Coach) -> String {
    coach.handle.as_deref().map_or_else(
        || format!("/agent add {}", coach.id),
        |handle| format!("/agent add @{handle}"),
    )
}

/// Handler for `/agent add @handle` — bring an installed agent into this
/// conversation.
///
/// The argument is the catalogue handle shown by `/agent`, or an agent id (the
/// form the list card sends for a coach that owns no handle). Either way the
/// coach has to be on the caller's list: a catalogue coach they never
/// installed is refused by name, and no `/discover install` happens on their
/// behalf. Everything after the lookup is [`bind_coach`].
///
/// Listed for every caller: in a personal conversation it always works, and
/// the group-thread refusal is decided by a role no group standing can see
/// without also knowing whether the conversation is personal.
pub struct CoachAddHandler;

#[async_trait]
impl CommandHandler for CoachAddHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();

        let typed = ctx.args.first().map_or("", String::as_str).trim();
        if typed.is_empty() {
            return Ok(CommandResponse::text(reg.render(
                KEY_COACH_ADD_USAGE,
                locale,
                &[],
            )));
        }

        let Some(coach) = resolve_listed_coach(ctx, typed).await? else {
            let shown = if typed.starts_with('@') {
                typed.to_owned()
            } else {
                format!("@{typed}")
            };
            return Ok(CommandResponse::text(reg.render(
                KEY_COACH_ADD_UNKNOWN,
                locale,
                &[&shown],
            )));
        };

        let binding = bind_coach(ctx, &coach).await?;
        let text = match &binding {
            CoachBinding::Personal => reg.render(KEY_COACH_USER_UPDATED, locale, &[&coach.title]),
            CoachBinding::Group(group_name) => {
                reg.render(KEY_COACH_GROUP_UPDATED, locale, &[&coach.title, group_name])
            }
            CoachBinding::Refused => reg.render(KEY_COACH_ASSIGN_FORBIDDEN, locale, &[]),
        };
        Ok(CommandResponse::text(text))
    }
}

/// The caller's own coach list: the coaches they created and the ones they
/// installed from Discover — the set `/agent` shows and `/agent add` resolves
/// against. System coaches they never installed are not on it.
fn installed_filter() -> ListCoachesFilter {
    ListCoachesFilter {
        include_system: false,
        ..ListCoachesFilter::with_defaults()
    }
}

/// Resolve the argument of `/agent add` to an agent on the caller's list.
///
/// A catalogue handle resolves through `find_installed_by_handle`. A coach id
/// — the form the list card sends for a coach that owns no handle — resolves
/// within that same list, so neither form reaches a coach the caller never
/// installed. A token the handle grammar refuses (`@Coach Tempo`) is the same
/// case as a well-formed handle nobody owns: neither names a listed coach.
async fn resolve_listed_coach(
    ctx: &PlatformCommandContext,
    typed: &str,
) -> Result<Option<Coach>, AppError> {
    let coaches = &ctx.ctx.repos().coaches;
    if let Ok(id) = parse_uuid(typed) {
        return Ok(coaches
            .list(ctx.user_id, ctx.tenant_id, &installed_filter())
            .await?
            .into_iter()
            .map(|item| item.coach)
            .find(|coach| coach.id == id));
    }
    match CoachHandle::parse(typed) {
        Ok(handle) => {
            coaches
                .find_installed_by_handle(&handle, ctx.user_id, ctx.tenant_id)
                .await
        }
        Err(_) => Ok(None),
    }
}

/// Where a coach ended up after [`bind_coach`].
pub(crate) enum CoachBinding {
    /// A personal conversation: the selection pointer moved and the
    /// conversation rebound.
    Personal,
    /// A group conversation: the named group's coach changed and the
    /// conversation rebound.
    Group(String),
    /// A group conversation whose settings the caller may not change.
    Refused,
}

/// Make `coach` answer in the conversation the command was typed in.
///
/// The one implementation behind `/agent add` and the confirm step of
/// `/agent create`: the two differ only in where the agent comes from.
///
/// In a personal conversation the selection is per-membership: the selection
/// pointer moves and the conversation the command was typed in rebinds (see
/// [`bind_conversation_coach`]), so the coach answers from the next message
/// on. In a group conversation the coach becomes the group's — a settings
/// change, so only an owner or admin of that group may make it — and the
/// conversation rebinds the same way.
pub(crate) async fn bind_coach(
    ctx: &PlatformCommandContext,
    coach: &Coach,
) -> Result<CoachBinding, AppError> {
    let coach_id = coach.id.to_string();
    let coach_id = coach_id.as_str();

    if ctx.is_direct_message {
        ctx.ctx
            .repos()
            .tenants
            .set_selected_coach(ctx.tenant_id, ctx.user_id, Some(coach_id))
            .await?;
        bind_conversation_coach(ctx, coach_id).await?;
        record_slash_selection(ctx, coach_id).await;
        return Ok(CoachBinding::Personal);
    }

    let reg = ctx.ctx.messaging_strings_registry();
    let group = resolve_target_group(ctx).await?;
    let member = ctx
        .ctx
        .repos()
        .groups
        .get_member(&group.id.to_string(), ctx.user_id)
        .await?
        .ok_or_else(|| {
            AppError::not_found(reg.render(KEY_COACH_ASSIGN_NOT_A_MEMBER, ctx.locale.as_str(), &[]))
        })?;
    if !CoachAssignHandler::permits(member.role) {
        return Ok(CoachBinding::Refused);
    }

    update_group_coach(ctx, &group.id.to_string(), group.tenant_id, coach_id).await?;
    bind_conversation_coach(ctx, coach_id).await?;
    Ok(CoachBinding::Group(group.name))
}

/// Point the conversation the command was typed in at `coach_id`.
///
/// The selection pointer alone reaches only conversations opened *after* it:
/// a web thread bound at creation kept its old coach for as long as it lived,
/// and a messaging thread waited for the next inbound turn to notice. Writing
/// the row here is what makes adding a coach mean it answers the next
/// message in this very thread, on every surface. The row is written only
/// when it is the caller's own; a dispatch site with no conversation (a Slack
/// button, the catalogue read) has nothing to bind.
async fn bind_conversation_coach(
    ctx: &PlatformCommandContext,
    coach_id: &str,
) -> Result<(), AppError> {
    let Some(conversation_id) = ctx.conversation_id.as_deref() else {
        return Ok(());
    };
    let chat = &ctx.ctx.repos().chat;
    let Some(conversation) = chat
        .get_conversation(
            conversation_id,
            &ctx.user_id.to_string(),
            ctx.conversation_tenant_id,
        )
        .await?
    else {
        return Ok(());
    };
    if conversation.coach_id.as_deref() == Some(coach_id) {
        return Ok(());
    }
    chat.set_conversation_coach_id(conversation_id, Some(coach_id), ctx.conversation_tenant_id)
        .await?;
    Ok(())
}

/// Handler for `/agent remove` — detach this conversation's agent.
///
/// Personal conversations only: in a group conversation the coach is the
/// group's, and `/group coach` is the command that changes it. Clears both
/// the conversation row and the selection pointer — a messaging thread
/// re-applies the pointer on every inbound turn, so clearing the row alone
/// would bring the coach back with the athlete's next message.
pub struct CoachRemoveHandler;

#[async_trait]
impl CommandHandler for CoachRemoveHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();

        if !ctx.is_direct_message {
            return Ok(CommandResponse::text(reg.render(
                KEY_COACH_REMOVE_GROUP_THREAD,
                locale,
                &[],
            )));
        }
        let nothing = || CommandResponse::text(reg.render(KEY_COACH_REMOVE_NOTHING, locale, &[]));
        let Some(conversation_id) = ctx.conversation_id.as_deref() else {
            return Ok(nothing());
        };
        let chat = &ctx.ctx.repos().chat;
        let Some(conversation) = chat
            .get_conversation(
                conversation_id,
                &ctx.user_id.to_string(),
                ctx.conversation_tenant_id,
            )
            .await?
        else {
            return Ok(nothing());
        };
        let Some(coach_id) = conversation.coach_id else {
            return Ok(nothing());
        };

        // The title is only decoration on the confirmation; a coach deleted
        // from under the conversation still gets detached.
        let title = ctx
            .ctx
            .repos()
            .coaches
            .get_by_id(&coach_id, ctx.user_id, ctx.tenant_id)
            .await?
            .map_or_else(|| "Agent".to_owned(), |coach| coach.title);

        chat.set_conversation_coach_id(conversation_id, None, ctx.conversation_tenant_id)
            .await?;
        ctx.ctx
            .repos()
            .tenants
            .set_selected_coach(ctx.tenant_id, ctx.user_id, None)
            .await?;

        Ok(CommandResponse::text(reg.render(
            KEY_COACH_REMOVED,
            locale,
            &[&title],
        )))
    }
}

/// Handler for `/agent invite` — the `/agent`-domain spelling of
/// `/group invite coach`.
///
/// Both run [`issue_group_invite`], so whoever redeems the code is attached
/// as the group's human coach either way. Bringing a Dravr coach into a
/// conversation is `/agent add`, not this.
pub struct CoachInviteHandler;

#[async_trait]
impl CommandHandler for CoachInviteHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        issue_group_invite(ctx, GroupInviteKind::Coach).await
    }

    /// Acts on the conversation's group, so the caller's role *there* decides
    /// — the same gate [`issue_group_invite`] enforces.
    fn is_available(&self, standing: &CallerGroupStanding) -> bool {
        standing.ambient.is_some_and(GroupInviteHandler::permits)
    }
}

/// Handler for `/agent assign <coach_id> <group_id>` — bind an agent to a specific group
pub struct CoachAssignHandler;

impl CoachAssignHandler {
    /// The single authority on who may change a group's coach, shared by
    /// `execute`, `is_available` and the group branch of [`bind_coach`].
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
            .ok_or_else(|| AppError::invalid_input("Usage: /agent assign <coach_id> <group_id>"))?;
        let group_id = ctx
            .args
            .get(1)
            .ok_or_else(|| AppError::invalid_input("Usage: /agent assign <coach_id> <group_id>"))?;

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

        update_group_coach(ctx, group_id, ctx.tenant_id, coach_id).await?;

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

/// Record the agent selection an `/agent` command just made, emitting the
/// catalogued `coach.selected` event through the shared recorder.
///
/// `/agent add` is the chat equivalent of picking an agent on Discover, so it
/// is the same product event as `POST /api/coaches/{id}/usage` — before this
/// call existed, the slash command was the one selection surface that emitted
/// nothing, and it is the surface most Dravr users actually have.
///
/// Best-effort by design: the selection itself is already persisted by the
/// caller, so a failed usage bump must not turn a working command into an
/// error reply. The coach's visibility is verified by the caller's lookup, so
/// the recorder's "not visible" branch is unreachable here.
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

/// Point a group at `coach_id` and record the selection.
///
/// `group_tenant_id` is the tenant that owns the `coaching_groups` row — the
/// conversation tenant when the chat binding supplied the group (a shared
/// room's group belongs to the channel tenant), the caller's own tenant for
/// `/agent assign`, which names the group by id in the caller's scope.
async fn update_group_coach(
    ctx: &PlatformCommandContext,
    group_id: &str,
    group_tenant_id: TenantId,
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
        .update_group(group_id, group_tenant_id, &update)
        .await?;

    // Shared by `/agent add` (group conversation) and `/agent assign`, so
    // both surfaces record the selection exactly once.
    record_slash_selection(ctx, coach_id).await;

    Ok(())
}
