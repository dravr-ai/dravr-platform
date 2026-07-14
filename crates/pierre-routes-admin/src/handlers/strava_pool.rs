// ABOUTME: Super-admin endpoints for the Strava shared-app OAuth credential pool
// ABOUTME: The server KMS-encrypts client_secret at rest; the operator only sends it over TLS, never touching the DEK
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin CRUD over the `strava_oauth_app_pool` table.
//!
//! The pool holds extra platform-owned Strava OAuth apps that grow the
//! athlete-seat cap beside the env `STRAVA_CLIENT_ID` app.
//!
//! These are the step-A endpoints of the authenticated admin CLI (ADR-022):
//! super-admin-gated, and the **server** encrypts the `client_secret` with its
//! KMS DEK (via the repository), so the secret never lives on an operator's
//! machine. Usable today via `curl` with a super-admin token; the gcloud-style
//! `pierre-cli auth login` + `pierre-cli strava-pool` client layers on later.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde_json::{json, Value};
use tracing::info;

use pierre_auth::strava_pool;
use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::errors::{AppError, AppResult};

use super::api_keys::json_response;
use super::types::AdminResponse;
use crate::context::AdminApiContext;

/// Managing platform OAuth-app secrets is a super-admin-only operation. Returns
/// the FORBIDDEN response when the caller isn't super-admin, else `None`.
fn deny_if_not_super_admin(admin_token: &ValidatedAdminToken) -> Option<Response> {
    if admin_token.is_super_admin {
        return None;
    }
    Some(
        json_response(
            AdminResponse {
                success: false,
                message: "Permission denied: super-admin required to manage the Strava OAuth pool"
                    .to_owned(),
                data: None,
            },
            StatusCode::FORBIDDEN,
        )
        .into_response(),
    )
}

/// `POST /admin/strava-pool/apps` — insert or update a pool app.
///
/// Body: `{ client_id, client_secret, seat_cap, label? }`. The server encrypts
/// `client_secret` at rest; it is never returned or logged.
///
/// # Errors
///
/// Returns an error if a required field is missing or invalid, or if the
/// repository upsert (including secret encryption) fails.
pub async fn handle_upsert_strava_pool_app(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Json(request): Json<Value>,
) -> AppResult<Response> {
    if let Some(denied) = deny_if_not_super_admin(&admin_token) {
        return Ok(denied);
    }

    let client_id = request
        .get("client_id")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::invalid_input("client_id is required"))?;
    let client_secret = request
        .get("client_secret")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::invalid_input("client_secret is required"))?;
    let seat_cap = request
        .get("seat_cap")
        .and_then(Value::as_u64)
        .and_then(|c| u32::try_from(c).ok())
        .filter(|&c| c > 0)
        .ok_or_else(|| AppError::invalid_input("seat_cap must be a positive integer"))?;
    let label = request.get("label").and_then(Value::as_str);

    context
        .repos
        .oauth_tokens
        .upsert_strava_pool_app(client_id, client_secret, seat_cap, label)
        .await?;

    info!(
        client_id = client_id,
        seat_cap = seat_cap,
        service = %admin_token.service_name,
        "Strava pool app upserted by super-admin"
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("Strava pool app {client_id} saved"),
            data: Some(json!({ "client_id": client_id, "seat_cap": seat_cap, "label": label })),
        },
        StatusCode::CREATED,
    )
    .into_response())
}

/// `GET /admin/strava-pool/apps` — list all pool apps (secrets excluded).
///
/// Also returns the aggregate seat summary across the env app and the pool.
///
/// # Errors
///
/// Returns an error if listing pool apps or computing the seat summary fails.
pub async fn handle_list_strava_pool_apps(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<Response> {
    if let Some(denied) = deny_if_not_super_admin(&admin_token) {
        return Ok(denied);
    }

    let apps = context
        .repos
        .oauth_tokens
        .list_strava_pool_apps(false)
        .await?;
    let summary = strava_pool::strava_seat_summary(context.repos.oauth_tokens.as_ref()).await?;

    let apps_json: Vec<Value> = apps
        .iter()
        .map(|a| {
            json!({
                "client_id": a.client_id,
                "seat_cap": a.seat_cap,
                "enabled": a.enabled,
                "label": a.label,
                "created_at": a.created_at,
                "updated_at": a.updated_at,
            })
        })
        .collect();

    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("{} Strava pool app(s)", apps.len()),
            data: Some(json!({
                "apps": apps_json,
                "seats": { "total": summary.total, "used": summary.used, "left": summary.left() },
            })),
        },
        StatusCode::OK,
    )
    .into_response())
}

/// `PATCH /admin/strava-pool/apps/{client_id}` — enable or disable a pool app.
///
/// Body: `{ enabled: bool }`. Disabled apps are skipped for new connections.
///
/// # Errors
///
/// Returns an error if the `enabled` field is missing or the repository update fails.
pub async fn handle_set_strava_pool_app_enabled(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(client_id): Path<String>,
    Json(request): Json<Value>,
) -> AppResult<Response> {
    if let Some(denied) = deny_if_not_super_admin(&admin_token) {
        return Ok(denied);
    }

    let enabled = request
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or_else(|| AppError::invalid_input("enabled (boolean) is required"))?;

    context
        .repos
        .oauth_tokens
        .set_strava_pool_app_enabled(&client_id, enabled)
        .await?;

    info!(
        client_id = %client_id,
        enabled = enabled,
        service = %admin_token.service_name,
        "Strava pool app enabled-state changed by super-admin"
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!(
                "Strava pool app {client_id} {}",
                if enabled { "enabled" } else { "disabled" }
            ),
            data: None,
        },
        StatusCode::OK,
    )
    .into_response())
}

/// `DELETE /admin/strava-pool/apps/{client_id}` — remove a pool app.
///
/// Tokens it already issued keep their attribution and will fail to refresh
/// once the secret is gone, so only delete an app whose athletes have migrated.
///
/// # Errors
///
/// Returns an error if the repository delete fails.
pub async fn handle_delete_strava_pool_app(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(client_id): Path<String>,
) -> AppResult<Response> {
    if let Some(denied) = deny_if_not_super_admin(&admin_token) {
        return Ok(denied);
    }

    context
        .repos
        .oauth_tokens
        .delete_strava_pool_app(&client_id)
        .await?;

    info!(
        client_id = %client_id,
        service = %admin_token.service_name,
        "Strava pool app deleted by super-admin"
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("Strava pool app {client_id} deleted"),
            data: None,
        },
        StatusCode::OK,
    )
    .into_response())
}
