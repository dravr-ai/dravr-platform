// ABOUTME: Handler for POST /api/chat/athletes/{athlete_user_id}/thread — a group's coach opens their thread about one athlete
// ABOUTME: Runs the by-id coach gates, then opens or reopens the one thread per coach per athlete, bound to the group's agent

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1070. A group's human coach works on one athlete's plan in a thread
//! of their own bound to that athlete. Group info opens it here: the first
//! time it is created, every later time the same thread comes back, active
//! again. The plan and calendar tools called in it act for that athlete, and
//! re-check the same gates this route checks on every call.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use pierre_core::errors::AppError;
use pierre_core::models::ConversationSubject;
use pierre_database::repositories::NewSubjectConversation;
use pierre_middleware::AuthenticatedUser;
use pierre_services::coached_athlete::{coached_athlete_access, CoachedAthleteRefusal};
use pierre_services::conversation_forge::resolve_conversation_model;
use tracing::info;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

use super::common::get_tenant_id;
use super::dto::{ConversationResponse, OpenSubjectThreadRequest};

/// Open the caller's thread about `athlete_user_id`, as the human coach of
/// the group the body names.
///
/// The gates are the ones every act in the thread runs again
/// ([`coached_athlete_access`]): the caller is the active group's coach, the
/// athlete a live member who still shares their data with them and has one
/// home tenant. A refusal carries `details.reason`: 403 `not_group_coach`,
/// 404 `not_a_member`, 403 `coach_sharing_revoked`, 409 `several_tenants` or
/// `no_tenant`.
///
/// The thread is bound to the group's agent, so the plan is built with the
/// package the athlete's own plan is. It is the coach's own: titled with the
/// athlete's name, never a room, never their personal thread, outside their
/// conversation cap. No welcome opens it — the group's agent greets athletes,
/// and this thread's reader is their coach.
pub(super) async fn open_subject_thread(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Path(athlete_user_id): Path<String>,
    Json(request): Json<OpenSubjectThreadRequest>,
) -> Result<Response, AppError> {
    let auth = auth.into_inner();
    let tenant_id = get_tenant_id(&auth, &resources).await?;
    let athlete = Uuid::parse_str(athlete_user_id.trim())
        .map_err(|_| AppError::invalid_input("athlete_user_id must be a UUID"))?;
    let repos = &resources.common.repos;

    let access = coached_athlete_access(repos, auth.user_id, athlete, request.group_id)
        .await?
        .map_err(CoachedAthleteRefusal::into_error)?;

    let model = resolve_conversation_model(None, &resources.common.config.llm_chain)?;
    let coach = auth.user_id.to_string();
    let opened = repos
        .chat
        .open_subject_conversation(&NewSubjectConversation {
            user_id: &coach,
            tenant_id,
            title: &access.display_name,
            model: &model,
            agent_id: &access.group.agent_id,
            subject: ConversationSubject {
                athlete_user_id: access.athlete_user_id,
                group_id: access.group.id,
            },
        })
        .await?;
    info!(
        coach_user_id = %auth.user_id,
        athlete_user_id = %access.athlete_user_id,
        group_id = %access.group.id,
        conversation_id = %opened.conversation.id,
        created = opened.created,
        "coach opened their thread about a coached athlete"
    );

    let response = ConversationResponse::new(opened.conversation, Some(access.display_name));
    Ok((StatusCode::OK, Json(response)).into_response())
}
