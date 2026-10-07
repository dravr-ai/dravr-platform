// ABOUTME: Axum handler for the chat verdicts endpoint — delegates to pierre_services::chat_verdicts
// ABOUTME: Resolves the tenant from the authenticated session then defers to the pure service
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Chat verdict handler.
//!
//! Thin axum wrapper around [`pierre_services::chat_verdicts::list_for_conversation`].
//! Lives in pierre-server because the axum + `ServerContext` glue is
//! server-local; the underlying repository logic and wire shapes are in
//! `pierre-services::chat_verdicts`.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use pierre_core::errors::AppError;
use pierre_middleware::extract_auth_from_headers;
use pierre_runtime_context::{resolve_tenant, tenant::require, TenantMode};
use pierre_services::chat_verdicts::list_for_conversation;
use pierre_tool_runtime::derived_content;

use crate::mcp::resources::ServerContext;

/// Axum handler for `GET /api/chat/conversations/:id/verdicts`.
///
/// Authenticates the caller, resolves their active tenant via the
/// canonical `resolve_tenant` helper (no user-id fallback, membership
/// verified for `active_tenant_id` claims), and delegates to
/// [`pierre_services::chat_verdicts::list_for_conversation`]. Routed
/// from `routes::chat` so the chat route module stays under the 1750-line
/// route-thinness threshold.
///
/// # Errors
///
/// Returns the same errors as
/// [`pierre_services::chat_verdicts::list_for_conversation`] plus
/// authentication failures from the middleware extractor, plus
/// [`AppError::auth_invalid`] when the user has no tenants.
pub async fn get_verdicts_handler(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path(conversation_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = require(resolve_tenant(&resources, &auth, TenantMode::Required).await?)?;
    // A thread opened from a first-party-only activity is about it
    // throughout; within any other, a verdict on a reply derived from such
    // data is withheld from an external caller (carnet#769).
    derived_content::refuse_withheld_thread(
        resources.as_ref(),
        &tenant_id,
        auth.user_id,
        &conversation_id,
    )
    .await?;
    let response = list_for_conversation(
        &resources.data().repos().agent_repos(),
        &resources.mcp.evidence_registry,
        &conversation_id,
        &auth.user_id.to_string(),
        tenant_id,
    )
    .await?;
    Ok((StatusCode::OK, Json(response)).into_response())
}
