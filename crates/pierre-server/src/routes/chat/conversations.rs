// ABOUTME: CRUD handlers for /api/chat/conversations — create/list/get/update/delete
// ABOUTME: No LLM dispatch; pure chat_pipeline persistence + chat repo calls
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use chrono::Utc;
use pierre_chat_pipeline::agent_welcome::{post_agent_welcome, WelcomeTarget};
use pierre_chat_pipeline::SurfaceId;
use pierre_contremaitre::messaging_strings::KEY_NEW_CONVERSATION_TITLE_PREFIX;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::{ConversationRecord, TenantId};
use pierre_database::repositories::NewConversation;
use pierre_middleware::AuthenticatedUser;
use pierre_providers::ai_scope;
use pierre_runtime_context::AdminConfigLookup;
use pierre_services::agent_selection::{record_agent_selection, AgentSelectionSource};
use pierre_services::agents::{require_bindable_agent, require_room_agent};
use pierre_services::conversation_forge::{
    counterpart_title, create_conversation_slot, dated_title, resolve_conversation_model,
    selected_or_system_agent, SlotQuota,
};
use pierre_services::locale::{resolve_user_locale, user_locale};
use pierre_tool_runtime::derived_content;

use super::common::{conversation_response, get_tenant_id, verify_group_membership};
use super::dto::preview_text;
use super::dto::{
    ConversationListResponse, ConversationSummaryResponse, CreateConversationRequest,
    LastMessageResponse, ListConversationsQuery, UpdateConversationRequest,
};
use super::send_message::client_surface;

/// Best-effort `agent_assignments.use_count++` for REST-created conversations,
/// via the shared selection recorder that also emits `agent.selected`.
/// Logs and swallows errors so a transient DB hiccup doesn't fail the user-
/// visible conversation create. `record_agent_selection` logs the
/// agent-not-visible case itself and emits nothing for it.
async fn record_agent_usage_best_effort(
    resources: &ServerContext,
    agent_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) {
    if let Err(e) = record_agent_selection(
        resources.agents_manager(),
        agent_id,
        user_id,
        tenant_id,
        AgentSelectionSource::ChatConversation,
    )
    .await
    {
        tracing::warn!(agent_id, error = %e, "failed to record coach usage");
    }
}

/// The agent a conversation was created with opens it (carnet#735): its
/// welcome lands before the client first reads the thread. Best-effort — a
/// welcome that cannot be written is logged and the conversation is still
/// created; one for an agent the athlete cannot see writes nothing.
pub(super) async fn post_welcome_best_effort(
    resources: &ServerContext,
    conv: &ConversationRecord,
    agent_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
    surface: SurfaceId,
) {
    let repos = &resources.common.repos;
    let locale = resolve_user_locale(repos.users.as_ref(), user_id).await;
    let user_id = user_id.to_string();
    let target = WelcomeTarget {
        conversation: conv,
        user_id: &user_id,
        conversation_tenant_id: tenant_id,
        agent_id,
        agent_tenant_id: tenant_id,
        locale: &locale,
        surface,
        // Both apps draw a welcome's starters as chips.
        renders_buttons: true,
    };
    if let Err(e) =
        post_agent_welcome(repos, &resources.mcp.messaging_strings_registry, target).await
    {
        tracing::warn!(agent_id, conversation_id = %conv.id, error = %e, "agent welcome not posted on create");
    }
}

/// Create a new conversation.
pub async fn create_conversation(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    headers: HeaderMap,
    Json(request): Json<CreateConversationRequest>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let user_id_str = auth.user_id.to_string();

    // Verify group membership when caller asks to attach a group_id —
    // a user can only create a conversation scoped to a group they belong to,
    // or one they are the human coach of.
    if let Some(gid) = request.group_id.as_deref() {
        verify_group_membership(&resources, gid, auth.user_id, tenant_id).await?;
    }

    // A named agent must be one the caller may bind: readable, coach-facing
    // only for a coach, and never coach-facing in a room, whose turns reach
    // every member.
    if let Some(agent_id) = request.agent_id.as_deref() {
        let (agents, users) = (
            resources.common.repos.agents.as_ref(),
            resources.common.repos.users.as_ref(),
        );
        if request.group_id.is_some() {
            require_room_agent(agents, users, agent_id, auth.user_id, tenant_id).await?;
        } else {
            require_bindable_agent(agents, users, agent_id, auth.user_id, tenant_id).await?;
        }
    }

    // A title the caller typed wins; otherwise the thread is named the way a
    // forged one is — the group, the agent, or the dated stamp — so the row
    // both clients print says who it is with from its first moment.
    let title = match request.title.as_deref().map(str::trim) {
        Some(typed) if !typed.is_empty() => typed.to_owned(),
        _ => {
            let user = resources
                .common
                .repos
                .users
                .get_global(auth.user_id)
                .await
                .ok()
                .flatten();
            let locale = user_locale(user.as_ref());
            let prefix = resources.mcp.messaging_strings_registry.render(
                KEY_NEW_CONVERSATION_TITLE_PREFIX,
                &locale,
                &[],
            );
            let stamp = dated_title(
                &prefix,
                &locale,
                user.as_ref().and_then(|u| u.timezone.as_deref()),
                Utc::now(),
            );
            counterpart_title(
                &resources.common.repos,
                tenant_id,
                &user_id_str,
                request.group_id.as_deref(),
                request.agent_id.as_deref(),
                &stamp,
            )
            .await
        }
    };

    // The agent the thread is bound to. One the caller named wins. A personal
    // thread created without one is bound to the athlete's selected agent,
    // else their own thread's default — the one answer the messaging DM forge
    // binds (carnet#829) — so the thread opens on an agent's welcome and the
    // starters it offers resolve against that agent. A room created without
    // one stays unbound: who answers there is the group's agent. The title
    // above still follows what the caller named, so two threads bound by
    // default keep their dated stamps apart in a list.
    let agent_id = match (request.agent_id.as_deref(), request.group_id.as_deref()) {
        (Some(named), _) => Some(named.to_owned()),
        (None, None) => {
            selected_or_system_agent(&resources.common.repos, tenant_id, auth.user_id).await
        }
        (None, Some(_)) => None,
    };

    // Created under max_active_conversations — the same locked count-and-
    // insert `/reset` runs — so two requests at the cap's edge cannot both
    // get the last slot.
    let model = resolve_conversation_model(request.model.as_deref())?;
    let conv = create_conversation_slot(
        &resources.common.repos,
        SlotQuota::Capped(
            resources
                .agent
                .admin_config
                .as_deref()
                .map(|c| c as &dyn AdminConfigLookup),
        ),
        &NewConversation {
            user_id: &user_id_str,
            tenant_id,
            title: &title,
            model: &model,
            agent_id: agent_id.as_deref(),
            group_id: request.group_id.as_deref(),
        },
    )
    .await?;

    // Bump the per-agent use_count + last_used_at when a conversation is
    // opened with an agent attached. Audit (2026-05-07) showed every agent
    // stuck at "0 uses" because nothing on the chat path called
    // record_usage even though the field is shown in the Coaches UI.
    //
    // The agent then opens the thread with its welcome — unless the athlete
    // is about to speak first, in which case its first reply introduces it.
    // A thread opened from an empty thread's starter is such a thread: the
    // starter is its first line, so no welcome offers the same starters again.
    if let Some(agent_id) = agent_id.as_deref() {
        record_agent_usage_best_effort(&resources, agent_id, auth.user_id, tenant_id).await;
        if !request.athlete_speaks_first {
            post_welcome_best_effort(
                &resources,
                &conv,
                agent_id,
                auth.user_id,
                tenant_id,
                client_surface(&headers),
            )
            .await;
        }
    }

    let response = conversation_response(&resources, conv).await;

    Ok((StatusCode::CREATED, Json(response)).into_response())
}

/// List every conversation the caller takes part in — whatever surface opened
/// it — newest activity first, one page at a time.
///
/// `total` is the caller's real count, so a client knows whether a "load more"
/// is owed; the page bounds are clamped server-side (see
/// [`ListConversationsQuery::bounded`]).
pub async fn list_conversations(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Query(query): Query<ListConversationsQuery>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let (limit, offset) = query.bounded();

    // An external caller is listed no thread opened from a first-party-only
    // activity, and every count and preview it reads is of unstamped rows —
    // in the statement, so the page fills (carnet#769).
    let page = resources
        .common
        .repos
        .chat
        .list_conversations(
            &auth.user_id.to_string(),
            tenant_id,
            limit,
            offset,
            ai_scope::readable_policy(),
        )
        .await?;

    let response = ConversationListResponse {
        conversations: page
            .items
            .into_iter()
            .map(|c| ConversationSummaryResponse {
                id: c.id,
                title: c.title,
                model: c.model,
                message_count: c.message_count,
                total_tokens: c.total_tokens,
                agent_id: c.agent_id,
                agent_handle: c.agent_handle,
                agent_title: c.agent_title,
                group_id: c.group_id,
                group_name: c.group_name,
                channel_type: c.channel_type,
                archived_at: c.archived_at,
                personal: c.personal,
                subject_user_id: c.subject_user_id.map(|id| id.to_string()),
                subject_group_id: c.subject_group_id.map(|id| id.to_string()),
                subject_display_name: c.subject_display_name,
                last_message: c
                    .last_message
                    .filter(|m| ai_scope::admit_derived(m.transport_policy))
                    .map(|m| LastMessageResponse {
                        preview: preview_text(&m.content_head),
                        role: m.role,
                        created_at: m.created_at,
                    }),
                unread_count: c.unread_count,
                created_at: c.created_at,
                updated_at: c.updated_at,
            })
            .collect(),
        total: page.total,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Fetch a single conversation by id (scoped to the caller's tenant).
pub async fn get_conversation(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;

    let conv = resources
        .common
        .repos
        .chat
        .get_conversation(&conversation_id, &auth.user_id.to_string(), tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation not found"))?;
    // A thread opened from a first-party-only activity is about it
    // throughout (carnet#769).
    derived_content::refuse_withheld_thread(
        resources.as_ref(),
        &tenant_id,
        auth.user_id,
        &conversation_id,
    )
    .await?;

    let response = conversation_response(&resources, conv).await;

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Rename a conversation, returning the updated record.
pub async fn update_conversation(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
    Json(request): Json<UpdateConversationRequest>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;

    let updated = resources
        .common
        .repos
        .chat
        .update_conversation_title(
            &conversation_id,
            &auth.user_id.to_string(),
            tenant_id,
            &request.title,
        )
        .await?;

    if !updated {
        return Err(AppError::not_found("Conversation not found"));
    }

    // Fetch and return the updated conversation (proper REST response)
    let conv = resources
        .common
        .repos
        .chat
        .get_conversation(&conversation_id, &auth.user_id.to_string(), tenant_id)
        .await?
        .ok_or_else(|| AppError::internal("Conversation not found after update"))?;

    let response = conversation_response(&resources, conv).await;

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Hard-delete a conversation the caller owns.
///
/// Deletion is the one conversation write reserved to the owner: a
/// participant who was added to the thread is refused with 403 rather than
/// the 404 a stranger gets, because for them the conversation does exist.
pub async fn delete_conversation(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(conversation_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let user_id_str = auth.user_id.to_string();

    let conv = resources
        .common
        .repos
        .chat
        .get_conversation(&conversation_id, &user_id_str, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation not found"))?;
    if conv.user_id != user_id_str {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Only the conversation's owner can delete it",
        ));
    }

    let deleted = resources
        .common
        .repos
        .chat
        .delete_conversation(&conversation_id, &user_id_str, tenant_id)
        .await?;

    if !deleted {
        return Err(AppError::not_found("Conversation not found"));
    }

    Ok((StatusCode::NO_CONTENT, ()).into_response())
}
