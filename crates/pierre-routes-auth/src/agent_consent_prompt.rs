// ABOUTME: Read and answer the one-time question letting Dravr's own agent read its memory and the athlete's health records
// ABOUTME: GET and POST /api/consents/agent-prompt — an answer gives the ticked sensitive_t3 / sensitive_t2 consents, a dismissal neither

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The agent's tier consent, asked once in Dravr's app (carnet#1544).
//!
//! `GET` reads whether the question is due and the standing answers; `POST`
//! records the athlete's answer: `{"memory": bool, "medical": bool}` gives
//! each consent ticked, `{"dismissed": true}` gives neither. Either way the
//! question is not asked again ([`pierre_services::agent_consent_prompt`]).

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use pierre_core::errors::AppError;
use pierre_runtime_context::require_tenant_in;
use pierre_services::agent_consent_prompt::{
    agent_consent_prompt, answer_agent_consent_prompt, AgentConsentAnswer,
};
use serde::Deserialize;

use crate::AuthRoutesContext;

/// The body of `POST /api/consents/agent-prompt`.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct AgentPromptAnswerBody {
    /// Give `sensitive_t3`: the agent reads its memory and the notes.
    #[serde(default)]
    memory: bool,
    /// Give `sensitive_t2`: the agent reads the health records.
    #[serde(default)]
    medical: bool,
    /// "Not now": give neither, and do not ask again.
    #[serde(default)]
    dismissed: bool,
}

/// Read the one-time question's state.
///
/// `GET /api/consents/agent-prompt` — `due` while the athlete has not
/// answered and a consent it asks is not given; `memory` and `medical` are
/// the standing answers.
///
/// # Errors
/// Unauthenticated; an account with no tenant; or the store's error.
pub(crate) async fn handle_get_agent_prompt(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    let auth = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;
    let tenant_id = require_tenant_in(resources.repos.tenants.as_ref(), &auth)
        .await?
        .as_uuid();
    let state = agent_consent_prompt(&resources.repos, tenant_id, auth.user_id).await?;
    Ok((StatusCode::OK, Json(state)).into_response())
}

/// Answer the one-time question.
///
/// `POST /api/consents/agent-prompt` — the answer is the athlete's own: an
/// admin's support session can neither give their consent nor answer for
/// them (carnet#1634).
///
/// # Errors
/// Unauthenticated; an admin's support session; no active tenant;
/// `dismissed` sent beside a consent to give (`invalid_input`); or the
/// store's error.
pub(crate) async fn handle_answer_agent_prompt(
    State(resources): State<AuthRoutesContext>,
    headers: HeaderMap,
    Json(body): Json<AgentPromptAnswerBody>,
) -> Result<Response, AppError> {
    let auth = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;
    auth.refuse_impersonation(
        resources.repos.admin_audit.as_ref(),
        "answer the athlete's question about their agent's tier consent",
    )
    .await?;
    if body.dismissed && (body.memory || body.medical) {
        return Err(AppError::invalid_input(
            "A dismissal gives no consent; send either dismissed or the consents to give",
        ));
    }
    let tenant_id = require_tenant_in(resources.repos.tenants.as_ref(), &auth)
        .await?
        .as_uuid();
    let answer = if body.dismissed {
        AgentConsentAnswer::Dismiss
    } else {
        AgentConsentAnswer::Allow {
            memory: body.memory,
            medical: body.medical,
        }
    };
    let state =
        answer_agent_consent_prompt(&resources.repos, tenant_id, auth.user_id, answer).await?;
    Ok((StatusCode::OK, Json(state)).into_response())
}
