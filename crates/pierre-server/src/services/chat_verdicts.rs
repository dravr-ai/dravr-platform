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
    let response = list_for_conversation(
        &resources.data().repos().coach_repos(),
        &conversation_id,
        &auth.user_id.to_string(),
        tenant_id,
    )
    .await?;
    Ok((StatusCode::OK, Json(response)).into_response())
}
