// ABOUTME: Admin route surfacing the Guardian security-policy configuration (system_settings.guardian_config)
// ABOUTME: GET/PUT the document; responses carry the effective policy + per-field sources incl. env-pinned fields

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Admin Guardian Configuration
//!
//! Exposes the [`GuardianConfigDocument`] the Guardian resolves its runtime
//! policy from. Mounted twice with the same handlers: under cookie auth at
//! `/api/admin/settings/guardian` (web console) and under admin-token auth at
//! `/admin/settings/guardian` (pierre-cli) — the auto-approval dual-mount
//! precedent.
//!
//! Every response carries the *effective* policy and a per-field source map
//! (`default` / `database` / `env`): a `GUARDIAN_*` env var pins its field
//! for the life of the process, so an admin edit to a pinned field persists
//! but is shadowed until the pin is lifted — the `env_pinned` list makes that
//! visible instead of silently confusing the operator.
//!
//! The registry install only swaps THIS process's snapshot (the harness
//! config trade-off): the single-instance dev deployment reads it on the next
//! dispatch; a multi-instance fleet pins posture via env instead.

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use pierre_tool_runtime::guardian::{
    validate_document, GuardianConfigDocument, GuardianConfigSource, GuardianFieldSources,
    GuardianPolicy, GUARDIAN_CONFIG_SETTING_KEY,
};
use serde::Serialize;
use serde_json::to_string;
use tracing::{error, info};

use pierre_core::admin::models::{AdminPermission, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult};

use crate::context::AdminApiContext;

/// Wire response wrapper for both GET and PUT.
#[derive(Debug, Serialize)]
pub struct GuardianConfigResponse {
    /// The persisted (or default) document — env overrides NOT folded in.
    pub config: GuardianConfigDocument,
    /// The policy actually enforcing right now (defaults ← config ← env).
    pub effective: GuardianPolicy,
    /// Which resolution layer won each field.
    pub sources: GuardianFieldSources,
    /// Fields pinned by `GUARDIAN_*` env vars — edits to these persist but
    /// stay shadowed until the env pin is lifted.
    pub env_pinned: Vec<&'static str>,
    /// `"persisted"` when a document row backs the snapshot, `"default"`
    /// when no admin has ever written one.
    pub source: &'static str,
    /// RFC3339 timestamp of the last write, or `None` for the default.
    pub updated_at: Option<String>,
}

fn snapshot_response(
    context: &AdminApiContext,
    updated_at: Option<String>,
) -> GuardianConfigResponse {
    let snapshot = context.guardian_config_registry.current();
    GuardianConfigResponse {
        config: (*snapshot.document).clone(),
        effective: snapshot.guardian.policy().clone(),
        sources: snapshot.field_sources,
        env_pinned: snapshot.field_sources.env_pinned(),
        source: match snapshot.source {
            GuardianConfigSource::Defaults => "default",
            GuardianConfigSource::Database | GuardianConfigSource::AdminUpdate => "persisted",
        },
        updated_at,
    }
}

/// Handle `GET /admin/settings/guardian` (and the cookie twin).
///
/// Serves the registry snapshot — the policy THIS process enforces — rather
/// than re-reading the row, so what the operator sees is what the Guardian
/// does. The row is consulted only for `updated_at`.
///
/// # Errors
///
/// Returns an error when the caller lacks `ViewConfiguration` or the
/// `system_settings` read fails.
pub async fn handle_get_guardian_config(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

    info!(
        service = %admin_token.service_name,
        "admin fetched guardian config"
    );

    let updated_at = context
        .database
        .get_system_setting(GUARDIAN_CONFIG_SETTING_KEY)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to read guardian config from system_settings");
            AppError::internal(format!("Failed to read guardian config: {e}"))
        })?
        .map(|row| row.updated_at.to_rfc3339());

    Ok((
        StatusCode::OK,
        Json(snapshot_response(&context, updated_at)),
    ))
}

/// Handle `PUT /admin/settings/guardian` (and the cookie twin).
///
/// Validates, persists, then installs the new snapshot (write-then-install,
/// so a DB failure leaves both stores on the previous values). The response
/// reflects the post-install effective policy, so an env-pinned field shows
/// its pinned value with `sources.<field> == "env"` — never the shadowed edit.
///
/// # Errors
///
/// Returns an error when the caller lacks `ManageConfiguration`, the document
/// fails validation, or the `system_settings` write fails.
pub async fn handle_put_guardian_config(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Json(document): Json<GuardianConfigDocument>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ManageConfiguration)?;

    validate_document(&document)?;

    let serialized = to_string(&document).map_err(|e| {
        error!(error = %e, "failed to serialize guardian config");
        AppError::internal(format!("Failed to serialize guardian config: {e}"))
    })?;

    context
        .database
        .set_system_setting(GUARDIAN_CONFIG_SETTING_KEY, &serialized)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to persist guardian config");
            AppError::internal(format!("Failed to persist guardian config: {e}"))
        })?;

    context
        .guardian_config_registry
        .install(document, GuardianConfigSource::AdminUpdate);

    let response = snapshot_response(&context, Some(chrono::Utc::now().to_rfc3339()));
    info!(
        service = %admin_token.service_name,
        mode = response.effective.mode.as_str(),
        plan_mode = response.effective.plan_mode.as_str(),
        env_pinned = ?response.env_pinned,
        "admin updated guardian config"
    );

    Ok((StatusCode::OK, Json(response)))
}
