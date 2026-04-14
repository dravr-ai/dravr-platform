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
use serde::{Deserialize, Serialize};
use serde_json::{from_str, to_string};
use tracing::{error, info};

use crate::admin::models::{AdminPermission, ValidatedAdminToken};
use crate::errors::{AppError, AppResult};

use super::AdminApiContext;

/// `system_settings` row key the harness config document is persisted under.
pub const HARNESS_CONFIG_SETTING_KEY: &str = "harness_config";

/// Compaction tunables that the dispatch path reads from
/// `services::conversation_compaction::CompactionConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessCompactionConfig {
    /// Target context window size in tokens (default `128_000`).
    pub window_tokens: u32,
    /// Fraction of `window_tokens` that triggers a summarization pass.
    pub warn_threshold: f32,
    /// Fraction of `window_tokens` that triggers the sliding-window emergency fallback.
    pub emergency_threshold: f32,
    /// How many of the oldest history turns to summarize when warn fires.
    pub summarize_oldest_n: u32,
    /// How many of the oldest history turns to drop under emergency mode.
    pub sliding_drop_n: u32,
}

impl Default for HarnessCompactionConfig {
    fn default() -> Self {
        Self {
            window_tokens: 128_000,
            warn_threshold: 0.70,
            emergency_threshold: 0.95,
            summarize_oldest_n: 6,
            sliding_drop_n: 4,
        }
    }
}

/// Tier 6 text guardrail tunables — disclaimer prefix, blocked topics, length cap.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HarnessGuardrailsConfig {
    /// Maximum character length for an outbound coach response. `0` disables the cap.
    pub max_response_chars: u32,
    /// Substrings (case-insensitive) that must not appear in any response.
    pub blocked_topics: Vec<String>,
    /// Substrings (case-insensitive) that, when matched, prepend the disclaimer.
    pub disclaimer_triggers: Vec<String>,
    /// Disclaimer text prepended when a trigger keyword is matched.
    pub disclaimer_text: String,
}

impl Default for HarnessGuardrailsConfig {
    fn default() -> Self {
        Self {
            max_response_chars: 5_000,
            blocked_topics: Vec::new(),
            disclaimer_triggers: vec![
                "injury".to_owned(),
                "pain".to_owned(),
                "medication".to_owned(),
                "diagnose".to_owned(),
                "doctor".to_owned(),
            ],
            disclaimer_text:
                "**Medical disclaimer:** I'm a fitness coach, not a medical professional. \
                 Please consult a qualified clinician for any injury, pain, or medication question."
                    .to_owned(),
        }
    }
}

/// Top-level document persisted under [`HARNESS_CONFIG_SETTING_KEY`].
///
/// Round-tripped to JSON in `system_settings.value`. Versioned via the
/// `schema_version` field so future migrations can reject documents that
/// predate breaking changes without losing operator data.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HarnessConfigDocument {
    /// Document schema version. Bump when fields are removed or renamed.
    pub schema_version: u32,
    /// Compaction tunables.
    pub compaction: HarnessCompactionConfig,
    /// Text guardrail tunables.
    pub guardrails: HarnessGuardrailsConfig,
}

impl Default for HarnessConfigDocument {
    fn default() -> Self {
        Self {
            schema_version: 1,
            compaction: HarnessCompactionConfig::default(),
            guardrails: HarnessGuardrailsConfig::default(),
        }
    }
}

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
pub(super) async fn handle_get_harness_config(
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
pub(super) async fn handle_put_harness_config(
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

/// Reject obviously broken documents before they reach storage.
///
/// The dispatch path can survive most mistuned values, but a few are
/// load-bearing: `warn_threshold < emergency_threshold` so the two-stage
/// compactor runs in order, both thresholds must be in `(0, 1]`, and the
/// disclaimer text cannot be empty when triggers are set (an empty
/// disclaimer with triggers would silently break the medical-fallback
/// behavior).
///
/// # Errors
///
/// Returns [`AppError::invalid_input`] when any invariant fails.
pub fn validate_document(doc: &HarnessConfigDocument) -> AppResult<()> {
    let c = &doc.compaction;
    if c.window_tokens == 0 {
        return Err(AppError::invalid_input("window_tokens must be > 0"));
    }
    if !(0.0..=1.0).contains(&c.warn_threshold) {
        return Err(AppError::invalid_input("warn_threshold must be in (0, 1]"));
    }
    if !(0.0..=1.0).contains(&c.emergency_threshold) {
        return Err(AppError::invalid_input(
            "emergency_threshold must be in (0, 1]",
        ));
    }
    if c.warn_threshold >= c.emergency_threshold {
        return Err(AppError::invalid_input(
            "warn_threshold must be strictly less than emergency_threshold",
        ));
    }

    let g = &doc.guardrails;
    if !g.disclaimer_triggers.is_empty() && g.disclaimer_text.trim().is_empty() {
        return Err(AppError::invalid_input(
            "disclaimer_text must be non-empty when disclaimer_triggers is set",
        ));
    }

    Ok(())
}
