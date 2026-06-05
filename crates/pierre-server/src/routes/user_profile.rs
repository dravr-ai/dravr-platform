// ABOUTME: User profile self-service routes — timezone setter + quota snapshot
// ABOUTME: Web + mobile call /api/users/me/timezone after login to populate users.timezone

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `/api/users/me/*` — user-profile self-service endpoints.
//!
//! These let an authenticated client update fields on its own user
//! row that aren't suitable for the OAuth2 ROPC token endpoint
//! (timezone, profile preferences, etc.). The chat prompt-assembly
//! stage reads `users.timezone` to resolve `{{CURRENT_DATE}}` to the
//! user's local calendar day, so the value must be writeable without
//! re-authenticating.

use std::sync::Arc;

use axum::extract::State;
use axum::routing::{get, put};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::mcp::resources::ServerContext;
use pierre_core::errors::{AppError, AppResult};
use pierre_middleware::extractors::AuthenticatedUser;
use pierre_runtime_context::{default_admin_config, AdminConfigLookup};

/// Mount the user-profile routes on the supplied router.
pub fn routes() -> Router<Arc<ServerContext>> {
    Router::new()
        .route("/api/users/me/timezone", put(set_timezone))
        .route("/api/users/me/quota", get(get_my_quota))
}

/// Request body for `PUT /api/users/me/timezone`.
#[derive(Debug, Deserialize)]
pub struct SetTimezoneRequest {
    /// IANA timezone database name, e.g. `"America/Toronto"`.
    pub timezone: String,
}

/// Response body for `PUT /api/users/me/timezone`.
#[derive(Debug, Serialize)]
pub struct SetTimezoneResponse {
    /// The timezone now stored on the user row.
    pub timezone: String,
}

/// `PUT /api/users/me/timezone` — persist the authenticated user's
/// IANA timezone so the chat prompt can render `{{CURRENT_DATE}}` in
/// the user's local calendar.
///
/// The body shape is `{"timezone": "<IANA name>"}`. The server
/// validates that the string parses as a known timezone before
/// touching the database — junk values (empty string, made-up names)
/// return `400` so the client knows to re-read the user's `Intl`
/// settings instead of silently storing garbage that the prompt
/// resolver will reject at read time.
async fn set_timezone(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
    Json(req): Json<SetTimezoneRequest>,
) -> AppResult<Json<SetTimezoneResponse>> {
    let tz = req.timezone.trim();
    if tz.is_empty() {
        return Err(AppError::invalid_input("timezone must not be empty"));
    }
    if tz.parse::<chrono_tz::Tz>().is_err() {
        return Err(AppError::invalid_input(format!(
            "{tz} is not a valid IANA timezone database name",
        )));
    }

    resources
        .common
        .repos
        .users
        .set_timezone(auth.user_id, tz)
        .await
        .map_err(|e| {
            warn!(user_id = %auth.user_id, error = %e, "Failed to persist user timezone");
            AppError::database(format!("set_timezone failed: {e}"))
        })?;

    Ok(Json(SetTimezoneResponse {
        timezone: tz.to_owned(),
    }))
}

// ----- /api/users/me/quota -----------------------------------------------

/// Per-counter snapshot exposed to the frontend so the user can see
/// where they sit against each tier-keyed cap.
#[derive(Debug, Serialize)]
pub struct QuotaCounter {
    /// Counter key — `daily_messages`, `daily_tokens`, `weekly_tokens`,
    /// `daily_tool_calls`, etc.
    pub counter_type: String,
    /// Current value within the active period.
    pub current: i64,
    /// Tier-keyed soft limit (post admin-config override).
    pub limit: i64,
    /// `true` when consumption has crossed the warning threshold.
    pub warning: bool,
    /// `true` when consumption is in the burst zone (between soft and
    /// hard limit).
    pub burst_zone: bool,
    /// ISO-8601 timestamp when the counter resets.
    pub resets_at: String,
}

/// `GET /api/users/me/quota` response.
#[derive(Debug, Serialize)]
pub struct MyQuotaResponse {
    /// Currently effective tier — drives the default caps below.
    pub tier: String,
    /// Snapshots for every quota the chat write path checks.
    pub counters: Vec<QuotaCounter>,
}

/// `GET /api/users/me/quota` — return the authenticated user's
/// current quota usage and limits across the tier-keyed counters
/// the chat write path enforces.
async fn get_my_quota(
    State(resources): State<Arc<ServerContext>>,
    auth: AuthenticatedUser,
) -> AppResult<Json<MyQuotaResponse>> {
    // Fall back to compile-time tier defaults when admin config is
    // unavailable so the user can still see their quota; enforcement
    // reads the same defaults.
    let admin_config: &dyn AdminConfigLookup = match resources.coach.admin_config.as_deref() {
        Some(c) => c,
        None => default_admin_config(),
    };

    let user = resources
        .common
        .repos
        .users
        .get_global(auth.user_id)
        .await?
        .ok_or_else(|| AppError::not_found("authenticated user row missing"))?;

    let tenant_id_str = if let Some(t) = auth.active_tenant_id {
        t.to_string()
    } else {
        let rows = resources
            .common
            .repos
            .tenants
            .list_for_user(auth.user_id)
            .await
            .map_err(|e| AppError::internal(format!("tenant lookup failed: {e}")))?;
        rows.first()
            .map(|t| t.id.to_string())
            .ok_or_else(|| AppError::not_found("user has no active tenant"))?
    };
    let user_id_str = auth.user_id.to_string();

    let usage_svc = pierre_services::usage_counter::UsageCounterService::new(
        resources.common.repos.usage_counters.as_ref(),
        admin_config,
    );

    let counters_to_check = [
        "daily_messages",
        "daily_tokens",
        "weekly_tokens",
        "daily_tool_calls",
        "daily_conversations",
        "active_coaches",
    ];
    let mut counters = Vec::with_capacity(counters_to_check.len());
    for counter_type in counters_to_check {
        let check = usage_svc
            .check_limit_for_tier(&tenant_id_str, &user_id_str, counter_type, &user.tier)
            .await?;
        counters.push(QuotaCounter {
            counter_type: counter_type.to_owned(),
            current: check.current,
            limit: check.limit,
            warning: check.warning,
            burst_zone: check.burst_zone,
            resets_at: check.resets_at,
        });
    }

    Ok(Json(MyQuotaResponse {
        tier: user.tier.as_str().to_owned(),
        counters,
    }))
}
