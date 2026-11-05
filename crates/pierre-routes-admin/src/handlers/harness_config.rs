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
//! complicate the dispatch read path without enabling new behavior. Being
//! global, it refuses a token scoped to one tenant.
//!
//! PUT is a compare-and-set ([`super::setting_revision`]) on the `updated_at` GET
//! returned. A row that does not parse still answers GET, with what this instance
//! holds and the row's `updated_at`, because a save is the only way to repair it.
//!
//! The registry install reaches the instance the request landed on; every other
//! instance converges through its poll of the row.

use std::sync::Arc;

use axum::{
    extract::{rejection::JsonRejection, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use pierre_contremaitre::harness_config_document::{
    validate_document, HarnessConfigDocument, HARNESS_CONFIG_SETTING_KEY,
};
use pierre_contremaitre::harness_config_registry::HarnessConfigSource;
use serde::{Deserialize, Serialize};
use serde_json::{from_str, to_string};
use tracing::{error, info};

use pierre_core::admin::models::{AdminPermission, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult};

use super::setting_revision::{malformed_refusal, stale_refusal, ExpectedRevision};
use crate::context::AdminApiContext;

const STALE_CODE: &str = "harness_config_stale";
const MALFORMED_CODE: &str = "harness_config_malformed";
const STALE_MESSAGE: &str = "The harness configuration changed since you read it, and nothing \
                             was saved. Reload it, reapply your edit, and save again.";
const UNNAMED_BASE_MESSAGE: &str = "A save must say which version it edits, and nothing was \
                                    saved. Reload the harness configuration, and send its \
                                    updated_at as expected_updated_at.";

/// Wire response wrapper for both GET and PUT.
#[derive(Debug, Serialize)]
pub struct HarnessConfigResponse {
    /// The (validated) document the server holds.
    pub config: HarnessConfigDocument,
    /// `"persisted"` when the document came from `system_settings`,
    /// `"default"` when no row exists and the response is the compile-time
    /// default.
    pub source: &'static str,
    /// RFC3339 timestamp of the last write, or `None` when no row exists. The
    /// concurrency token: echo it verbatim as `expected_updated_at`.
    pub updated_at: Option<String>,
    /// Why `config` is not the stored row: it does not parse, so this is what this
    /// instance holds, and a save replaces the row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub degraded_reason: Option<String>,
}

/// Body of a `PUT`: the document, flattened, and the version it edits. Only the
/// document is persisted.
#[derive(Debug, Deserialize)]
pub(crate) struct HarnessConfigPutRequest {
    /// The document to store.
    #[serde(flatten)]
    document: HarnessConfigDocument,
    /// The version the editor read.
    #[serde(default)]
    expected_updated_at: ExpectedRevision,
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
    admin_token.require_global_scope()?;

    info!(
        token_id = %admin_token.token_id,
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
            let updated_at = Some(row.updated_at.to_rfc3339());
            match from_str::<HarnessConfigDocument>(&row.value) {
                Ok(config) => HarnessConfigResponse {
                    config,
                    source: "persisted",
                    updated_at,
                    degraded_reason: None,
                },
                Err(e) => {
                    error!(error = %e, "stored harness config does not parse");
                    HarnessConfigResponse {
                        config: (*context.harness_config_registry.current().document).clone(),
                        source: "persisted",
                        updated_at,
                        degraded_reason: Some(format!("the stored row does not parse ({e})")),
                    }
                }
            }
        }
        None => HarnessConfigResponse {
            config: HarnessConfigDocument::default(),
            source: "default",
            updated_at: None,
            degraded_reason: None,
        },
    };

    Ok((StatusCode::OK, Json(response)))
}

/// Handle `PUT /admin/settings/harness`.
///
/// Validates a few invariants the dispatch path depends on (warn <
/// emergency, thresholds within `(0, 1]`, non-empty disclaimer when
/// triggers are set) and compare-and-sets the stored document. Returns the
/// persisted document so the UI can confirm the round-trip. A stale or missing
/// token is a 409 that writes nothing, and a body that does not parse is a refusal
/// with the same shape.
pub(crate) async fn handle_put_harness_config(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    body: Result<Json<HarnessConfigPutRequest>, JsonRejection>,
) -> AppResult<Response> {
    admin_token.require_permission(&AdminPermission::ManageConfiguration)?;
    admin_token.require_global_scope()?;

    let Json(HarnessConfigPutRequest {
        document,
        expected_updated_at,
    }) = match body {
        Ok(body) => body,
        Err(rejection) => {
            return Ok(malformed_refusal(
                MALFORMED_CODE,
                "The harness configuration",
                &rejection,
            ))
        }
    };

    validate_document(&document)?;

    let expected = match expected_updated_at {
        ExpectedRevision::Unnamed => {
            return Ok(stale_refusal(STALE_CODE, UNNAMED_BASE_MESSAGE));
        }
        ExpectedRevision::NoRow => None,
        ExpectedRevision::At(stamp) => Some(stamp),
    };

    let serialized = to_string(&document).map_err(|e| {
        error!(error = %e, "failed to serialize harness config");
        AppError::internal(format!("Failed to serialize harness config: {e}"))
    })?;

    let Some(updated_at) = context
        .database
        .compare_and_set_system_setting(HARNESS_CONFIG_SETTING_KEY, &serialized, expected)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to persist harness config");
            AppError::internal(format!("Failed to persist harness config: {e}"))
        })?
    else {
        return Ok(stale_refusal(STALE_CODE, STALE_MESSAGE));
    };

    info!(
        token_id = %admin_token.token_id,
        window_tokens = document.compaction.window_tokens,
        guardrail_blocked_topics = document.guardrails.blocked_topics.len(),
        "admin updated harness config"
    );

    // Swap the in-memory snapshot the chat pipeline reads from. Done
    // after the row write so a DB failure leaves both stores consistent
    // on the previous values.
    context
        .harness_config_registry
        .install(document.clone(), HarnessConfigSource::AdminUpdate);

    Ok((
        StatusCode::OK,
        Json(HarnessConfigResponse {
            config: document,
            source: "persisted",
            updated_at: Some(updated_at.to_rfc3339()),
            degraded_reason: None,
        }),
    )
        .into_response())
}
