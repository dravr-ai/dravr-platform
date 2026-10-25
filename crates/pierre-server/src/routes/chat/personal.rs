// ABOUTME: Handlers for /api/chat/personal — the caller's one personal thread, and the topics Home's "+" opens in it
// ABOUTME: Resolved on the server (carnet#840): Home opens the thread it names, "+" divides it instead of forging another

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_middleware::AuthenticatedUser;
use pierre_services::personal_thread::{
    find_personal_thread, open_topic, resolve_personal_thread, PersonalForge, TopicOpening,
};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::services::messaging_ingress::fresh_title_fallback;

use super::common::{conversation_response, get_tenant_id};
use super::conversations::post_welcome_best_effort;
use super::dto::{OpenTopicRequest, OpenTopicResponse, PersonalThreadResponse};
use super::send_message::client_surface;
use super::turn_response::message_response;

/// The caller's personal thread: the designated one, else their newest
/// in-app thread of their own, adopted now.
///
/// `conversation` is `null` for an athlete who has none yet. Nothing is
/// forged for a look: Home opens on its empty pane, and the first thing the
/// athlete starts there — "+", a starter — opens the thread.
pub(super) async fn get_personal_thread(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let conversation = find_personal_thread(
        &resources.common.repos,
        &auth.user_id.to_string(),
        tenant_id,
    )
    .await?;
    let conversation = match conversation {
        Some(conv) => Some(conversation_response(&resources, conv).await),
        None => None,
    };
    let response = PersonalThreadResponse { conversation };
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Open a new topic in the caller's personal thread — Home's "+".
///
/// A divider row, after which the model reads the thread afresh; the thread
/// is rebound to the agent the request names, when it names one, and that
/// agent's welcome opens the topic unless the athlete is about to speak
/// first. An athlete with no thread yet gets one, opened the way "+" always
/// opened a thread: bound to their agent, with no guided walk. No cap applies.
pub(super) async fn open_personal_topic(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    headers: HeaderMap,
    Json(request): Json<OpenTopicRequest>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let user = auth.user_id.to_string();
    let title_fallback = fresh_title_fallback(&resources, &user).await;
    let thread = resolve_personal_thread(
        &resources.common.repos,
        &user,
        tenant_id,
        PersonalForge {
            title_fallback: &title_fallback,
            guided_flow: false,
            llm_chain: &resources.common.config.llm_chain,
        },
    )
    .await?
    .conversation;

    let topic = open_topic(
        &resources.common.repos,
        &thread,
        &user,
        tenant_id,
        TopicOpening {
            agent_id: request.agent_id.as_deref(),
            // "+" never started a walk; it was a blank thread with its agent.
            guided_flow: false,
        },
    )
    .await?;

    // The thread as it stands now: the topic may have rebound its agent. A
    // thread forged for this topic is welcomed here, once, by that agent.
    let thread = resources
        .common
        .repos
        .chat
        .get_conversation(&thread.id, &user, tenant_id)
        .await?
        .ok_or_else(|| AppError::internal("personal thread unreadable after opening a topic"))?;
    if !request.athlete_speaks_first {
        if let Some(agent_id) = thread.agent_id.as_deref() {
            post_welcome_best_effort(
                &resources,
                &thread,
                agent_id,
                auth.user_id,
                tenant_id,
                client_surface(&headers),
            )
            .await;
        }
    }

    let response = OpenTopicResponse {
        conversation: conversation_response(&resources, thread).await,
        divider: topic.divider.map(message_response),
    };
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

/// Open the caller's personal thread with the welcome of the lead the
/// onboarding team step named (carnet#1099): the first-run Meet your team
/// turn, posted on Skip as on Start so every surface lands on it.
///
/// Forges the thread when there is none yet, bound to the athlete's
/// selection with no guided walk, as "+" opens one. Posts only when the
/// thread answers with `lead` — a thread the athlete had pointed elsewhere
/// keeps its own agent — and the introduction ledger makes it once per
/// topic, so the topic Start opens next finds it already welcomed.
/// Best-effort: the step is the athlete's answer, and a welcome that cannot
/// be written is logged.
pub(crate) async fn welcome_onboarding_lead(
    resources: &ServerContext,
    headers: &HeaderMap,
    user_id: Uuid,
    tenant_id: TenantId,
    lead: &str,
) {
    let user = user_id.to_string();
    let title_fallback = fresh_title_fallback(resources, &user).await;
    let forge = PersonalForge {
        title_fallback: &title_fallback,
        guided_flow: false,
        llm_chain: &resources.common.config.llm_chain,
    };
    let thread = match resolve_personal_thread(&resources.common.repos, &user, tenant_id, forge)
        .await
    {
        Ok(thread) => thread.conversation,
        Err(e) => {
            tracing::warn!(error = %e, "onboarding lead: the personal thread could not be resolved; no welcome");
            return;
        }
    };
    if thread.agent_id.as_deref() == Some(lead) {
        post_welcome_best_effort(
            resources,
            &thread,
            lead,
            user_id,
            tenant_id,
            client_surface(headers),
        )
        .await;
    }
}
