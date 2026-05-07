// ABOUTME: Admin routes for myth-busting summary + promote-topic feedback loop into Tier 6 guardrails
// ABOUTME: Reads Tier 5.5 verdicts and writes recurring claim phrases into harness_config.blocked_topics
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin myth-busting routes.
//!
//! `GET /api/admin/myth-busting/summary` wraps
//! [`crate::services::myth_busting::compute_summary`] for tenant pattern
//! review (display only).
//!
//! `POST /api/admin/myth-busting/promote-topic` is the **feedback loop**
//! that closes the gap between observation and enforcement: when an admin
//! sees a recurring unsupported claim phrase, they can promote it directly
//! into `harness_config.guardrails.blocked_topics`. The handler reads the
//! current document, appends the topic if not already present, validates
//! via [`crate::routes::admin::harness_config::validate_document`],
//! persists to `system_settings`, and installs the new snapshot in the
//! [`crate::harness_config_registry::HarnessConfigRegistry`] so the chat
//! pipeline starts rejecting replies containing the topic on the next turn.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::{Deserialize, Serialize};
use serde_json::to_string;
use tracing::{error, info};

use pierre_database::backends::factory::Database;

use crate::admin::models::{AdminPermission, ValidatedAdminToken};
use crate::errors::{AppError, AppResult};
use crate::harness_config_registry::{HarnessConfigRegistry, HarnessConfigSource};
use crate::models::TenantId;
use crate::routes::admin::harness_config::{
    validate_document, HarnessConfigDocument, HARNESS_CONFIG_SETTING_KEY,
};
use crate::services::myth_busting::{compute_summary, DEFAULT_VERDICT_LIMIT, MAX_VERDICTS_SCANNED};

use super::AdminApiContext;

/// Maximum allowed length for a single promoted topic substring.
///
/// Prevents an admin from pasting an entire reply into the blocked list
/// (which would never match) or exhausting storage with a giant string.
const MAX_TOPIC_LEN: usize = 200;

/// Query parameters for the myth-busting summary endpoint.
#[derive(Debug, Deserialize)]
pub struct MythBustingQuery {
    /// Tenant to compute the summary for (admin tokens span tenants).
    pub tenant_id: String,
    /// Maximum verdicts to scan, clamped to `1..=MAX_VERDICTS_SCANNED`.
    pub limit: Option<i64>,
}

/// Handle `GET /admin/myth-busting/summary`.
pub(super) async fn handle_get_summary(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(params): Query<MythBustingQuery>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ViewConfiguration)?;

    let tenant: TenantId = params
        .tenant_id
        .parse()
        .map_err(|_| AppError::invalid_input(format!("Invalid tenant ID: {}", params.tenant_id)))?;
    let limit = params
        .limit
        .unwrap_or(DEFAULT_VERDICT_LIMIT)
        .clamp(1, MAX_VERDICTS_SCANNED);

    let summary = compute_summary(&context.repos, tenant, limit)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to compute myth-busting summary");
            AppError::internal(format!("Failed to compute myth-busting summary: {e}"))
        })?;

    info!(
        service = %admin_token.service_name,
        tenant = %params.tenant_id,
        verdicts_scanned = summary.verdicts_scanned,
        flagged_total = summary.flagged_total,
        "admin fetched myth-busting summary"
    );

    Ok((StatusCode::OK, Json(summary)))
}

/// Body for `POST /api/admin/myth-busting/promote-topic`.
#[derive(Debug, Deserialize)]
pub struct PromoteTopicRequest {
    /// The substring (case-insensitive) the chat pipeline should reject.
    /// Trimmed and lowercased before comparison; admin-supplied text only.
    pub topic: String,
}

/// Response for `POST /api/admin/myth-busting/promote-topic`.
#[derive(Debug, Serialize)]
pub struct PromoteTopicResponse {
    /// The trimmed topic that was added (or already present).
    pub topic: String,
    /// Whether the topic was a new addition (`true`) or already in the list (`false`).
    pub added: bool,
    /// Total count of blocked topics after the operation.
    pub total_blocked_topics: usize,
}

/// Handle `POST /api/admin/myth-busting/promote-topic`.
///
/// Thin axum wrapper over [`promote_topic`] — request validation +
/// permission gate, then delegates the read-mutate-persist-install cycle.
pub(super) async fn handle_promote_topic(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Json(req): Json<PromoteTopicRequest>,
) -> AppResult<impl IntoResponse> {
    admin_token.require_permission(&AdminPermission::ManageConfiguration)?;

    let response = promote_topic(
        &context.database,
        &context.harness_config_registry,
        &req.topic,
    )
    .await?;

    info!(
        service = %admin_token.service_name,
        topic = %response.topic,
        added = response.added,
        total_blocked_topics = response.total_blocked_topics,
        "promote-topic complete"
    );

    Ok((StatusCode::OK, Json(response)))
}

/// Append `topic` to `harness_config.guardrails.blocked_topics`, persist,
/// and install the new snapshot.
///
/// Idempotent: re-promoting an existing topic returns `added=false` and
/// skips the storage / registry write, avoiding churn from misclicks.
///
/// Trims whitespace; rejects empty or overlong inputs. Comparison is
/// ASCII case-insensitive — same rule the guardrails apply at match time.
///
/// # Errors
///
/// - [`AppError::invalid_input`] if `topic` is empty after trim or
///   exceeds [`MAX_TOPIC_LEN`] characters.
/// - [`AppError::internal`] on serialization or DB persistence failure.
pub async fn promote_topic(
    db: &Database,
    registry: &HarnessConfigRegistry,
    raw_topic: &str,
) -> AppResult<PromoteTopicResponse> {
    let topic = raw_topic.trim().to_owned();
    if topic.is_empty() {
        return Err(AppError::invalid_input("topic must be non-empty"));
    }
    if topic.chars().count() > MAX_TOPIC_LEN {
        return Err(AppError::invalid_input(format!(
            "topic exceeds {MAX_TOPIC_LEN} characters"
        )));
    }

    // Take a snapshot of the running harness config and clone the
    // document so the registry's `Arc<HarnessConfigDocument>` stays
    // immutable until install time.
    let mut document: HarnessConfigDocument = (*registry.current().document).clone();

    let already_present = document
        .guardrails
        .blocked_topics
        .iter()
        .any(|t| t.eq_ignore_ascii_case(&topic));

    if already_present {
        return Ok(PromoteTopicResponse {
            topic,
            added: false,
            total_blocked_topics: document.guardrails.blocked_topics.len(),
        });
    }

    document.guardrails.blocked_topics.push(topic.clone());

    // Reuse the harness config validator so the same invariants the PUT
    // route enforces apply here. A failure here would mean we started
    // from a valid persisted document and only appended a string — the
    // validator would have to fail on length, but blocked_topics has no
    // count limit, so this is defensive only.
    validate_document(&document)?;

    let serialized = to_string(&document).map_err(|e| {
        error!(error = %e, "failed to serialize harness config after promote-topic");
        AppError::internal(format!("Failed to serialize harness config: {e}"))
    })?;
    db.set_system_setting(HARNESS_CONFIG_SETTING_KEY, &serialized)
        .await
        .map_err(|e| {
            error!(error = %e, "failed to persist harness config after promote-topic");
            AppError::internal(format!("Failed to persist harness config: {e}"))
        })?;
    registry.install(document.clone(), HarnessConfigSource::AdminUpdate);

    Ok(PromoteTopicResponse {
        topic,
        added: true,
        total_blocked_topics: document.guardrails.blocked_topics.len(),
    })
}
