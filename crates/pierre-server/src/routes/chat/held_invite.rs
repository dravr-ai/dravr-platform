// ABOUTME: Handler for POST /api/chat/groups/join — the caller joins the group whose invite the server held for them
// ABOUTME: /group join's checks and follow-ups, the member's room filed with the join's reply in it (carnet#1648)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Redeeming a held group invite.
//!
//! A `/groups/join/<code>` link opened before the account could join is held
//! on the server ([`pierre_services::group_invite_hold`]). The app asks this
//! route once the account is active: it joins the invite's group the way
//! `/group join` does — the same checks, the coach-link proposal, the coach's
//! notice, or a coach invite's attach ([`GroupJoinHandler::join_held_invite`])
//! — then files the caller's conversation in the group's room and writes the
//! join's reply into it, so an athlete reads what joining shares with the
//! coach where the coach will talk to them.
//!
//! The hold is dropped once the join is decided either way. A failure before
//! that keeps it, and the app asks again on its next load: an interrupted
//! request, a reload or a closed tab never loses the invite.

use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use pierre_auth::auth::AuthResult;
use pierre_commands::group_membership::{GroupJoinHandler, HeldInviteJoin};
use pierre_commands::{ConversationRotation, PlatformCommandContext};
use pierre_contremaitre::messaging_strings::KEY_GROUP_JOIN_INVALID_CODE;
use pierre_core::errors::AppError;
use pierre_core::models::groups::{CoachingGroup, GroupInviteKind};
use pierre_core::models::{AddMessageParams, TenantId};
use pierre_core::transport::TransportPolicy;
use pierre_middleware::AuthenticatedUser;
use pierre_runtime_context::CommandCtx;
use pierre_services::group_invite_hold;
use pierre_services::locale::resolve_user_locale;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde::Serialize;
use tracing::{info, warn};

use crate::mcp::resources::ServerContext;

use super::common::get_tenant_id;
use super::conversations::{create_for, RoomSlot};
use super::dto::CreateConversationRequest;

/// Channel label recorded on the command context this route builds: an app
/// redeeming a held invite, not a chat turn.
const HELD_INVITE_CHANNEL: &str = "held_group_invite";

/// What `POST /api/chat/groups/join` decided.
#[derive(Debug, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub(super) enum JoinHeldInviteResponse {
    /// The caller holds no invite.
    None,
    /// The caller joined the group now, as an athlete (`member`) or as its
    /// human coach (`coach`). `conversation_id` is their room, holding the
    /// join's reply; `None` when it could not be filed (the app files it on
    /// first open, as it does for any room).
    Joined {
        /// The group joined
        group_id: String,
        /// Its name
        group_name: String,
        /// Which side of the group the caller joined
        kind: GroupInviteKind,
        /// The caller's conversation in the group's room
        conversation_id: Option<String>,
    },
    /// The caller was already in the group on that side; the hold is dropped.
    AlreadyMember {
        /// The group
        group_id: String,
        /// Its name
        group_name: String,
        /// Which side of the group the caller is on
        kind: GroupInviteKind,
    },
    /// The invite could not be used; `message` says why, in the caller's
    /// language. The hold is dropped.
    Refused {
        /// The refusal `/group join` would have given
        message: String,
    },
}

/// `POST /api/chat/groups/join` — join the group of the invite the server
/// holds for the caller, and drop the hold.
///
/// The app asks on every load of an active account, so the hold is read
/// before anything else: an account that holds none is answered without a
/// tenant lookup, whose refusal would sign it out.
///
/// A support session is refused before anything is read (carnet#1634):
/// joining grants the group's coach sharing, and a coach invite attaches the
/// coach who reads every member, so neither may outlive the admin's session.
///
/// # Errors
///
/// Authentication and tenant failures, a support session's `403`, and a
/// repository or service failure before the join is decided, which keeps the
/// hold for the next attempt.
pub(super) async fn join_held_invite(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let repos = &resources.common.repos;
    auth.refuse_impersonation(repos.admin_audit.as_ref(), "join a held group invite")
        .await?;
    let Some(hold) = repos.signup_codes.held_group_invite(auth.user_id).await? else {
        return Ok(answer(JoinHeldInviteResponse::None));
    };
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let ctx = command_context(&resources, &auth, tenant_id).await;

    let outcome = if group_invite_hold::lapsed(&hold, Utc::now()) {
        info!(user_id = %auth.user_id, invite_id = %hold.invite_id, "A held group invite lapsed before its account joined");
        HeldInviteJoin::Refused {
            reply: resources.mcp.messaging_strings_registry.render(
                KEY_GROUP_JOIN_INVALID_CODE,
                &ctx.locale,
                &[],
            ),
        }
    } else {
        GroupJoinHandler::join_held_invite(&ctx, hold.invite_id).await?
    };

    // Decided: the hold has done its work. A hold the account took on another
    // invite meanwhile is kept, for the next call.
    repos
        .signup_codes
        .release_group_invite_hold(auth.user_id, hold.invite_id)
        .await?;

    let body = match outcome {
        HeldInviteJoin::Joined { group, kind, reply } => {
            let conversation_id =
                file_room_with_reply(&resources, &auth, tenant_id, &headers, &group, &reply).await;
            JoinHeldInviteResponse::Joined {
                group_id: group.id.to_string(),
                group_name: group.name,
                kind,
                conversation_id,
            }
        }
        HeldInviteJoin::AlreadyMember { group, kind } => JoinHeldInviteResponse::AlreadyMember {
            group_id: group.id.to_string(),
            group_name: group.name,
            kind,
        },
        HeldInviteJoin::Refused { reply } => JoinHeldInviteResponse::Refused { message: reply },
    };
    Ok(answer(body))
}

/// The command context `/group join` runs with, for a REST caller with no
/// thread: the caller's own tenant and language, no channel sender.
async fn command_context(
    resources: &Arc<ServerContext>,
    auth: &AuthResult,
    tenant_id: TenantId,
) -> PlatformCommandContext {
    let locale = resolve_user_locale(resources.common.repos.users.as_ref(), auth.user_id).await;
    let command_ctx: Arc<dyn CommandCtx> = Arc::<ServerContext>::clone(resources);
    let tool_runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(resources);
    PlatformCommandContext {
        user_id: auth.user_id,
        tenant_id,
        channel_type: HELD_INVITE_CHANNEL.to_owned(),
        args: Vec::new(),
        raw_text: String::new(),
        ctx: command_ctx,
        locale,
        is_direct_message: true,
        ambient_group_fallback: false,
        conversation_id: None,
        conversation_tenant_id: tenant_id,
        sender_id: None,
        rotation: ConversationRotation::default(),
        tool_runtime,
    }
}

/// File the caller's conversation in `group`'s room, the way the app opens a
/// room, and write the join's reply into it as a platform-authored assistant
/// row, unread. Returns the conversation's id. Best-effort: the membership
/// stands either way, and the app files the room on first open.
async fn file_room_with_reply(
    resources: &Arc<ServerContext>,
    auth: &AuthResult,
    tenant_id: TenantId,
    headers: &HeaderMap,
    group: &CoachingGroup,
    reply: &str,
) -> Option<String> {
    let request = CreateConversationRequest {
        title: None,
        model: None,
        agent_id: None,
        group_id: Some(group.id.to_string()),
        athlete_speaks_first: false,
    };
    let room = match create_for(
        resources,
        auth,
        tenant_id,
        headers,
        &request,
        RoomSlot::CallersRoom,
    )
    .await
    {
        Ok((room, _)) => room,
        Err(e) => {
            warn!(user_id = %auth.user_id, group_id = %group.id, error = %e, "Joined from a held invite, but the member's room could not be filed");
            return None;
        }
    };
    post_reply(resources, auth, tenant_id, &room.id, reply).await;
    info!(user_id = %auth.user_id, group_id = %group.id, conversation_id = %room.id, "Filed the member's room after a held invite's join");
    Some(room.id)
}

/// Write the join's reply into the member's room as a platform-authored
/// assistant row, unread. A failure is logged: the membership and the room
/// stand without it.
async fn post_reply(
    resources: &Arc<ServerContext>,
    auth: &AuthResult,
    tenant_id: TenantId,
    room_id: &str,
    reply: &str,
) {
    let user_id = auth.user_id.to_string();
    let written = resources
        .common
        .repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id,
            conversation_id: room_id,
            user_id: &user_id,
            role: "assistant",
            content: reply,
            // Platform-authored, like the command's reply: no model answered
            // and no tokens were spent.
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
            // The group's and the coach's names only, built from no provider
            // data.
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: None,
            about_user_id: None,
            turn_attempted_write: None,
        })
        .await;
    if let Err(e) = written {
        warn!(user_id = %auth.user_id, conversation_id = %room_id, error = %e, "Joined from a held invite, but the join's reply could not be written to the room");
    }
}

/// 200 with `body`.
fn answer(body: JoinHeldInviteResponse) -> Response {
    (StatusCode::OK, Json(body)).into_response()
}
