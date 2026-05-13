// ABOUTME: Session resolution for linked channel users (lookup, self-heal, group binding)
// ABOUTME: Plus the unlinked-user prompt that mints a link code + login URL

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::env;

use chrono::{Duration, Utc};
use pierre_core::models::messaging::{
    ChannelType, MessageContent, OutgoingMessage, LINK_CODE_TTL_MINUTES,
};
use pierre_core::models::TenantId;
use pierre_database::backends::{CreateLinkStateParams, CreateSessionParams, MessagingRepository};
use pierre_database::repositories::ChatRepository;
use pierre_messaging::turn::ConversationTurnId as CanotTurnId;
use serde_json::Value;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::contremaitre::messaging_strings::{
    format_template, DEFAULT_LOCALE, KEY_LINK_FALLBACK_PROMPT, KEY_LINK_INITIAL_PROMPT,
};
use crate::errors::AppError;
use crate::mcp::resources::ServerContext;
use crate::routes::messaging::linking::generate_link_code;
use crate::services::analytics::{analytics, hash_id};
use crate::services::chat_orchestration;
use crate::services::messaging_group_bind::resolve_or_create_channel_group;

use super::linking::hydrate_analytics_consent;
use super::ResolvedSession;

/// Bundle of inbound-chat metadata threaded through the session-resolution
/// chain. Carries the platform-side chat id and (when the transport
/// exposes one) the human-readable title so the auto-bound
/// `coaching_groups` row can use the real Telegram / Discord channel name
/// instead of a synthetic `{channel} group {chat_id}` placeholder.
#[derive(Clone, Copy)]
pub(super) struct ChannelChatRef<'a> {
    pub chat_id: Option<&'a str>,
    pub chat_title: Option<&'a str>,
}

/// Resolve the conversation id a session should use for this turn.
///
/// Returns the stored `pierre_conversation_id` when it points to a
/// row that the linked user can read; otherwise forges a fresh
/// conversation, repoints the session at it, and returns the new id.
/// The unreachable cases collapse into one branch: NULL column (FK
/// `ON DELETE SET NULL` fired), row deleted out from under the FK,
/// row owned by a previous linker (post-rebind), or row owned by a
/// different tenant. Without this self-heal, every message in such a
/// session fails the ownership check in
/// `chat_pipeline::stages::persistence` and the user gets the generic
/// "Conversation not found" error reply.
async fn resolve_session_conversation(
    resources: &ServerContext,
    db: &dyn MessagingRepository,
    session: &Value,
    session_id: &str,
    user_id: &str,
    tenant_id: TenantId,
    channel_type: &str,
) -> Result<String, AppError> {
    if let Some(id) = session["pierre_conversation_id"].as_str() {
        if resources
            .repos
            .chat
            .get_conversation(id, user_id, tenant_id)
            .await?
            .is_some()
        {
            return Ok(id.to_owned());
        }
    }
    forge_fresh_session_conversation(resources, db, session_id, user_id, tenant_id, channel_type)
        .await
}

/// Create a fresh `chat_conversation` and repoint the session at it.
///
/// Called from `resolve_session_conversation` whenever the session's
/// `pierre_conversation_id` cannot be reused for the current linked
/// user (NULL column, deleted row, rebind to a different user, or
/// cross-tenant move). Returns the new conversation id.
async fn forge_fresh_session_conversation(
    resources: &ServerContext,
    db: &dyn MessagingRepository,
    session_id: &str,
    user_id: &str,
    tenant_id: TenantId,
    channel_type: &str,
) -> Result<String, AppError> {
    warn!(
        session_id = %session_id,
        user_id = %user_id,
        channel_type = %channel_type,
        "Session pierre_conversation_id missing or unreachable; self-healing with a fresh conversation"
    );
    let title = format!("Messaging: {channel_type}");
    // Bind the user's default coach so verdicts/grades/myth-busting attribute
    // correctly. When the user has no default, leave it null and the
    // downstream attribution panels will simply skip the row.
    let coach_id = resolve_default_coach_id(resources, user_id).await;
    let conversation = chat_orchestration::create_conversation(
        resources.repos.chat.as_ref(),
        user_id,
        tenant_id,
        &title,
        None,
        coach_id.as_deref(),
        None,
    )
    .await?;
    if let Some(coach_id_str) = coach_id.as_deref() {
        record_coach_usage(resources, coach_id_str, user_id, tenant_id).await;
    }
    let new_id = conversation.conversation.id.clone();
    db.set_session_conversation(session_id, &new_id).await?;
    Ok(new_id)
}

/// Best-effort `coach_assignments.use_count++` for messaging-channel
/// conversations. Logs and swallows errors so transient DB issues don't
/// break the user-visible turn.
async fn record_coach_usage(
    resources: &ServerContext,
    coach_id: &str,
    user_id_str: &str,
    tenant_id: TenantId,
) {
    let Ok(user_id) = Uuid::parse_str(user_id_str) else {
        return;
    };
    match resources
        .coaches_manager()
        .record_usage(coach_id, user_id, tenant_id)
        .await
    {
        Ok(true) => {}
        Ok(false) => {
            warn!(
                coach_id,
                %tenant_id,
                "skipping coach usage bump from messaging path — coach not visible to caller's tenant"
            );
        }
        Err(e) => {
            warn!(coach_id, error = %e, "failed to record coach usage from messaging path");
        }
    }
}

/// Resolve the user's default coach for messaging-channel conversations so
/// claim verdicts, coach grades, and myth-busting can attribute the
/// generated content to a coach. Returns `None` when the user has no
/// default coach or the lookup fails — callers must not panic on the
/// missing attribution.
async fn resolve_default_coach_id(resources: &ServerContext, user_id_str: &str) -> Option<String> {
    let parsed = Uuid::parse_str(user_id_str).ok()?;
    let user = resources.repos.users.get_global(parsed).await.ok()??;
    user.default_coach_id
}

/// Resolve a messaging session for a linked channel user.
///
/// Looks up the channel link to find the Pierre user, then looks up or creates
/// a session. Returns `None` if the sender has no channel link (unlinked user).
///
/// `is_direct_message == false` triggers the channel-group binding pass: the
/// chat is mapped to (or auto-creates) a `coaching_groups` row, and the
/// resulting `group_id` is attached to the conversation so the prompt-assembly
/// stage injects group context (member roster, peer training data subject to
/// per-member consent).
pub(super) async fn resolve_linked_session(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel_type: &str,
    sender_id: &str,
    chat_ref: ChannelChatRef<'_>,
    is_direct_message: bool,
) -> Result<Option<ResolvedSession>, AppError> {
    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();

    // Channel link is the source of truth for "is this sender currently bound
    // to a Pierre user". `logout_channel_sender` deletes only the link and
    // retains messaging_sessions/messages for support and audit, so an
    // orphaned session can outlive its link. Checking the link first prevents
    // post-logout messages from leaking into the previously linked user's
    // chat history.
    let channel_link = db
        .get_channel_link(tenant_id, channel_type, sender_id)
        .await?;
    let Some(link) = channel_link else {
        return Ok(None);
    };
    let linked_user_id = link["user_id"]
        .as_str()
        .ok_or_else(|| AppError::internal("Channel link missing user_id"))?
        .to_owned();

    // Existing session, scoped to this specific chat. A user with a
    // Telegram DM AND a Telegram group chat gets two sessions — one per
    // (tenant, channel, user, chat) — because of migration
    // 20260505000001_messaging_sessions_per_chat.
    if let Some(session) = db
        .get_session_by_channel_identity(tenant_id, channel_type, sender_id, chat_ref.chat_id)
        .await?
    {
        return resume_existing_session(
            resources,
            session,
            &linked_user_id,
            tenant_id,
            channel_type,
            chat_ref,
            is_direct_message,
        )
        .await
        .map(Some);
    }

    // No existing session — open a fresh one for the linked user.
    open_new_session(
        resources,
        &linked_user_id,
        tenant_id,
        channel_type,
        sender_id,
        chat_ref,
        is_direct_message,
    )
    .await
    .map(Some)
}

/// Resume the per-chat session row returned by
/// `get_session_by_channel_identity`. Self-heals a missing conversation,
/// retrofits the group binding for non-DM chats, and touches the session
/// before returning.
async fn resume_existing_session(
    resources: &ServerContext,
    session: Value,
    linked_user_id: &str,
    tenant_id: TenantId,
    channel_type: &str,
    chat_ref: ChannelChatRef<'_>,
    is_direct_message: bool,
) -> Result<ResolvedSession, AppError> {
    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();
    let session_id = session["id"]
        .as_str()
        .ok_or_else(|| AppError::internal("Session missing id field"))?
        .to_owned();
    // Routing follows the channel link, not the session row. session.user_id
    // records the original linker (logout retains the session for audit), so
    // a different Pierre user re-linking the same channel sender would
    // otherwise route into the previous user's chat history.
    let session_user_id = session["user_id"].as_str().unwrap_or("");
    if session_user_id != linked_user_id {
        warn!(
            session_id = %session_id,
            session_user_id = %session_user_id,
            linked_user_id = %linked_user_id,
            "Channel link rebound — routing this turn to the currently linked user"
        );
    }
    let user_id = linked_user_id.to_owned();

    // Self-heal: forge a fresh conversation when the stored
    // pierre_conversation_id can't be reused for this turn (NULL, row
    // deleted, owned by a different user post-rebind, etc.). See
    // [`resolve_session_conversation`] for the full case analysis.
    let conversation = resolve_session_conversation(
        resources,
        db,
        &session,
        &session_id,
        &user_id,
        tenant_id,
        channel_type,
    )
    .await?;

    // Retrofit group_id on conversations that predate the channel binding.
    if !is_direct_message {
        if let Some(chat_id) = chat_ref.chat_id {
            ensure_conversation_group_binding(
                resources,
                tenant_id,
                channel_type,
                chat_id,
                chat_ref.chat_title,
                &user_id,
                &conversation,
            )
            .await;
        }
    }

    if let Err(e) = db.touch_session(&session_id).await {
        error!(error = %e, session_id = %session_id, "Failed to touch session");
    }

    hydrate_analytics_consent(resources, &user_id).await;

    Ok(ResolvedSession {
        session_id,
        conversation,
        user_id,
    })
}

/// Open a brand-new session for a linked user. The caller
/// (`resolve_linked_session`) has already verified the channel link exists
/// and passes the linked `user_id` in. For non-DM chats, resolves the
/// channel-bound `coaching_group` and attaches its id to the new
/// conversation so prompt assembly injects group context.
async fn open_new_session(
    resources: &ServerContext,
    linked_user_id: &str,
    tenant_id: TenantId,
    channel_type: &str,
    sender_id: &str,
    chat_ref: ChannelChatRef<'_>,
    is_direct_message: bool,
) -> Result<ResolvedSession, AppError> {
    let db: &dyn MessagingRepository = resources.repos.messaging.as_ref();
    let user_id = linked_user_id.to_owned();

    let group_id_opt = resolve_group_for_new_session(
        resources,
        tenant_id,
        channel_type,
        chat_ref,
        is_direct_message,
        &user_id,
    )
    .await;

    let title = format!("Messaging: {channel_type}");
    let coach_id = resolve_default_coach_id(resources, &user_id).await;
    let conversation = chat_orchestration::create_conversation(
        resources.repos.chat.as_ref(),
        &user_id,
        tenant_id,
        &title,
        None,
        coach_id.as_deref(),
        group_id_opt.as_deref(),
    )
    .await?;
    if let Some(coach_id_str) = coach_id.as_deref() {
        record_coach_usage(resources, coach_id_str, &user_id, tenant_id).await;
    }

    let conversation_id = conversation.conversation.id.clone();
    let session_id = Uuid::new_v4().to_string();

    let session_params = CreateSessionParams {
        id: &session_id,
        user_id: &user_id,
        tenant_id,
        channel_type,
        channel_user_id: sender_id,
        channel_conversation_id: chat_ref.chat_id,
        pierre_conversation_id: Some(&conversation_id),
    };
    db.create_session(&session_params).await?;

    info!(
        session_id = %session_id,
        conversation_id = %conversation_id,
        channel_type = %channel_type,
        sender_id = %sender_id,
        user_id = %user_id,
        group_id = ?group_id_opt,
        "Created messaging session for linked user"
    );

    hydrate_analytics_consent(resources, &user_id).await;

    analytics().track_session_started(
        channel_type,
        &hash_id(&tenant_id.to_string()),
        &hash_id(&user_id),
        true,
    );

    Ok(ResolvedSession {
        session_id,
        conversation: conversation_id,
        user_id,
    })
}

/// Pick the `coaching_groups.id` to attach to a new messaging conversation,
/// or `None` for DMs / when no coach is available to bootstrap the group.
///
/// `chat_ref.chat_title` carries the human-readable group name from the
/// inbound payload (Telegram `chat.title`, Discord `channel.name`). When
/// `None`, the binding helper falls back to the synthetic
/// `{channel} group {id}` label so existing groups still resolve.
async fn resolve_group_for_new_session(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel_type: &str,
    chat_ref: ChannelChatRef<'_>,
    is_direct_message: bool,
    user_id: &str,
) -> Option<String> {
    if is_direct_message {
        return None;
    }
    let chat_id = chat_ref.chat_id?;
    let chat_title_hint = chat_ref
        .chat_title
        .map_or_else(|| format!("{channel_type} group {chat_id}"), str::to_owned);
    match resolve_or_create_channel_group(
        resources,
        tenant_id,
        channel_type,
        chat_id,
        user_id,
        &chat_title_hint,
    )
    .await
    {
        Ok(opt) => opt,
        Err(e) => {
            error!(
                error = %e,
                channel_type,
                chat_id,
                "Failed to resolve/create channel-bound coaching_group; conversation will be ungrouped"
            );
            None
        }
    }
}

/// Retrofit `chat_conversations.group_id` for an existing conversation
/// that pre-dates the channel/group binding.
///
/// Only writes if the conversation currently has a NULL `group_id`;
/// failures are logged (not surfaced) so a binding hiccup doesn't block
/// message handling.
async fn ensure_conversation_group_binding(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel_type: &str,
    channel_chat_id: &str,
    chat_title: Option<&str>,
    user_id: &str,
    conversation_id: &str,
) {
    let chat_repo = resources.repos.chat.as_ref();
    let Ok(already_bound) =
        conversation_already_bound(chat_repo, conversation_id, user_id, tenant_id).await
    else {
        return;
    };
    if already_bound {
        return;
    }

    let chat_title_hint = chat_title.map_or_else(
        || format!("{channel_type} group {channel_chat_id}"),
        str::to_owned,
    );
    let Some(new_group_id) = resolve_group_for_retrofit(
        resources,
        tenant_id,
        channel_type,
        channel_chat_id,
        user_id,
        &chat_title_hint,
    )
    .await
    else {
        return;
    };

    if let Err(e) = chat_repo
        .set_conversation_group_id(conversation_id, Some(&new_group_id), tenant_id)
        .await
    {
        error!(
            error = %e,
            conversation_id,
            group_id = %new_group_id,
            "Failed to set group_id on conversation during retrofit"
        );
    } else {
        info!(
            conversation_id,
            group_id = %new_group_id,
            "Retrofit group_id onto pre-binding conversation"
        );
    }
}

/// Look up the conversation and report whether it already has a `group_id`.
/// Returns `Err(())` on lookup failure (callers swallow the error so the
/// ongoing message turn proceeds).
async fn conversation_already_bound(
    chat_repo: &dyn ChatRepository,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
) -> Result<bool, ()> {
    match chat_repo
        .get_conversation(conversation_id, user_id, tenant_id)
        .await
    {
        Ok(Some(c)) => Ok(c.group_id.is_some()),
        Ok(None) => {
            warn!(
                conversation_id,
                user_id, "Conversation not found while attempting group retrofit"
            );
            Err(())
        }
        Err(e) => {
            error!(
                error = %e,
                conversation_id,
                "Failed to load conversation for group retrofit"
            );
            Err(())
        }
    }
}

/// Resolve (or auto-create on first sender) the channel-bound
/// `coaching_groups.id` for a retrofit pass. Logs and returns `None` on
/// failure or when no coach is available to bootstrap the group.
async fn resolve_group_for_retrofit(
    resources: &ServerContext,
    tenant_id: TenantId,
    channel_type: &str,
    channel_chat_id: &str,
    user_id: &str,
    chat_title_hint: &str,
) -> Option<String> {
    match resolve_or_create_channel_group(
        resources,
        tenant_id,
        channel_type,
        channel_chat_id,
        user_id,
        chat_title_hint,
    )
    .await
    {
        Ok(opt) => opt,
        Err(e) => {
            error!(
                error = %e,
                channel_type,
                channel_chat_id,
                "Failed to resolve/create coaching_group during retrofit"
            );
            None
        }
    }
}

/// Create a link state and return a prompt message with a clickable login URL
///
/// Generates a 32-character cryptographic code with a 10-minute TTL, stores it
/// in the database, and constructs a message with a clickable URL for the user.
pub(super) async fn create_link_and_prompt(
    resources: &ServerContext,
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel_type: ChannelType,
    sender_id: &str,
    sender_name: Option<&str>,
) -> OutgoingMessage {
    let code = generate_link_code();
    let expires_at = Utc::now() + Duration::minutes(LINK_CODE_TTL_MINUTES);
    let id = Uuid::new_v4().to_string();
    let channel_str = channel_type.to_string();

    let params = CreateLinkStateParams {
        id: &id,
        tenant_id,
        user_id: None,
        channel_type: &channel_str,
        code: &code,
        method: "channel_initiated",
        channel_user_id: Some(sender_id),
        sender_name,
        expires_at: &expires_at.to_rfc3339(),
    };

    if let Err(e) = db.create_link_state(&params).await {
        error!(error = %e, "Failed to create link state for unlinked user");
        let body = resources
            .messaging_strings_registry
            .get(KEY_LINK_FALLBACK_PROMPT, DEFAULT_LOCALE);
        return OutgoingMessage {
            channel_type,
            recipient_id: sender_id.to_owned(),
            content: MessageContent::Text { body },
            turn_id: CanotTurnId::new(),
            reply_to: None,
            thread_id: None,
        };
    }

    let base_url = env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:8081".to_owned());
    let link_url = format!("{base_url}/messaging/link/{code}");

    let template = resources
        .messaging_strings_registry
        .get(KEY_LINK_INITIAL_PROMPT, DEFAULT_LOCALE);
    let body = format_template(&template, &[&link_url]);

    OutgoingMessage {
        channel_type,
        recipient_id: sender_id.to_owned(),
        content: MessageContent::Text { body },
        turn_id: CanotTurnId::new(),
        reply_to: None,
        thread_id: None,
    }
}
