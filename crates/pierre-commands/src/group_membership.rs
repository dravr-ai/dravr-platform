// ABOUTME: Handlers for /group create and /group join — how a coaching group enters a conversation list
// ABOUTME: Create binds the fresh thread it was typed in; join files the member's own group-scoped conversation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_contremaitre::messaging_strings::{
    KEY_GROUP_CREATED, KEY_GROUP_CREATE_FORBIDDEN, KEY_GROUP_CREATE_NO_COACH,
    KEY_GROUP_CREATE_UNAVAILABLE, KEY_GROUP_CREATE_USAGE, KEY_GROUP_INVITE_LABEL, KEY_GROUP_JOINED,
    KEY_GROUP_JOINED_AS_COACH, KEY_GROUP_JOIN_ALREADY_MEMBER, KEY_GROUP_JOIN_FULL,
    KEY_GROUP_JOIN_INVALID_CODE,
};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::coaches::Coach;
use pierre_core::models::groups::{CoachingGroup, CreateGroupRequest, GroupInviteKind};
use pierre_core::models::{ConversationRecord, TenantId};
use pierre_groups::creation_policy::{check_create_group_permission, GROUP_CREATION_POLICY_KEY};
use pierre_groups::strategies::tier::tier_strategy_for;
use pierre_messaging::commands::{CommandAction, CommandResponse};
use pierre_runtime_context::ConfigLookupScope;
use tracing::info;

use crate::{CommandHandler, PlatformCommandContext};

/// The conversation the command was typed in, when the surface has one and
/// the caller is in it. `None` on a synthetic dispatch (a Slack button).
async fn typed_in_thread(
    ctx: &PlatformCommandContext,
) -> Result<Option<ConversationRecord>, AppError> {
    let Some(conversation_id) = ctx.conversation_id.as_deref() else {
        return Ok(None);
    };
    ctx.ctx
        .repos()
        .chat
        .get_conversation(
            conversation_id,
            &ctx.user_id.to_string(),
            ctx.conversation_tenant_id,
        )
        .await
}

/// Handler for `/group create <name>`.
///
/// Creates a coaching group around the coach of the chat it is typed in
/// (else the caller's selected coach), behind the same gates as the web and
/// mobile create flows: the tenant plan must include group coaching and the
/// tenant's `group_creation_policy` decides whether a non-admin may create.
/// The creator then gets a conversation that is the group's chat — see
/// [`GroupCreateHandler::file_creator_conversation`].
pub struct GroupCreateHandler;

impl GroupCreateHandler {
    /// The coach the new group answers with: the thread's own coach, else the
    /// caller's selected coach. Verified visible to the caller, so a stale
    /// pointer at a deleted coach reads as "no coach" rather than creating a
    /// group nobody can talk to.
    async fn resolve_group_coach(
        ctx: &PlatformCommandContext,
        thread: Option<&ConversationRecord>,
    ) -> Result<Option<Coach>, AppError> {
        let repos = ctx.ctx.repos();
        let coach_id = match thread.and_then(|t| t.coach_id.clone()) {
            Some(id) => Some(id),
            None => {
                repos
                    .tenants
                    .get_selected_coach(ctx.tenant_id, ctx.user_id)
                    .await?
            }
        };
        let Some(coach_id) = coach_id else {
            return Ok(None);
        };
        repos
            .coaches
            .get_by_id(&coach_id, ctx.user_id, ctx.tenant_id)
            .await
    }

    /// The tenant's creation policy, read lazily through the admin config
    /// exactly as `POST /api/groups` reads it. `None` — no admin config
    /// wired, or no policy set — is the default policy.
    async fn creation_policy(ctx: &PlatformCommandContext) -> Option<String> {
        let lookup = ctx.ctx.admin_config()?;
        let tenant = ctx.tenant_id.to_string();
        lookup
            .get_value(
                GROUP_CREATION_POLICY_KEY,
                ConfigLookupScope::tenant(&tenant),
            )
            .await
            .ok()
            .flatten()
            .and_then(|v| v.as_str().map(ToOwned::to_owned))
    }

    /// Give the creator a conversation that is the group's chat.
    ///
    /// An in-app thread with no group and no coaching turn yet — the one the
    /// apps open for "New group chat" before sending this command — becomes
    /// it: bound to the group, pointed at its coach and renamed after it. A
    /// slash command's own rows (stamped
    /// [`COMMAND_FINISH_REASON`](pierre_core::models::COMMAND_FINISH_REASON),
    /// this command's line included) do not count as history: a thread whose
    /// only rows are commands has never held a coaching turn. Any
    /// other thread is left as it is and a group-scoped conversation named
    /// after the group is created beside it, which is how a member's group
    /// chat is made everywhere else (`POST /api/chat/conversations` with a
    /// `group_id`, `/group join`). A messaging DM's session conversation is
    /// the athlete's one thread on that channel and stays theirs, so it is
    /// never bound; a dispatch with no conversation has nothing to file.
    async fn file_creator_conversation(
        ctx: &PlatformCommandContext,
        thread: Option<ConversationRecord>,
        group: &CoachingGroup,
    ) -> Result<(), AppError> {
        let Some(thread) = thread else {
            return Ok(());
        };
        let chat = &ctx.ctx.repos().chat;
        let user_id = ctx.user_id.to_string();
        let group_id = group.id.to_string();

        let in_app = ctx.sender_id.is_none();
        if in_app && thread.group_id.is_none() {
            let messages = chat
                .get_messages(&thread.id, &user_id, ctx.conversation_tenant_id)
                .await?;
            let has_coaching_turn = messages.iter().any(|message| {
                matches!(message.role.as_str(), "user" | "assistant") && !message.is_command_turn()
            });
            if !has_coaching_turn {
                chat.set_conversation_group_id(
                    &thread.id,
                    Some(&group_id),
                    ctx.conversation_tenant_id,
                )
                .await?;
                if thread.coach_id.is_none() {
                    chat.set_conversation_coach_id(
                        &thread.id,
                        Some(&group.coach_id),
                        ctx.conversation_tenant_id,
                    )
                    .await?;
                }
                chat.update_conversation_title(
                    &thread.id,
                    &user_id,
                    ctx.conversation_tenant_id,
                    &group.name,
                )
                .await?;
                return Ok(());
            }
        }

        chat.create_conversation(
            &user_id,
            ctx.tenant_id,
            &group.name,
            &thread.model,
            Some(&group.coach_id),
            Some(&group_id),
        )
        .await?;
        Ok(())
    }
}

#[async_trait]
impl CommandHandler for GroupCreateHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();

        // The whole rest of the line is the name, so multi-word names arrive intact.
        let name = ctx.args.join(" ");
        let name = name.trim();
        if name.is_empty() {
            return Ok(CommandResponse::text(reg.render(
                KEY_GROUP_CREATE_USAGE,
                locale,
                &[],
            )));
        }

        let thread = typed_in_thread(ctx).await?;
        let Some(coach) = Self::resolve_group_coach(ctx, thread.as_ref()).await? else {
            return Ok(CommandResponse::text(reg.render(
                KEY_GROUP_CREATE_NO_COACH,
                locale,
                &[],
            )));
        };

        let repos = ctx.ctx.repos();
        if let Err(e) = check_create_group_permission(
            repos.tenants.as_ref(),
            ctx.user_id,
            ctx.tenant_id,
            Self::creation_policy(ctx),
        )
        .await
        {
            if e.code == ErrorCode::PermissionDenied {
                return Ok(CommandResponse::text(reg.render(
                    KEY_GROUP_CREATE_FORBIDDEN,
                    locale,
                    &[],
                )));
            }
            return Err(e);
        }

        // The tenant plan's per-group member cap, resolved as the REST route
        // resolves it; a cap of zero is a plan without group coaching, and is
        // answered here in the caller's locale rather than as the service's
        // PermissionDenied error.
        let plan = repos.tenants.get_by_id(ctx.tenant_id).await?.plan;
        let tier_cap = tier_strategy_for(&plan).max_members_per_group();
        if tier_cap == 0 {
            return Ok(CommandResponse::text(reg.render(
                KEY_GROUP_CREATE_UNAVAILABLE,
                locale,
                &[],
            )));
        }
        let tier_cap = i32::try_from(tier_cap).unwrap_or(i32::MAX);

        let request = CreateGroupRequest {
            name: name.to_owned(),
            description: None,
            coach_id: coach.id.to_string(),
            max_members: None,
        };
        // `group.created` is emitted by the service, once for every surface.
        let group = ctx
            .ctx
            .group_service()
            .create_group(&request, ctx.user_id, ctx.tenant_id, tier_cap)
            .await?;

        Self::file_creator_conversation(ctx, thread, &group).await?;

        info!(
            user_id = %ctx.user_id,
            group_id = %group.id,
            coach_id = %coach.id,
            channel = %ctx.channel_type,
            "Coaching group created via /group create"
        );

        Ok(CommandResponse::card(
            group.name.clone(),
            reg.render(KEY_GROUP_CREATED, locale, &[&group.name, &coach.title]),
            vec![CommandAction {
                label: reg.render(KEY_GROUP_INVITE_LABEL, locale, &[]),
                action_type: "postback".to_owned(),
                value: "/group invite".to_owned(),
            }],
        ))
    }
}

/// Handler for `/group join <invite-code>`.
///
/// Mirrors `POST /api/groups/join`: a member-kind invite adds the caller as
/// an athlete through `GroupService::join_group`, a coach-kind invite
/// attaches them as the group's human coach through
/// `GroupService::redeem_coach_invite`, and both run under the group's own
/// tenant since membership is cross-tenant by design. Every way a code can
/// be unusable — unknown, expired, deactivated, used up, or a coach invite
/// the caller is not eligible for — gets one fixed reply that never echoes
/// the code back.
pub struct GroupJoinHandler;

impl GroupJoinHandler {
    /// The fixed refusal for an unusable code.
    fn invalid_code(ctx: &PlatformCommandContext) -> CommandResponse {
        CommandResponse::text(ctx.ctx.messaging_strings_registry().render(
            KEY_GROUP_JOIN_INVALID_CODE,
            ctx.locale.as_str(),
            &[],
        ))
    }

    /// Whether a service refusal means the code itself is unusable — an
    /// invalid-input or not-found answer from the invite checks — as opposed
    /// to a failure the caller should see as an error.
    fn is_code_refusal(error: &AppError) -> bool {
        matches!(
            error.code,
            ErrorCode::InvalidInput | ErrorCode::ResourceNotFound
        )
    }

    /// The member's own group-scoped conversation, named after the group, so
    /// the group appears in their conversation list the moment they join. It
    /// is filed under the member's own tenant, exactly as the apps file one
    /// through `POST /api/chat/conversations` with a `group_id`, and takes
    /// its model from the thread the command was typed in. A dispatch with
    /// no conversation has no model to inherit; the membership stands and
    /// the app creates the conversation on first open.
    async fn file_member_conversation(
        ctx: &PlatformCommandContext,
        group: &CoachingGroup,
    ) -> Result<(), AppError> {
        let Some(thread) = typed_in_thread(ctx).await? else {
            return Ok(());
        };
        ctx.ctx
            .repos()
            .chat
            .create_conversation(
                &ctx.user_id.to_string(),
                ctx.tenant_id,
                &group.name,
                &thread.model,
                Some(&group.coach_id),
                Some(&group.id.to_string()),
            )
            .await?;
        Ok(())
    }

    async fn join_as_member(
        ctx: &PlatformCommandContext,
        code: &str,
        group: &CoachingGroup,
        group_tenant: TenantId,
    ) -> Result<CommandResponse, AppError> {
        let reg = ctx.ctx.messaging_strings_registry();
        let locale = ctx.locale.as_str();
        let repos = ctx.ctx.repos();
        let group_id = group.id.to_string();

        // The two refusals worth naming the group for — both are the
        // caller's own situation, not the code's.
        if repos
            .groups
            .get_member(&group_id, ctx.user_id)
            .await?
            .is_some()
        {
            return Ok(CommandResponse::text(reg.render(
                KEY_GROUP_JOIN_ALREADY_MEMBER,
                locale,
                &[&group.name],
            )));
        }
        if repos.groups.count_members(&group_id).await? >= i64::from(group.max_members) {
            return Ok(CommandResponse::text(reg.render(
                KEY_GROUP_JOIN_FULL,
                locale,
                &[&group.name],
            )));
        }

        // `group.joined` is emitted by the service, once for every surface.
        match ctx
            .ctx
            .group_service()
            .join_group(code, ctx.user_id, group_tenant)
            .await
        {
            Ok(_) => {}
            Err(e) if Self::is_code_refusal(&e) => return Ok(Self::invalid_code(ctx)),
            Err(e) => return Err(e),
        }

        Self::file_member_conversation(ctx, group).await?;

        info!(
            user_id = %ctx.user_id,
            group_id = %group.id,
            channel = %ctx.channel_type,
            "Joined coaching group via /group join"
        );

        Ok(CommandResponse::text(reg.render(
            KEY_GROUP_JOINED,
            locale,
            &[&group.name],
        )))
    }

    async fn join_as_coach(
        ctx: &PlatformCommandContext,
        code: &str,
        group_tenant: TenantId,
    ) -> Result<CommandResponse, AppError> {
        // Eligibility as the REST route checks it: a roster-managing coach
        // (or a platform admin) who belongs to the group's tenant — athlete
        // membership is cross-tenant, coach attachment is not.
        let Some(user) = ctx.ctx.repos().users.get_global(ctx.user_id).await? else {
            return Ok(Self::invalid_code(ctx));
        };
        if !(user.manages_roster || user.is_admin) || ctx.tenant_id != group_tenant {
            return Ok(Self::invalid_code(ctx));
        }

        // `group.joined` is emitted by the service when the attach happens.
        match ctx
            .ctx
            .group_service()
            .redeem_coach_invite(code, ctx.user_id, group_tenant)
            .await
        {
            Ok(attached) => {
                info!(
                    user_id = %ctx.user_id,
                    group_id = %attached.id,
                    channel = %ctx.channel_type,
                    "Attached as human coach via /group join"
                );
                Ok(CommandResponse::text(
                    ctx.ctx.messaging_strings_registry().render(
                        KEY_GROUP_JOINED_AS_COACH,
                        ctx.locale.as_str(),
                        &[&attached.name],
                    ),
                ))
            }
            Err(e) if Self::is_code_refusal(&e) => Ok(Self::invalid_code(ctx)),
            Err(e) => Err(e),
        }
    }
}

#[async_trait]
impl CommandHandler for GroupJoinHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let Some(code) = ctx.args.first().map(|a| a.trim()).filter(|a| !a.is_empty()) else {
            return Ok(Self::invalid_code(ctx));
        };

        let repos = ctx.ctx.repos();
        let Some(invite) = repos.groups.get_invite_by_code(code).await? else {
            return Ok(Self::invalid_code(ctx));
        };
        // The invite's tenant is the group's tenant: membership is filed
        // there, not under the caller's home tenant.
        let group_tenant = TenantId::parse_str(&invite.tenant_id)
            .map_err(|e| AppError::internal(format!("Invalid invite tenant: {e}")))?;
        let Some(group) = repos
            .groups
            .get_group(&invite.group_id.to_string(), group_tenant)
            .await?
        else {
            return Ok(Self::invalid_code(ctx));
        };

        match invite.kind {
            GroupInviteKind::Member => Self::join_as_member(ctx, code, &group, group_tenant).await,
            GroupInviteKind::Coach => Self::join_as_coach(ctx, code, group_tenant).await,
        }
    }
}
