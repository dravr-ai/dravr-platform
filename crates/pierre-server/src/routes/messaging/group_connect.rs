// ABOUTME: REST for connecting a web coaching group to a Telegram group chat (carnet#1683): status, connect code, disconnect
// ABOUTME: The owner or coach mints a single-use code the bot redeems in the chat; disconnecting tells the chat and keeps it ungrouped
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `/api/groups/{group_id}/chat-connection`.
//!
//! Lives with the messaging routes rather than the group routes because all
//! three answers need the messaging side: the bot that serves the group's
//! tenant (its channel configuration), the Telegram add-to-group link built
//! from that bot's username, and the poster that tells the chat it was
//! disconnected.
//!
//! - `GET` — whether the group is connected to a chat, and whether the caller
//!   may change that. Any member, and the group's coach, may read it.
//! - `POST` `{ "channel": "telegram" }` — a connect code with the link and
//!   the typed fallback. Owner or coach only; refused while the group already
//!   has a chat (one chat app per group).
//! - `DELETE` — disconnect. Owner or coach only. The chat is told, and stays
//!   ungrouped: its next message creates no group.

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use chrono::{DateTime, Utc};
use pierre_contremaitre::messaging_strings::KEY_GROUP_CHAT_DISCONNECTED;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::groups::{CoachingGroup, ReleasedGroupChat};
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::TenantId;
use pierre_database::backends::MessagingRepository;
use pierre_middleware::extract_auth_from_headers;
use pierre_services::channel_adapters::ChannelAdapterFactory;
use pierre_services::locale::user_locale;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{info, warn};
use uuid::Uuid;

use pierre_auth::auth::AuthResult;

use super::linking::{qr_svg_for, resolve_tenant_id, telegram_add_to_group_url};
use crate::mcp::resources::ServerContext;

/// What the group panel reads.
#[derive(Debug, Serialize)]
pub(crate) struct ChatConnectionResponse {
    /// Whether the group is bound to a chat at all.
    connected: bool,
    /// The chat's channel (`telegram`), when connected.
    channel: Option<String>,
    /// The chat's title as it was when connected from the web app, when the
    /// platform sent one.
    chat_title: Option<String>,
    /// When the chat was connected from the web app.
    connected_at: Option<DateTime<Utc>>,
    /// Whether the caller may connect or disconnect: the owner or the coach.
    can_manage: bool,
}

/// `POST` body.
#[derive(Debug, Deserialize)]
pub(crate) struct CreateChatConnectionBody {
    /// The chat app to connect: `telegram` today.
    channel: String,
}

/// A connect code and the two ways to redeem it. `Debug` never prints the
/// code, nor the link and command that carry it.
#[derive(Serialize)]
pub(crate) struct ChatConnectCodeResponse {
    channel: String,
    code: String,
    /// Opens Telegram's "add to group" picker; Telegram then delivers
    /// `/start <code>` in the chat picked.
    deep_link_url: String,
    /// What to type in a chat the bot already sits in.
    command: String,
    /// The link as a QR code, for a person on a desktop whose chat app is on
    /// their phone.
    qr_svg: Option<String>,
    expires_at: DateTime<Utc>,
}

impl fmt::Debug for ChatConnectCodeResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ChatConnectCodeResponse")
            .field("channel", &self.channel)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

/// The group named by the path, when the caller may read it: a member or its
/// coach. Anyone else gets the same not-found a missing group gets.
async fn readable_group(
    resources: &Arc<ServerContext>,
    auth: &AuthResult,
    group_id: &str,
) -> Result<CoachingGroup, AppError> {
    let not_found = || AppError::not_found("Group not found");
    Uuid::parse_str(group_id).map_err(|_| AppError::invalid_input("Invalid group_id format"))?;
    let caller = auth.user_id;
    // Groups are cross-tenant: the id alone finds the row, and membership
    // below decides who may read it.
    let caller_tenant = resolve_tenant_id(auth, resources).await?;
    let groups = &resources.common.repos.groups;
    let group = groups
        .get_group(group_id, caller_tenant)
        .await?
        .filter(|group| group.is_active)
        .ok_or_else(not_found)?;
    let tenant = TenantId::parse_str(&group.tenant_id)
        .map_err(|_| AppError::internal("Group carries no valid tenant_id"))?;
    if !groups
        .admits_member_or_coach(group_id, caller, tenant)
        .await?
    {
        return Err(not_found());
    }
    Ok(group)
}

/// `GET /api/groups/{group_id}/chat-connection`
pub(crate) async fn get_chat_connection(
    State(resources): State<Arc<ServerContext>>,
    Path(group_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let group = readable_group(&resources, &auth, &group_id).await?;
    let connection = group.chat_connection.as_ref();
    Ok(Json(ChatConnectionResponse {
        connected: group.channel_type.is_some(),
        channel: group.channel_type.clone(),
        chat_title: connection.and_then(|c| c.chat_title.clone()),
        connected_at: connection.map(|c| c.connected_at),
        can_manage: group.can_manage_chat(auth.user_id),
    })
    .into_response())
}

/// The bot that serves `group`'s tenant on `channel`, and the tenant that
/// owns its configuration — where the bot will look a code up.
///
/// The group's tenant, not the caller's: an owner and a coach can belong to
/// different tenants, and the chat must be served by one bot whoever minted
/// the code. A disabled configuration is refused as absent, since the webhook
/// only routes to active ones.
async fn serving_bot(
    resources: &ServerContext,
    group: &CoachingGroup,
    channel: ChannelType,
) -> Result<(Value, TenantId), AppError> {
    let group_tenant = TenantId::parse_str(&group.tenant_id)
        .map_err(|_| AppError::internal("Group carries no valid tenant_id"))?;
    let db: &dyn MessagingRepository = resources.common.repos.messaging.as_ref();
    let config = db
        .resolve_channel_config(group_tenant, &channel.to_string())
        .await?
        .filter(|config| {
            config
                .get("is_active")
                .and_then(Value::as_bool)
                .unwrap_or(true)
        })
        .ok_or_else(|| {
            AppError::invalid_input(format!(
                "No {channel} bot is available for this group, so there is nothing to connect to."
            ))
        })?;
    let bot_tenant = config
        .get("tenant_id")
        .and_then(Value::as_str)
        .and_then(|raw| TenantId::parse_str(raw).ok())
        .ok_or_else(|| AppError::internal("Channel config carries no valid tenant_id"))?;
    Ok((config, bot_tenant))
}

/// `POST /api/groups/{group_id}/chat-connection`
pub(crate) async fn create_chat_connection(
    State(resources): State<Arc<ServerContext>>,
    Extension(adapters): Extension<Arc<dyn ChannelAdapterFactory>>,
    Path(group_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<CreateChatConnectionBody>,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    // A connected chat keeps acting for the group after a support session ends.
    auth.refuse_impersonation(
        resources.common.repos.admin_audit.as_ref(),
        "connect a group to a chat",
    )
    .await?;
    let channel = ChannelType::from_str(&body.channel)
        .map_err(|_| AppError::invalid_input(format!("Unknown chat app: {}", body.channel)))?;
    if channel != ChannelType::Telegram {
        return Err(AppError::invalid_input(format!(
            "Connecting a group to {channel} is not available yet"
        )));
    }
    let group = readable_group(&resources, &auth, &group_id).await?;
    // Both asked before the bot is resolved: a member who may not connect
    // hears that rather than whether a bot exists, and a group that already
    // has a chat is told so whatever bot would serve a new one (one chat app
    // per group). `mint_chat_connect_code` asks both again.
    if !group.can_manage_chat(auth.user_id) {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Only the group's owner or coach can connect it to a chat",
        ));
    }
    if group.channel_type.is_some() {
        return Err(AppError::new(
            ErrorCode::ResourceAlreadyExists,
            "This group is already connected to a chat; disconnect it first",
        ));
    }
    let (config, bot_tenant) = serving_bot(&resources, &group, channel).await?;
    let code = resources
        .group_service()
        .mint_chat_connect_code(&group, auth.user_id, &channel.to_string(), bot_tenant)
        .await?;
    let deep_link_url = telegram_add_to_group_url(&code.code, &config, adapters.as_ref()).await?;
    let response = ChatConnectCodeResponse {
        channel: channel.to_string(),
        command: format!("/group link {}", code.code),
        qr_svg: qr_svg_for(&deep_link_url),
        deep_link_url,
        code: code.code,
        expires_at: code.expires_at,
    };
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

/// `DELETE /api/groups/{group_id}/chat-connection`
pub(crate) async fn delete_chat_connection(
    State(resources): State<Arc<ServerContext>>,
    Path(group_id): Path<String>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let group = readable_group(&resources, &auth, &group_id).await?;
    let Some(released) = resources
        .group_service()
        .disconnect_chat(&group, auth.user_id)
        .await?
    else {
        return Err(AppError::not_found("This group is not connected to a chat"));
    };
    tell_the_chat(&resources, &group, &released, auth.user_id).await;
    Ok(StatusCode::NO_CONTENT.into_response())
}

/// Tell a chat it is no longer the group's room, in the language of the
/// person who disconnected it. Best-effort: a chat the bot was removed from
/// cannot be told, and that never undoes the disconnect.
async fn tell_the_chat(
    resources: &ServerContext,
    group: &CoachingGroup,
    released: &ReleasedGroupChat,
    caller: Uuid,
) {
    let Ok(channel) = ChannelType::from_str(&released.channel_type) else {
        return;
    };
    let caller_row = resources
        .common
        .repos
        .users
        .get_global(caller)
        .await
        .ok()
        .flatten();
    let locale = user_locale(caller_row.as_ref());
    let text = resources.mcp.messaging_strings_registry.render(
        KEY_GROUP_CHAT_DISCONNECTED,
        &locale,
        &[&group.name],
    );
    let parts = resources
        .mcp
        .group_chat_poster
        .post(
            released.channel_tenant_id,
            channel,
            &released.channel_chat_id,
            &text,
        )
        .await;
    if parts == 0 {
        warn!(group_id = %group.id, channel = %channel, "chat disconnect: the chat could not be told");
    } else {
        info!(group_id = %group.id, channel = %channel, "chat disconnect: told the chat");
    }
}
