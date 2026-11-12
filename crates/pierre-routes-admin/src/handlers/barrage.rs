// ABOUTME: Admin route adopting the activity cache into the activity store: every live cached row kept in barrage
// ABOUTME: Super-admin only, audited when it writes; `pierre-cli barrage adopt-cache` reaches it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `POST /admin/barrage/adopt-cache` (carnet#1482).
//!
//! Run once per environment after the deploy that moved every provider read
//! into the activity store: the rows cached before it are kept in the store
//! too, each at its own sync stamp ([`pierre_services::cache_adoption`]).
//! The answer names every `(tenant, user, key)` and whether the store now
//! holds as many records as the cache holds rows.

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use serde::Deserialize;
use serde_json::json;
use tracing::info;

use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_services::cache_adoption::{AdoptionOutcome, AdoptionScope, CacheAdoption};
use uuid::Uuid;

use super::api_keys::json_response;
use super::strava_pool::deny_if_not_super_admin;
use super::types::AdminResponse;
use crate::audit::AuditDetail;
use crate::context::AdminApiContext;

/// Request to adopt the cache.
#[derive(Debug, Default, Deserialize)]
pub struct AdoptCacheRequest {
    /// Only this tenant's cache; every tenant's when absent.
    #[serde(default)]
    pub tenant_id: Option<String>,
    /// Only this user's cache.
    #[serde(default)]
    pub user_id: Option<Uuid>,
    /// Only this cache key.
    #[serde(default)]
    pub provider: Option<String>,
    /// Say what would be adopted and write nothing.
    #[serde(default)]
    pub dry_run: bool,
}

/// `POST /admin/barrage/adopt-cache` — keep the cached rows of live sources
/// in the activity store.
///
/// The counts are checked per user and key. `tenant_id`, `user_id` and
/// `provider` narrow it: the CLI reads the pairs with a dry run, then adopts
/// them one per request.
///
/// Super-admin only (403 otherwise, before anything is read). Answers 200
/// with the report whatever it found: a difference is in `data.failures`,
/// for the operator to act on.
///
/// # Errors
///
/// Returns an invalid-input error for a malformed tenant id, a not-found
/// error for an unknown tenant, a resource-unavailable error when no live
/// store is wired, and the cache's or the store's error when a read or a
/// write fails.
pub async fn handle_adopt_cache(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Json(request): Json<AdoptCacheRequest>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_if_not_super_admin(&admin_token) {
        return Ok(denied);
    }
    let Some(store) = context.live_activities.as_deref() else {
        return Err(AppError::resource_unavailable(
            "The activity store is not wired on this server; the cache cannot be adopted",
        ));
    };
    let tenant = request
        .tenant_id
        .as_deref()
        .map(|id| {
            TenantId::parse_str(id)
                .map_err(|e| AppError::invalid_input(format!("tenant id {id}: {e}")))
        })
        .transpose()?;

    let scope = AdoptionScope {
        tenant,
        user_id: request.user_id,
        provider: request.provider.clone(),
    };
    let report = CacheAdoption::new(&context.repos, store)
        .run(&scope, request.dry_run)
        .await?;
    let failures = report.failures();
    let adopted = report
        .rows
        .iter()
        .filter(|row| row.outcome == AdoptionOutcome::Matched)
        .count();
    info!(
        dry_run = report.dry_run,
        pairs = report.rows.len(),
        adopted,
        failures,
        token_id = %admin_token.token_id,
        "Activity cache adoption"
    );
    let message = if report.dry_run {
        format!(
            "Dry run: {} (user, key) pairs read, nothing written",
            report.rows.len()
        )
    } else {
        format!(
            "{adopted} (user, key) pairs adopted, {failures} to act on, of {}",
            report.rows.len()
        )
    };
    let data = json!({
        "dry_run": report.dry_run,
        "failures": failures,
        "rows": report.rows,
    });
    let response = json_response(
        AdminResponse {
            success: true,
            message,
            data: Some(data),
        },
        StatusCode::OK,
    )
    .into_response();
    if report.dry_run {
        return Ok(response);
    }
    let audit = AuditDetail::new(
        "barrage.adopt_cache",
        format!(
            "Adopted the activity cache: {adopted} (user, key) pairs, {failures} differing, of {}",
            report.rows.len()
        ),
    )
    .target(json!({
        "tenant_id": request.tenant_id,
        "user_id": request.user_id,
        "provider": request.provider,
        "pairs": report.rows.len(),
        "adopted": adopted,
        "failures": failures,
    }));
    Ok(audit.attach(response))
}
