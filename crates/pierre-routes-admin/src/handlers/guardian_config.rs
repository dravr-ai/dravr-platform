// ABOUTME: Admin route surfacing the Guardian security-policy configuration (system_settings.guardian_config)
// ABOUTME: GET the row with the effective policy, and a compare-and-set PUT with arming and impact gates, recorded on the audit trail

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Admin Guardian Configuration
//!
//! Exposes the [`GuardianConfigDocument`] the Guardian resolves its runtime
//! policy from. Mounted twice with the same handlers: under cookie auth at
//! `/api/admin/settings/guardian` (web console) and under admin-token auth at
//! `/admin/settings/guardian` (pierre-cli) — the auto-approval dual-mount
//! precedent. The policy is global, so a token scoped to one tenant is refused.
//!
//! GET answers two questions that can differ: `config` and `updated_at` are the
//! stored row, which is what an editor must start from, and `effective`,
//! `sources`, `env_pinned` and `loaded_at` are what THIS instance enforces. A
//! `GUARDIAN_*` env var pins its field for the life of the process, so an admin
//! edit to a pinned field persists but is shadowed until the pin is lifted.
//! `in_sync` says whether this instance has caught up with the row.
//!
//! PUT is a compare-and-set ([`super::setting_revision`]), so an instance that
//! booted on defaults cannot write them over the row either. A high-impact edit
//! also needs `acknowledge_impact`, and `plan_mode=enforce` needs
//! `GUARDIAN_PLAN_MODE_ARMABLE`.
//!
//! The registry install swaps THIS instance's snapshot; every other instance
//! picks the row up on its poll.
//!
//! A save and a refused save are each one `audit_events` row, written by the
//! recorder in [`crate::audit`] from the [`AuditDetail`] put on the response:
//! action `guardian.config.update`, and for a save the values before and after.
//! A token refused outright (403) never reached a save, so its row stays under
//! the route's own action.

use std::sync::Arc;

use axum::{
    extract::{rejection::JsonRejection, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use pierre_database::database::system_settings::SystemSetting;
use pierre_logging::host_identity;
use pierre_tool_runtime::guardian::{
    high_impact_changes, parse_persisted_document, validate_document, validate_plan_mode_arming,
    GuardianConfigDocument, GuardianConfigSource, GuardianFieldSources, GuardianPolicy,
    GUARDIAN_CONFIG_SETTING_KEY,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::{error, info, warn};

use pierre_core::admin::models::{AdminPermission, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult};

use super::setting_revision::{impact_refusal, malformed_refusal, stale_refusal, ExpectedRevision};
use crate::audit::AuditDetail;
use crate::context::AdminApiContext;

/// The audit action of a Guardian policy save, landed or refused.
const AUDIT_ACTION: &str = "guardian.config.update";

/// Wire response wrapper for both GET and PUT.
#[derive(Debug, Serialize)]
pub struct GuardianConfigResponse {
    /// The stored document at the current schema version, env overrides NOT
    /// folded in: what an editor starts from. When the row does not parse, the
    /// document this instance last held, with `degraded` set.
    pub config: GuardianConfigDocument,
    /// The policy this instance enforces right now (defaults ← config ← env).
    pub effective: GuardianPolicy,
    /// The compiled-in defaults: what a field left unset follows when no env var
    /// pins it. Not the same thing as `effective`.
    pub defaults: GuardianPolicy,
    /// Which resolution layer won each field on this instance.
    pub sources: GuardianFieldSources,
    /// Fields pinned by `GUARDIAN_*` env vars — edits to these persist but
    /// stay shadowed until the env pin is lifted.
    pub env_pinned: Vec<&'static str>,
    /// `"persisted"` when a row exists, `"default"` when no admin has ever
    /// written one.
    pub source: &'static str,
    /// RFC3339 timestamp of the last write, or `None` when no row exists. The
    /// concurrency token: echo it verbatim as `expected_updated_at`.
    pub updated_at: Option<String>,
    /// When this instance installed the snapshot it enforces.
    pub loaded_at: String,
    /// Which process answered: the revision plus an id unique to the process.
    pub instance_id: String,
    /// Whether this instance enforces the stored document. `false` until its
    /// poll catches up with a save another instance took, or while the row
    /// cannot be used.
    pub in_sync: bool,
    /// Whether the stored row is not simply in force: it does not parse, could
    /// not be read at boot, or holds a value the resolution refused.
    pub degraded: bool,
    /// Why, when `degraded`.
    pub degraded_reason: Option<String>,
    /// The version the row carries, when it is not the current one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stored_schema_version: Option<u32>,
    /// Keys in the row this binary does not read, and which a save would drop.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stored_unknown_fields: Vec<String>,
    /// Whether an admin edit may arm `plan_mode=enforce` on this deployment.
    pub plan_mode_armable: bool,
}

/// Body of a `PUT`: the document, flattened, and the save's two assertions. Only
/// the document is persisted.
#[derive(Debug, Deserialize)]
pub struct GuardianConfigPutRequest {
    /// The document to store.
    #[serde(flatten)]
    pub document: GuardianConfigDocument,
    /// The version the editor read.
    #[serde(default)]
    pub expected_updated_at: ExpectedRevision,
    /// The super admin has read what the high-impact edits in this save do.
    #[serde(default)]
    pub acknowledge_impact: bool,
}

const STALE_CODE: &str = "guardian_config_stale";
const IMPACT_CODE: &str = "guardian_config_impact_not_acknowledged";
const MALFORMED_CODE: &str = "guardian_config_malformed";
const IMPACT_MESSAGE: &str = "This save changes what athletes can do on every turn. Read each \
                              impacted field, then repeat the save with acknowledge_impact set \
                              to true.";
const STALE_MESSAGE: &str = "The Guardian policy changed since you read it, and nothing was \
                             saved. Reload it, reapply your edit, and save again.";
const UNNAMED_BASE_MESSAGE: &str = "A save must say which version it edits, and nothing was \
                                    saved. Reload the Guardian policy, and send its updated_at \
                                    as expected_updated_at.";

/// The current row, or an error: a failed read never reads as "no row", which
/// an editor would then write defaults over.
async fn read_row(context: &AdminApiContext) -> AppResult<Option<SystemSetting>> {
    context
        .database
        .get_system_setting(GUARDIAN_CONFIG_SETTING_KEY)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to read guardian config from system_settings");
            AppError::internal(format!("Failed to read guardian config: {e}"))
        })
}

/// What an edit is measured against: the row's document, or the defaults when
/// there is no row or it does not parse, which is all that is known to be in force.
fn stored_document(row: Option<&SystemSetting>) -> GuardianConfigDocument {
    row.and_then(|row| parse_persisted_document(&row.value).ok())
        .map(|persisted| persisted.document)
        .unwrap_or_default()
}

fn response_for(context: &AdminApiContext, row: Option<&SystemSetting>) -> GuardianConfigResponse {
    let registry = &context.guardian_config_registry;
    let snapshot = registry.current();
    let parsed = row.map(|row| parse_persisted_document(&row.value));

    let row_failure = parsed
        .as_ref()
        .and_then(|parsed| parsed.as_ref().err())
        .map(|e| format!("the stored row does not parse ({e})"));
    let (stored_schema_version, stored_unknown_fields) = match &parsed {
        Some(Ok(persisted)) => (
            persisted.stored_schema_version,
            persisted.unknown_fields.clone(),
        ),
        _ => (None, Vec::new()),
    };
    let row_document = match parsed {
        Some(Ok(persisted)) => Some(persisted.document),
        Some(Err(_)) => None,
        None => Some(GuardianConfigDocument::default()),
    };
    let in_sync = row_document.as_ref() == Some(&*snapshot.document);
    let degraded_reason = match (row_failure, snapshot.degraded_reason()) {
        (Some(row), Some(held)) if snapshot.load_failure.is_none() => {
            Some(format!("{row}; {held}"))
        }
        (_, Some(held)) => Some(held),
        (Some(row), None) => Some(row),
        (None, None) => None,
    };

    GuardianConfigResponse {
        config: row_document.unwrap_or_else(|| (*snapshot.document).clone()),
        effective: snapshot.guardian.policy().clone(),
        defaults: GuardianPolicy::default(),
        sources: snapshot.field_sources,
        env_pinned: snapshot.field_sources.env_pinned(),
        source: if row.is_some() {
            "persisted"
        } else {
            "default"
        },
        updated_at: row.map(|row| row.updated_at.to_rfc3339()),
        loaded_at: snapshot.loaded_at.to_rfc3339(),
        instance_id: format!(
            "{}:{}",
            host_identity().unwrap_or("local"),
            registry.instance_nonce()
        ),
        in_sync,
        degraded: degraded_reason.is_some(),
        degraded_reason,
        stored_schema_version,
        stored_unknown_fields,
        plan_mode_armable: registry.plan_mode_armable(),
    }
}

/// Handle `GET /admin/settings/guardian` (and its console mount).
///
/// Reads the row, and a failed read is an error rather than a default document,
/// so nobody edits from an empty form that a save would write over the policy.
///
/// # Errors
///
/// Returns an error when the caller lacks `ViewConfiguration`, the token is
/// scoped to one tenant, or the `system_settings` read fails.
pub async fn handle_get_guardian_config(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;
    admin_token.require_global_scope()?;

    info!(
        token_id = %admin_token.token_id,
        "admin fetched guardian config"
    );

    let row = read_row(&context).await?;
    Ok((StatusCode::OK, Json(response_for(&context, row.as_ref()))))
}

/// Handle `PUT /admin/settings/guardian` (and its console mount).
///
/// Validates, compare-and-sets the row, then installs the new snapshot
/// (write-then-install, so a DB failure leaves both stores on the previous
/// values). The response reflects the post-install effective policy, so an
/// env-pinned field shows its pinned value with `sources.<field> == "env"` —
/// never the shadowed edit.
///
/// # Errors
///
/// Returns an error when the caller lacks `ManageConfiguration`, the token is
/// scoped to one tenant, or the `system_settings` access fails. The refusals of a
/// save are responses, so each one reaches the audit trail under this policy's
/// action: a document that fails validation or arms plan mode where that is not
/// allowed is a 400, a stale or missing token and an unacknowledged high-impact
/// edit are 409s, and a body that does not parse has the 409's shape, so its
/// message reaches the page.
pub async fn handle_put_guardian_config(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    body: Result<Json<GuardianConfigPutRequest>, JsonRejection>,
) -> AppResult<Response> {
    admin_token.require_permission(&AdminPermission::ManageConfiguration)?;
    admin_token.require_global_scope()?;

    let Json(request) = match body {
        Ok(body) => body,
        Err(rejection) => {
            return Ok(refused_save(malformed_refusal(
                MALFORMED_CODE,
                "The Guardian policy",
                &rejection,
            )))
        }
    };
    let GuardianConfigPutRequest {
        document,
        expected_updated_at,
        acknowledge_impact,
    } = request;
    // Answered here rather than with `?`, so the refusal carries the action the
    // policy's own trail lists.
    if let Err(refused) = validate_document(&document).and_then(|()| {
        validate_plan_mode_arming(
            &document,
            context.guardian_config_registry.plan_mode_armable(),
        )
    }) {
        return Ok(refused_save(refused.into_response()));
    }

    let expected = match expected_updated_at {
        ExpectedRevision::Unnamed => {
            return Ok(refused_save(stale_refusal(
                STALE_CODE,
                UNNAMED_BASE_MESSAGE,
            )))
        }
        ExpectedRevision::NoRow => None,
        ExpectedRevision::At(stamp) => Some(stamp),
    };
    let row = read_row(&context).await?;
    if row.as_ref().map(|row| row.updated_at) != expected {
        return Ok(refused_save(stale_refusal(STALE_CODE, STALE_MESSAGE)));
    }
    let impacts = high_impact_changes(&stored_document(row.as_ref()), &document);
    if !acknowledge_impact && !impacts.is_empty() {
        return Ok(refused_save(impact_refusal(
            IMPACT_CODE,
            IMPACT_MESSAGE,
            impacts,
        )));
    }

    let serialized = serde_json::to_string(&document).map_err(|e| {
        error!(error = %e, "failed to serialize guardian config");
        AppError::internal(format!("Failed to serialize guardian config: {e}"))
    })?;
    let Some(updated_at) = context
        .database
        .compare_and_set_system_setting(GUARDIAN_CONFIG_SETTING_KEY, &serialized, expected)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to persist guardian config");
            AppError::internal(format!("Failed to persist guardian config: {e}"))
        })?
    else {
        return Ok(refused_save(stale_refusal(STALE_CODE, STALE_MESSAGE)));
    };

    warn_on_dropped_fields(row.as_ref());
    let document_after = serde_json::to_value(&document).unwrap_or(Value::Null);
    context
        .guardian_config_registry
        .install(document, GuardianConfigSource::AdminUpdate);
    let audit = saved_detail(row.as_ref(), &document_after);

    let written = SystemSetting {
        key: GUARDIAN_CONFIG_SETTING_KEY.to_owned(),
        value: serialized,
        description: None,
        updated_at,
    };
    let response = response_for(&context, Some(&written));
    log_save(&admin_token, &response);
    Ok(audit.attach((StatusCode::OK, Json(response)).into_response()))
}

/// A refused save's response, carrying the action so the policy's own trail lists it.
fn refused_save(response: Response) -> Response {
    AuditDetail::new(
        AUDIT_ACTION,
        format!(
            "Refused a Guardian policy save ({})",
            response.status().as_u16()
        ),
    )
    .attach(response)
}

/// The row before a save as stored: unknown fields kept, since the save drops them
/// from the row, and text that does not parse kept verbatim.
fn stored_value(row: Option<&SystemSetting>) -> Value {
    row.map_or(Value::Null, |row| {
        serde_json::from_str(&row.value).unwrap_or_else(|_| Value::String(row.value.clone()))
    })
}

/// A landed save's audit detail: what it moved, and both documents.
fn saved_detail(row: Option<&SystemSetting>, after: &Value) -> AuditDetail {
    let before = stored_value(row);
    AuditDetail::new(AUDIT_ACTION, change_summary(&before, after)).change(before, after.clone())
}

/// One readable line naming each field the save moved.
fn change_summary(before: &Value, after: &Value) -> String {
    let Some(new_fields) = after.as_object() else {
        return "Saved the Guardian policy".to_owned();
    };
    match before {
        Value::Null => return "Created the Guardian policy".to_owned(),
        Value::Object(_) => {}
        _ => return "Replaced a Guardian policy row that was not a document".to_owned(),
    }
    let missing = Value::Null;
    let moved = new_fields.iter().filter_map(|(field, new)| {
        let old = before.get(field).unwrap_or(&missing);
        (old != new).then(|| format!("{field}: {old} → {new}"))
    });
    let dropped = before
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(field, _)| !new_fields.contains_key(*field))
        .map(|(field, old)| format!("{field}: {old} → dropped"));
    let changes: Vec<String> = moved.chain(dropped).collect();
    if changes.is_empty() {
        "Saved the Guardian policy with no field changed".to_owned()
    } else {
        format!("Guardian policy {}", changes.join(", "))
    }
}

/// A save replaces the whole row, so what this binary does not read is gone from
/// it. The audit row's `before` keeps the original.
fn warn_on_dropped_fields(row: Option<&SystemSetting>) {
    let Some(persisted) = row.and_then(|row| parse_persisted_document(&row.value).ok()) else {
        return;
    };
    if !persisted.unknown_fields.is_empty() {
        warn!(
            unknown = ?persisted.unknown_fields,
            "guardian config save dropped fields this binary does not read"
        );
    }
}

fn log_save(admin_token: &ValidatedAdminToken, response: &GuardianConfigResponse) {
    let effective = &response.effective;
    info!(
        token_id = %admin_token.token_id,
        mode = effective.mode.as_str(),
        max_destructive_per_turn = effective.max_destructive_per_turn,
        max_writes_per_turn = effective.max_writes_per_turn,
        external_send = ?effective.external_send,
        tainted_destructive = ?effective.tainted_destructive,
        plan_mode = effective.plan_mode.as_str(),
        env_pinned = ?response.env_pinned,
        "admin updated guardian config"
    );
}
