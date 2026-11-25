// ABOUTME: Axum handlers for the user-facing memory facts endpoints
// ABOUTME: Thin shims that delegate to pierre_services::memory_facts pure helpers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! User-facing memory facts handlers.
//!
//! Thin axum wrappers around [`pierre_services::memory_facts`]. The wire
//! shapes and repository-backed logic live in `pierre-services`; this
//! module exists only because the axum + `ServerContext` glue is
//! server-local.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use uuid::Uuid;

use dravr_barrage::Stream;
use pierre_auth::auth::AuthResult;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_middleware::extract_auth_from_headers;
use pierre_providers::ai_scope;
use pierre_runtime_context::require_tenant;
use pierre_services::locale::resolve_user_locale;
use pierre_services::memory_facts::{
    approve_user_fact, fact_kind_from_query, fact_status_from_query, forget_user_fact,
    list_user_facts, ApproveFact, FactListQuery, SentenceRenderer, DEFAULT_LIST_LIMIT,
};
use pierre_services::provider_notice::outside_client_reads;

use crate::mcp::resources::ServerContext;

/// Query parameters for [`get_facts_handler`].
#[derive(Debug, Deserialize)]
pub struct ListFactsQuery {
    /// Optional agent scope — when set, only facts attached to this
    /// agent are returned.
    pub agent_id: Option<String>,
    /// Optional fact-kind filter (`snake_case` form).
    pub kind: Option<String>,
    /// Maximum facts to return, clamped to `1..=100`. Defaults to 50.
    pub limit: Option<i64>,
    /// `draft` lists the facts waiting on the athlete; the default lists the
    /// kept ones.
    pub status: Option<String>,
}

/// Axum handler for `GET /api/memory/facts`.
///
/// Returns the user's stored facts. Filters by agent and/or kind when
/// provided. The response always includes an explicit `total` so the
/// frontend can render an empty-state message without a separate request.
///
/// # Errors
///
/// - Authentication failures from the middleware extractor.
/// - Repository errors propagated from
///   [`pierre_services::memory_facts::list_user_facts`].
pub async fn get_facts_handler(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Query(params): Query<ListFactsQuery>,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = require_tenant(&resources, &auth).await?;
    refuse_outside_memory_read(&resources, &auth, tenant_id.as_uuid()).await?;
    let data = resources.data();
    let limit = params.limit.unwrap_or(DEFAULT_LIST_LIMIT);
    let kind = fact_kind_from_query(params.kind.as_deref());
    // The sentence is rendered here, once, in the athlete's stored locale —
    // the same way every REST read resolves it.
    let locale = resolve_user_locale(resources.common.repos.users.as_ref(), auth.user_id).await;

    let response = list_user_facts(
        &data.repos().agent_repos(),
        SentenceRenderer::new(&resources.mcp.messaging_strings_registry, &locale),
        tenant_id,
        &auth.user_id.to_string(),
        FactListQuery {
            agent_id: params.agent_id.as_deref(),
            kind,
            status: fact_status_from_query(params.status.as_deref()),
            limit,
        },
    )
    .await?;

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Axum handler for `DELETE /api/memory/facts/:fact_id`.
///
/// GDPR-grade Forget: removes the named fact when it belongs to the
/// authenticated user. Returns `200 OK` with `deleted: false` if no row
/// was matched (idempotent — the post-condition is "this fact is gone").
///
/// # Errors
///
/// - Authentication failures from the middleware extractor.
/// - Repository errors propagated from
///   [`pierre_services::memory_facts::forget_user_fact`].
pub async fn forget_fact_handler(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path(fact_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = require_tenant(&resources, &auth).await?;
    let data = resources.data();

    let response = forget_user_fact(
        &data.repos().agent_repos(),
        &fact_id,
        tenant_id,
        &auth.user_id.to_string(),
    )
    .await?;

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Axum handler for `POST /api/memory/facts/:fact_id/approve`.
///
/// Keeps one of the athlete's drafts: from the next turn the agent reads it.
/// `409 Conflict` when a newer fact about the same thing was saved after the
/// draft, which then can only be discarded; `404` when the athlete has no
/// fact with that id.
///
/// # Errors
///
/// - Authentication failures from the middleware extractor.
/// - Repository errors propagated from
///   [`pierre_services::memory_facts::approve_user_fact`].
pub async fn approve_fact_handler(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Path(fact_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = require_tenant(&resources, &auth).await?;
    // Approving answers with the fact: a read of the memory like the list.
    refuse_outside_memory_read(&resources, &auth, tenant_id.as_uuid()).await?;
    let data = resources.data();
    let locale = resolve_user_locale(resources.common.repos.users.as_ref(), auth.user_id).await;

    match approve_user_fact(
        &data.repos().agent_repos(),
        SentenceRenderer::new(&resources.mcp.messaging_strings_registry, &locale),
        &fact_id,
        tenant_id,
        &auth.user_id.to_string(),
    )
    .await?
    {
        ApproveFact::Approved(response) => Ok((StatusCode::OK, Json(response)).into_response()),
        // 409: a newer fact about the same thing exists, so the draft is stale.
        ApproveFact::Superseded => Err(AppError::new(
            ErrorCode::ResourceAlreadyExists,
            "A newer fact about this was saved after the draft",
        )),
        ApproveFact::NotFound => Err(AppError::not_found(format!("Fact {fact_id}"))),
    }
}

/// Refuse an outside credential's read of the agent's memory.
///
/// The agent's memory is the athlete's `RestrictedNotes` stream (T3), as the
/// memory tools declare it, and T3 never goes to an outside client,
/// `sensitive_t3` given or not (ChefFamille 2026-11-25, carnet#1544): the one
/// tier rule ([`outside_client_reads`]) answers it.
async fn refuse_outside_memory_read(
    resources: &ServerContext,
    auth: &AuthResult,
    tenant_id: Uuid,
) -> Result<(), AppError> {
    if ai_scope::served_outside(auth.acts_for_outside_client())
        && !outside_client_reads(
            &resources.common.repos,
            tenant_id,
            auth.user_id,
            Stream::RestrictedNotes.tier(),
        )
        .await
    {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "The athlete's agent's memory is never served to outside AI apps. The athlete \
             still sees it in Dravr.",
        ));
    }
    Ok(())
}
