// ABOUTME: Phase B Sprint C3 — admin route surfacing the global coaching harness configuration
// ABOUTME: Compaction thresholds, text guardrails, and verification defaults persisted via SystemSettings
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Admin Harness Configuration
//!
//! Exposes a single JSON document at `/admin/settings/harness` that bundles
//! the dispatch-time tunables the harness uses — compaction window /
//! thresholds, text guardrails (blocked topics, disclaimer, length cap),
//! and the default fallback behavior when verification fires.
//!
//! Persistence rides on the existing `system_settings` key/value table;
//! each tenant currently shares a single global document. Per-tenant
//! overrides are a Phase D follow-up — adding the column now would
//! complicate the dispatch read path without enabling new behavior.

use std::sync::Arc;

use axum::{extract::State, http::StatusCode, response::IntoResponse, Extension, Json};
use pierre_contremaitre::harness_config_document::{
    validate_document, HarnessConfigDocument, HARNESS_CONFIG_SETTING_KEY,
};
use pierre_contremaitre::harness_config_registry::HarnessConfigSource;
use serde::Serialize;
use serde_json::{from_str, to_string};
use tracing::{error, info};

use pierre_core::admin::models::{AdminPermission, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult};

use crate::context::AdminApiContext;

/// Wire response wrapper for both GET and PUT.
#[derive(Debug, Serialize)]
pub struct HarnessConfigResponse {
    /// The (validated) document the server holds.
    pub config: HarnessConfigDocument,
    /// `"persisted"` when the document came from `system_settings`,
    /// `"default"` when no row exists and the response is the compile-time
    /// default.
    pub source: &'static str,
    /// RFC3339 timestamp of the last write, or `None` when the response is
    /// the compile-time default.
    pub updated_at: Option<String>,
}

/// Handle `GET /admin/settings/harness`.
///
/// Returns the persisted document or [`HarnessConfigDocument::default`]
/// when no row has ever been written. The `source` field tells the UI
/// whether the values are operator-written (`persisted`) or compiled-in
/// defaults (`default`).
pub(crate) async fn handle_get_harness_config(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

    info!(
        service = %admin_token.service_name,
        "admin fetched harness config"
    );

    let setting = context
        .database
        .get_system_setting(HARNESS_CONFIG_SETTING_KEY)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to read harness config from system_settings");
            AppError::internal(format!("Failed to read harness config: {e}"))
        })?;

    let response = match setting {
        Some(row) => {
            let parsed = from_str::<HarnessConfigDocument>(&row.value).map_err(|e| {
                error!(error = %e, "failed to deserialize harness config");
                AppError::internal(format!("Stored harness config is invalid JSON: {e}"))
            })?;
            HarnessConfigResponse {
                config: parsed,
                source: "persisted",
                updated_at: Some(row.updated_at.to_rfc3339()),
            }
        }
        None => HarnessConfigResponse {
            config: HarnessConfigDocument::default(),
            source: "default",
            updated_at: None,
        },
    };

    Ok((StatusCode::OK, Json(response)))
}

/// Handle `PUT /admin/settings/harness`.
///
/// Validates a few invariants the dispatch path depends on (warn <
/// emergency, thresholds within `(0, 1]`, non-empty disclaimer when
/// triggers are set) and overwrites the stored document. Returns the
/// persisted document so the UI can confirm the round-trip.
pub(crate) async fn handle_put_harness_config(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Json(document): Json<HarnessConfigDocument>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ManageConfiguration)?;

    validate_document(&document)?;

    info!(
        service = %admin_token.service_name,
        window_tokens = document.compaction.window_tokens,
        guardrail_blocked_topics = document.guardrails.blocked_topics.len(),
        "admin updated harness config"
    );

    let serialized = to_string(&document).map_err(|e| {
        error!(error = %e, "failed to serialize harness config");
        AppError::internal(format!("Failed to serialize harness config: {e}"))
    })?;

    context
        .database
        .set_system_setting(HARNESS_CONFIG_SETTING_KEY, &serialized)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to persist harness config");
            AppError::internal(format!("Failed to persist harness config: {e}"))
        })?;

    // Swap the in-memory snapshot the chat pipeline reads from. Done
    // after the row write so a DB failure leaves both stores consistent
    // on the previous values.
    context
        .harness_config_registry
        .install(document.clone(), HarnessConfigSource::AdminUpdate);

    let updated_at = chrono::Utc::now().to_rfc3339();
    Ok((
        StatusCode::OK,
        Json(HarnessConfigResponse {
            config: document,
            source: "persisted",
            updated_at: Some(updated_at),
        }),
    ))
}
