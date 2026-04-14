// ABOUTME: User-facing memory fact service — list and forget what the coach remembers
// ABOUTME: Wraps HarnessMemoryRepository with user-scoped wire shapes for the GDPR Forget UX
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! User-facing memory facts service.
//!
//! Exposes a tightly-scoped read+delete surface for [`pierre_memory::UserFact`]
//! rows so the user-facing memory panel can show what the coach remembers
//! and let the user GDPR-forget any individual fact. Tenant ownership is
//! enforced by the caller (the route handler resolves the active tenant
//! from the authenticated session before invoking these helpers).

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde::{Deserialize, Serialize};

use pierre_core::models::TenantId;
use pierre_memory::FactKind;

use crate::errors::AppError;
use crate::mcp::resources::ServerResources;
use crate::middleware::extract_auth_from_headers;

/// Default page size when the client omits `limit`. Bounded to 100 by
/// [`MAX_LIST_LIMIT`] so a misconfigured client cannot drag the database.
const DEFAULT_LIST_LIMIT: i64 = 50;
/// Maximum number of facts returned in a single response.
const MAX_LIST_LIMIT: i64 = 100;

/// Wire shape for a single stored user fact. Mirrors the domain
/// [`pierre_memory::UserFact`] but flattens enums to stable string keys
/// so the `TypeScript` client can render them without an extra mapping.
#[derive(Debug, Serialize, Deserialize)]
pub struct UserFactRow {
    /// Stable identifier — the key the Forget action uses.
    pub id: String,
    /// Coach the fact is scoped to, or `null` for cross-coach facts.
    pub coach_id: Option<String>,
    /// One of `preference | physiology | injury | goal | schedule | equipment | other`.
    pub kind: String,
    /// Subject phrase, typically `"you"`.
    pub subject: String,
    /// Verb phrase (`prefers`, `has`, `runs`, etc.).
    pub predicate: String,
    /// Object phrase — the fact value the user can review.
    pub object: String,
    /// Confidence in `[0.0, 1.0]` from the extractor.
    pub confidence: f32,
    /// Source message id for "jump to source" UI affordances.
    pub source_msg_id: Option<String>,
    /// RFC3339 timestamp of the most recent update to this fact.
    pub updated_at: String,
}

/// Response envelope for `GET /api/memory/facts`.
#[derive(Debug, Serialize, Deserialize)]
pub struct UserFactListResponse {
    /// Facts ordered most-recently-updated first.
    pub facts: Vec<UserFactRow>,
    /// Total number of facts returned in this response.
    pub total: usize,
}

/// Response envelope for `DELETE /api/memory/facts/:fact_id`.
#[derive(Debug, Serialize, Deserialize)]
pub struct ForgetFactResponse {
    /// `true` when a row was removed, `false` when no matching fact was
    /// found (still a success — the desired post-condition holds).
    pub deleted: bool,
}

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

/// Resolve the user's active tenant the same way `routes::chat` does,
/// without depending on the chat module so this handler can live entirely
/// inside the services layer.
async fn resolve_tenant_id(resources: &ServerResources, user_id: uuid::Uuid) -> TenantId {
    resources
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .ok()
        .and_then(|tenants| tenants.first().map(|t| t.id))
        .unwrap_or_else(|| TenantId::from(user_id))
}

fn fact_kind_from_query(raw: Option<&str>) -> Option<FactKind> {
    raw.map(FactKind::parse_lenient)
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
///   [`pierre_database::repositories::HarnessMemoryRepository::list_user_facts`].
pub async fn get_facts_handler(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Query(params): Query<ListFactsQuery>,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&resources, auth.user_id).await;
    let limit = params
        .limit
        .unwrap_or(DEFAULT_LIST_LIMIT)
        .clamp(1, MAX_LIST_LIMIT);
    let kind = fact_kind_from_query(params.kind.as_deref());

    let facts = resources
        .repos
        .memory
        .list_user_facts(
            tenant_id,
            &auth.user_id.to_string(),
            params.coach_id.as_deref(),
            kind,
            limit,
        )
        .await?;

    let rows: Vec<UserFactRow> = facts
        .into_iter()
        .map(|f| UserFactRow {
            id: f.id,
            coach_id: f.coach_id,
            kind: f.kind.as_str().to_owned(),
            subject: f.subject,
            predicate: f.predicate,
            object: f.object,
            confidence: f.confidence,
            source_msg_id: f.source_msg_id,
            updated_at: f.updated_at.to_rfc3339(),
        })
        .collect();

    let total = rows.len();
    Ok((
        StatusCode::OK,
        Json(UserFactListResponse { facts: rows, total }),
    )
        .into_response())
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
///   [`pierre_database::repositories::HarnessMemoryRepository::delete_user_fact`].
pub async fn forget_fact_handler(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    Path(fact_id): Path<String>,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let tenant_id = resolve_tenant_id(&resources, auth.user_id).await;

    let deleted = resources
        .repos
        .memory
        .delete_user_fact(&fact_id, tenant_id, &auth.user_id.to_string())
        .await?;

    Ok((StatusCode::OK, Json(ForgetFactResponse { deleted })).into_response())
}
