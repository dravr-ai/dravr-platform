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
    KEY_GROUP_COACH_DETACHED, KEY_GROUP_CONSENT_UPDATED, KEY_GROUP_CONSENT_USAGE,
    KEY_GROUP_INVITE_FORBIDDEN, KEY_GROUP_LEAVE_PROMPT, KEY_GROUP_LIST_EMPTY,
    KEY_GROUP_LIST_HEADER, KEY_GROUP_LIST_ITEM, KEY_GROUP_MEMBERS_HEADER, KEY_GROUP_MEMBERS_ITEM,
    KEY_GROUP_MEMBERS_UNKNOWN, KEY_GROUP_NOT_A_MEMBER, KEY_GROUP_PEER_SHARING_OFF,
    KEY_GROUP_PEER_SHARING_ON, KEY_GROUP_RESPOND_ALL, KEY_GROUP_RESPOND_MENTIONS,
    KEY_GROUP_RESPOND_STATUS_MENTIONS, KEY_GROUP_RESPOND_USAGE, KEY_GROUP_ROLE_ADMIN,
    KEY_GROUP_ROLE_MEMBER, KEY_GROUP_ROLE_OWNER, KEY_GROUP_STATUS_SUMMARY,
};
use pierre_core::models::coaches::ListCoachesFilter;
#[cfg(feature = "tools-groups")]
use pierre_core::models::groups::GroupInviteKind;
use pierre_core::models::groups::{GroupRespondMode, GroupRole, UpdateGroupRequest};
use pierre_core::models::TenantId;
use uuid::Uuid;

use crate::{CommandHandler, PlatformCommandContext};

/// The coaching group a `/group` subcommand acts on, together with the tenant
/// whose rows describe it.
struct TargetGroup {
    /// `coaching_groups.id`.
    id: Uuid,
    /// Display name, echoed back in confirmations.
    name: String,
    /// Tenant that owns the `coaching_groups` row — the conversation tenant
    /// when the chat binding supplied the group, the caller's own tenant on the
    /// `list_groups_for_user` path (which reports no tenant).
    tenant_id: TenantId,
    /// How the group was resolved, for the operator log line: either
    /// `"conversation_group_id"` or `"list_groups_for_user_first"`.
    source: &'static str,
}

/// Resolve the group bound to this turn's conversation.
///
/// `Ok(None)` means the conversation exists and simply carries no group — a
/// personal chat. A conversation id that is invisible under
/// [`PlatformCommandContext::conversation_tenant_id`], or a binding pointing at
/// a group that tenant cannot see, is an error: the caller must refuse rather
/// than pick some other group.
async fn conversation_bound_group(
    ctx: &PlatformCommandContext,
    conversation_id: &str,
) -> Result<Option<TargetGroup>, AppError> {
    let reg = ctx.ctx.messaging_strings_registry();
    let locale = ctx.locale.as_str();

    let conversation = ctx
        .ctx
        .repos()
        .chat
        .get_conversation(
            conversation_id,
            &ctx.user_id.to_string(),
            ctx.conversation_tenant_id,
        )
        .await?
        .ok_or_else(|| AppError::not_found(reg.render(KEY_GROUP_NOT_A_MEMBER, locale, &[])))?;

    let Some(group_id) = conversation.group_id else {
        return Ok(None);
    };

    let group = ctx
        .ctx
        .repos()
        .groups
        .get_group(&group_id, ctx.conversation_tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(reg.render(KEY_GROUP_NOT_A_MEMBER, locale, &[])))?;

    Ok(Some(TargetGroup {
        id: group.id,
        name: group.name,
        tenant_id: ctx.conversation_tenant_id,
        source: "conversation_group_id",
    }))
}

/// Resolve the coaching group a `/group` subcommand acts on.
///
/// The conversation carrying the turn is the authority. Its `group_id` and the
/// group it names are read under [`PlatformCommandContext::conversation_tenant_id`]
/// — a shared room's conversation and coaching group belong to the channel/bot
/// tenant, which is not the caller's tenant whenever a member joined from
/// elsewhere.
///
/// `list_groups_for_user` is the fallback only for personal chats and for
/// surfaces that carry no conversation id at all (Slack `block_actions`
/// buttons). It spans tenants, applies no tenant filter and orders by
/// `updated_at DESC`, so using it after a failed conversation lookup would
/// silently point the command at whichever other group the caller touched last.
/// For a privacy control such as `/group consent` a visible refusal is the
/// safer failure, so a shared room that cannot name its group refuses.
async fn resolve_target_group(ctx: &PlatformCommandContext) -> Result<TargetGroup, AppError> {
    let reg = ctx.ctx.messaging_strings_registry();
    let locale = ctx.locale.as_str();

    find_target_group(ctx)
        .await?
        .ok_or_else(|| AppError::not_found(reg.render(KEY_GROUP_NOT_A_MEMBER, locale, &[])))
}

/// The same resolution as [`resolve_target_group`], reporting "no group" as
/// `None` instead of an error.
///
/// Split out so `/help` can ask which group a command *would* act on without
/// treating the absence of one as a failure. Keeping one ladder matters: a
/// second copy would drift, and `/help` would then advertise a different group
/// than the handlers act on.
async fn find_target_group(ctx: &PlatformCommandContext) -> Result<Option<TargetGroup>, AppError> {
    if let Some(conversation_id) = ctx.conversation_id.as_deref() {
        if let Some(group) = conversation_bound_group(ctx, conversation_id).await? {
            return Ok(Some(group));
        }
        if !ctx.is_direct_message {
            return Ok(None);
        }
    }

    let groups = ctx
        .ctx
        .repos()
        .groups
        .list_groups_for_user(ctx.user_id)
        .await?;

    Ok(groups.first().map(|group| TargetGroup {
        id: group.id,
        name: group.name.clone(),
        tenant_id: ctx.tenant_id,
        source: "list_groups_for_user_first",
    }))
}

/// Everything `/help` needs to ask a handler whether it would refuse the
/// caller, resolved once per turn.
///
/// Two facts, because the handlers genuinely ask two different questions
/// about the caller's groups:
///
/// - `/group invite`, `/group coach`, `/group respond`, `/group consent`
///   resolve the conversation's group and check the caller's standing
///   *there*. `ambient` answers them.
/// - `/group status`, `/group members`, `/group leave` read
///   `list_groups_for_user().first()` instead — any group the caller belongs
///   to will do, and the conversation's group is irrelevant. `/coach assign
///   <coach-id> <group-id>` checks the group the caller *typed*, which no
///   conversation-scoped fact can decide. `highest` answers all four, because
///   holding a role anywhere means some invocation succeeds.
pub struct CallerGroupStanding {
    /// The caller's role in the group [`resolve_target_group`] names — `None`
    /// when no group resolves, or when the caller is not one of its members.
    pub ambient: Option<GroupRole>,
    /// The strongest role the caller holds in any group — `None` when they
    /// belong to none, which also answers "does this caller have a group".
    pub highest: Option<GroupRole>,
}

/// Rank the roles so "strongest held" is well defined. `Owner` outranks
/// `Admin` outranks `Member`; [`GroupRole`] itself is deliberately not `Ord`,
/// since ordering roles only means something for this one comparison.
const fn role_rank(role: GroupRole) -> u8 {
    match role {
        GroupRole::Owner => 2,
        GroupRole::Admin => 1,
        GroupRole::Member => 0,
    }
}

/// Resolve the caller's standing for `/help`, so it can ask each handler
/// whether that handler would refuse.
///
/// Walks the same ladder the handlers do and reads the same membership row
/// they check (`get_member`), then adds the group list the other half of them
/// read instead. Two queries beyond what a single group command costs, paid
/// once for the whole listing.
///
/// `pub` because the command-catalogue route filters its listing through the
/// same standing: a palette that offers `/group invite` to an athlete in no
/// group is lying, and re-deriving the ladder there would be a second source
/// of truth for group standing.
///
/// # Errors
///
/// Returns the underlying repository error when the caller's groups or their
/// membership row cannot be read.
pub async fn caller_group_standing(
    ctx: &PlatformCommandContext,
) -> Result<CallerGroupStanding, AppError> {
    let ambient = match find_target_group(ctx).await? {
        Some(group) => ctx
            .ctx
            .repos()
            .groups
            .get_member(&group.id.to_string(), ctx.user_id)
            .await?
            .map(|m| m.role),
        None => None,
    };

    // `GroupSummary` already carries the caller's role, so the strongest role
    // costs no lookup beyond this one list.
    let highest = ctx
        .ctx
        .repos()
        .groups
        .list_groups_for_user(ctx.user_id)
        .await?
        .iter()
        .map(|g| g.my_role)
        .max_by_key(|role| role_rank(*role));

    Ok(CallerGroupStanding { ambient, highest })
}

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

        let mut text = reg.render(
            KEY_GROUP_STATUS_SUMMARY,
            locale,
            &[&group.name, &mc, &active, &peer_sharing],
        );

        // Respond mode lives on the full group row (the summary omits it).
        // Plain-text suffix, mirroring the /group coach and /group respond
        // replies that stay outside the tools-groups-gated string registry.
        if let Ok(Some(full)) = ctx
            .ctx
            .repos()
            .groups
            .get_group(&group.id.to_string(), ctx.tenant_id)
            .await
        {
            if full.respond_mode == GroupRespondMode::Mentions {
                text.push('\n');
                text.push_str(&reg.render(KEY_GROUP_RESPOND_STATUS_MENTIONS, locale, &[]));
            }
        }

        Ok(CommandResponse::text(text))
    }

    /// Reads `list_groups_for_user().first()`, so any group the caller belongs
    /// to will do — the conversation's group never enters into it.
    fn is_available(&self, standing: &CallerGroupStanding) -> bool {
        standing.highest.is_some()
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

    /// Reads `list_groups_for_user().first()`, so any group the caller belongs
    /// to will do — the conversation's group never enters into it.
    fn is_available(&self, standing: &CallerGroupStanding) -> bool {
        standing.highest.is_some()
    }
}

/// Handler for `/group invite` — generate invite link
pub struct GroupInviteHandler;

impl GroupInviteHandler {
    /// The single authority on who may invite. `execute` checks it against the
    /// membership row it fetched; `is_available` checks it against the role
    /// `/help` already resolved for the same group.
    fn permits(role: GroupRole) -> bool {
        role.can_manage_members()
    }
}

#[async_trait]
impl CommandHandler for GroupInviteHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();

        let group = resolve_target_group(ctx).await?;

        // Check admin role
        let member = ctx
            .ctx
            .repos()
            .groups
            .get_member(&group.id.to_string(), ctx.user_id)
            .await?
            .ok_or_else(|| AppError::not_found("Membership not found"))?;

        if !Self::permits(member.role) {
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

            // The invite is filed under the tenant that owns the group, not the
            // issuer's — in a shared room those differ.
            let invite = ctx
                .ctx
                .group_service()
                .create_invite(group.id, ctx.user_id, group.tenant_id, Some(7), None, kind)
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

    /// Acts on the conversation's group, so the caller's role *there* decides.
    fn is_available(&self, standing: &CallerGroupStanding) -> bool {
        standing.ambient.is_some_and(Self::permits)
    }
}

/// Handler for `/group coach <name>` and `/group coach detach`.
///
/// Owner/admin only, both forms. `<name>` resolves a Dravr coach by
/// (case-insensitive) title from the coaches visible to the caller and points
/// the group's `coach_id` at it, so that persona answers in the group
/// thereafter. `detach` clears the group's *human* coach
/// (`coaching_groups.coach_user_id`) — the attachment `/group invite coach`
/// creates — leaving the AI persona untouched, and is the only way to undo it
/// from chat.
pub struct GroupCoachHandler;

impl GroupCoachHandler {
    /// The single authority on who may change the group's coach, shared by
    /// `execute` and `is_available`.
    fn permits(role: GroupRole) -> bool {
        role.can_manage_members()
    }

    /// Clear `coaching_groups.coach_user_id` for the resolved group.
    ///
    /// Called only after `execute` has confirmed the caller is an owner or
    /// admin of that group. Scoped to the tenant that owns the group row,
    /// which is the channel tenant when the command came from a shared room.
    async fn detach_human_coach(
        ctx: &PlatformCommandContext,
        group: &TargetGroup,
    ) -> Result<CommandResponse, AppError> {
        let cleared = ctx
            .ctx
            .group_service()
            .set_group_coach(&group.id.to_string(), None, group.tenant_id)
            .await?;
        if !cleared {
            return Err(AppError::not_found("Group not found"));
        }

        info!(
            group_id = %group.id,
            "Group human coach detached via /group coach detach"
        );

        Ok(CommandResponse::text(
            ctx.ctx.messaging_strings_registry().render(
                KEY_GROUP_COACH_DETACHED,
                ctx.locale.as_str(),
                &[&group.name],
            ),
        ))
    }
}

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
                "Usage: /group coach <name> — set this group's Dravr coach (e.g. /group coach 5K Marathon), \
                 or /group coach detach to remove the group's human coach."
                    .to_owned(),
            ));
        }

        // Resolve the chat-bound group (mirrors /group invite).
        let group = resolve_target_group(ctx).await?;

        // Owner/admin only — changing the group's coach is a settings change.
        let member = ctx
            .ctx
            .repos()
            .groups
            .get_member(&group.id.to_string(), ctx.user_id)
            .await?
            .ok_or_else(|| AppError::not_found("Membership not found"))?;
        if !Self::permits(member.role) {
            return Ok(CommandResponse::text(reg.render(
                KEY_GROUP_INVITE_FORBIDDEN,
                locale,
                &[],
            )));
        }

        // `detach` clears the human coach attached by `/group invite coach`.
        // It runs behind the same owner/admin gate as setting the persona, and
        // is deliberately not a coach *title*: a Dravr coach named "detach"
        // would still be reachable as `/group coach detach coach`.
        if name.eq_ignore_ascii_case("detach") {
            return Self::detach_human_coach(ctx, &group).await;
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
            respond_mode: None,
            is_active: None,
        };
        // Scoped to the tenant that owns the group row, which is the channel
        // tenant when the command came from a shared room.
        let updated = ctx
            .ctx
            .group_service()
            .update_group(&group.id.to_string(), group.tenant_id, &request)
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

    /// Acts on the conversation's group, so the caller's role *there* decides.
    fn is_available(&self, standing: &CallerGroupStanding) -> bool {
        standing.ambient.is_some_and(Self::permits)
    }
}

/// Handler for `/group respond <mentions|all>` — set when the group's AI
/// coach replies in the bound chat.
///
/// Owner/admin only. `mentions` restricts replies to explicitly-addressed
/// messages (an @-mention of the bot or a reply to one of its messages);
/// unaddressed chatter is captured silently as ambient context. `all`
/// restores the answer-everything default. Replies in plain text so it does
/// not depend on the `tools-groups`-gated messaging strings (mirrors
/// `/group coach`).
pub struct GroupRespondHandler;

impl GroupRespondHandler {
    /// The single authority on who may change when the coach speaks, shared by
    /// `execute` and `is_available`.
    fn permits(role: GroupRole) -> bool {
        role.can_modify_settings()
    }
}

#[async_trait]
impl CommandHandler for GroupRespondHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();

        let mode = match ctx
            .args
            .first()
            .map(|a| a.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("mentions" | "mention") => GroupRespondMode::Mentions,
            Some("all" | "everything") => GroupRespondMode::All,
            _ => {
                return Ok(CommandResponse::text(reg.render(
                    KEY_GROUP_RESPOND_USAGE,
                    locale,
                    &[],
                )))
            }
        };

        // Resolve the chat-bound group (mirrors /group coach).
        let group = resolve_target_group(ctx).await?;

        // Owner/admin only — changing when the coach speaks is a settings change.
        let member = ctx
            .ctx
            .repos()
            .groups
            .get_member(&group.id.to_string(), ctx.user_id)
            .await?
            .ok_or_else(|| AppError::not_found("Membership not found"))?;
        if !Self::permits(member.role) {
            return Ok(CommandResponse::text(reg.render(
                KEY_GROUP_INVITE_FORBIDDEN,
                locale,
                &[],
            )));
        }

        let request = UpdateGroupRequest {
            name: None,
            description: None,
            coach_id: None,
            max_members: None,
            peer_data_sharing: None,
            respond_mode: Some(mode),
            is_active: None,
        };
        // Scoped to the tenant that owns the group row, which is the channel
        // tenant when the command came from a shared room.
        let updated = ctx
            .ctx
            .group_service()
            .update_group(&group.id.to_string(), group.tenant_id, &request)
            .await?
            .ok_or_else(|| AppError::not_found("Group not found"))?;

        info!(
            group_id = %updated.id,
            respond_mode = %mode,
            "Group respond mode updated via /group respond"
        );

        // Deliberately no group name in the body: this reply is posted into the
        // shared room, and an auto-bound group carries the synthetic
        // "{channel} group {chat_id}" label, which would put a raw chat id in
        // front of every member.
        let key = match mode {
            GroupRespondMode::Mentions => KEY_GROUP_RESPOND_MENTIONS,
            GroupRespondMode::All => KEY_GROUP_RESPOND_ALL,
        };
        Ok(CommandResponse::text(reg.render(key, locale, &[])))
    }

    /// Acts on the conversation's group, so the caller's role *there* decides.
    fn is_available(&self, standing: &CallerGroupStanding) -> bool {
        standing.ambient.is_some_and(Self::permits)
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

    /// Reads `list_groups_for_user().first()`, so any group the caller belongs
    /// to will do — the conversation's group never enters into it.
    fn is_available(&self, standing: &CallerGroupStanding) -> bool {
        standing.highest.is_some()
    }
}

/// Handler for `/group consent yes|no` — toggle peer-sharing consent.
///
/// Updates `coaching_group_members.peer_sharing_consent` for the requester
/// in the group [`resolve_target_group`] names — the group bound to the chat
/// the command was typed in, refusing rather than retargeting when a shared
/// room cannot name one.
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

        let target = resolve_target_group(ctx).await?;
        let group_id_str = target.id.to_string();
        let group_name = target.name;

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
            source = target.source,
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

    /// Acts on the conversation's group and refuses a non-member — the consent
    /// write reports zero rows and `execute` turns that into "not a member".
    fn is_available(&self, standing: &CallerGroupStanding) -> bool {
        standing.ambient.is_some()
    }
}
