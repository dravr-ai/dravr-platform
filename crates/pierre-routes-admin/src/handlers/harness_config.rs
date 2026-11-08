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

use std::collections::BTreeSet;
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
use serde_json::{from_str, to_string, Value};
use tracing::{error, info};

use pierre_core::admin::models::{AdminPermission, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult};
use pierre_logging::host_identity;

use super::setting_revision::{malformed_refusal, stale_refusal, ExpectedRevision};
use crate::audit::AuditDetail;
use crate::context::AdminApiContext;

/// The audit action of a harness configuration save.
const AUDIT_ACTION: &str = "harness_config.update";

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
    /// What the instance that answered is running, which a stored row does not
    /// say: an instance applies a save another took on its next poll.
    pub in_force: HarnessInForce,
}

/// One instance's view of the document it applies to chat turns.
#[derive(Debug, Serialize)]
pub struct HarnessInForce {
    /// Which process answered: the revision plus an id unique to the process.
    pub instance_id: String,
    /// When this instance installed the values it runs.
    pub loaded_at: String,
    /// Whether this instance runs the stored document. `false` until its poll
    /// catches up with a save another instance took, or while the row is unusable.
    pub in_sync: bool,
    /// Why this instance could not take its values from the row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_failure: Option<String>,
}

/// `stored` is the document the row holds, or `None` when it does not parse.
fn in_force(context: &AdminApiContext, stored: Option<&HarnessConfigDocument>) -> HarnessInForce {
    let registry = &context.harness_config_registry;
    let snapshot = registry.current();
    HarnessInForce {
        instance_id: format!(
            "{}:{}",
            host_identity().unwrap_or("local"),
            registry.instance_nonce()
        ),
        loaded_at: snapshot.loaded_at.to_rfc3339(),
        in_sync: snapshot.load_failure.is_none() && stored == Some(&*snapshot.document),
        load_failure: snapshot.load_failure.clone(),
    }
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

    let response = if let Some(row) = setting {
        let updated_at = Some(row.updated_at.to_rfc3339());
        match from_str::<HarnessConfigDocument>(&row.value) {
            Ok(config) => HarnessConfigResponse {
                in_force: in_force(&context, Some(&config)),
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
                    in_force: in_force(&context, None),
                }
            }
        }
    } else {
        let config = HarnessConfigDocument::default();
        HarnessConfigResponse {
            in_force: in_force(&context, Some(&config)),
            config,
            source: "default",
            updated_at: None,
            degraded_reason: None,
        }
    };

    Ok((StatusCode::OK, Json(response)))
}

/// Handle `PUT /admin/settings/harness`.
///
/// Validates a few invariants the dispatch path depends on (warn <
/// emergency, thresholds within `(0, 1]`) and compare-and-sets the stored document. Returns the
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

    // Read before the write, for the audit row. A failure to read it costs the
    // audit its `before`, never the save.
    let before = context
        .database
        .get_system_setting(HARNESS_CONFIG_SETTING_KEY)
        .await
        .ok()
        .flatten()
        .and_then(|row| from_str::<Value>(&row.value).ok())
        .unwrap_or(Value::Null);

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

    let after = from_str::<Value>(&serialized).unwrap_or(Value::Null);
    let audit =
        AuditDetail::new(AUDIT_ACTION, change_summary(&before, &after)).change(before, after);

    Ok(audit.attach(
        (
            StatusCode::OK,
            Json(HarnessConfigResponse {
                in_force: in_force(&context, Some(&document)),
                config: document,
                source: "persisted",
                updated_at: Some(updated_at.to_rfc3339()),
                degraded_reason: None,
            }),
        )
            .into_response(),
    ))
}

/// One readable line naming each setting the save moved, by its dotted path.
fn change_summary(before: &Value, after: &Value) -> String {
    if before.is_null() {
        return "Created the harness configuration".to_owned();
    }
    let mut changes = Vec::new();
    collect_changes("", before, after, &mut changes);
    if changes.is_empty() {
        "Saved the harness configuration with no setting changed".to_owned()
    } else {
        format!("Harness configuration {}", changes.join(", "))
    }
}

fn collect_changes(path: &str, before: &Value, after: &Value, changes: &mut Vec<String>) {
    if let (Value::Object(old), Value::Object(new)) = (before, after) {
        let keys: BTreeSet<&String> = old.keys().chain(new.keys()).collect();
        for key in keys {
            let child = if path.is_empty() {
                key.clone()
            } else {
                format!("{path}.{key}")
            };
            collect_changes(
                &child,
                old.get(key).unwrap_or(&Value::Null),
                new.get(key).unwrap_or(&Value::Null),
                changes,
            );
        }
    } else if before != after {
        changes.push(format!("{path}: {before} → {after}"));
    }
}
