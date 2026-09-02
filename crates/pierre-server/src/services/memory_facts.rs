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

use pierre_core::errors::AppError;
use pierre_core::models::default_locale;
use pierre_middleware::extract_auth_from_headers;
use pierre_runtime_context::{resolve_tenant, tenant::require, TenantMode};
use pierre_services::memory_facts::{
    fact_kind_from_query, forget_user_fact, list_user_facts, SentenceRenderer, DEFAULT_LIST_LIMIT,
};

use crate::mcp::resources::ServerContext;

/// Query parameters for [`get_facts_handler`].
#[derive(Debug, Deserialize)]
pub struct ListFactsQuery {
    /// Optional coach scope — when set, only facts attached to this
    /// coach are returned.
    pub coach_id: Option<String>,
    /// Optional fact-kind filter (`snake_case` form).
    pub kind: Option<String>,
    /// Maximum facts to return, clamped to `1..=100`. Defaults to 50.
    pub limit: Option<i64>,
}

/// Axum handler for `GET /api/memory/facts`.
///
/// Returns the user's stored facts. Filters by coach and/or kind when
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
    let tenant_id = require(resolve_tenant(&resources, &auth, TenantMode::Required).await?)?;
    let data = resources.data();
    let limit = params.limit.unwrap_or(DEFAULT_LIST_LIMIT);
    let kind = fact_kind_from_query(params.kind.as_deref());
    // The sentence is rendered here, once, in the athlete's stored locale —
    // the same way every REST read resolves it.
    let locale = resources
        .common
        .repos
        .users
        .get_global(auth.user_id)
        .await
        .ok()
        .flatten()
        .map_or_else(default_locale, |user| user.locale);

    let response = list_user_facts(
        &data.repos().coach_repos(),
        SentenceRenderer::new(&resources.mcp.messaging_strings_registry, &locale),
        tenant_id,
        &auth.user_id.to_string(),
        params.coach_id.as_deref(),
        kind,
        limit,
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
    let tenant_id = require(resolve_tenant(&resources, &auth, TenantMode::Required).await?)?;
    let data = resources.data();

    let response = forget_user_fact(
        &data.repos().coach_repos(),
        &fact_id,
        tenant_id,
        &auth.user_id.to_string(),
    )
    .await?;

    Ok((StatusCode::OK, Json(response)).into_response())
}
