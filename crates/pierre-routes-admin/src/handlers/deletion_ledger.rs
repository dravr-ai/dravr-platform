// ABOUTME: Operator routes over the deletion ledger: the hash-chained receipts and the import tombstones
// ABOUTME: Super-admin only; pierre-cli verifies the chain from the receipts and clears a tombstone, audited

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The deletion ledger, for an operator.
//!
//! Every account deletion appends a receipt chained to the one before it and
//! leaves tombstones on the account's provider accounts and email
//! (`pierre_database::repositories::account_deletion`). These routes read
//! both back for `pierre-cli deletion`, which verifies the chain itself
//! rather than trusting this server's word for it, and let an operator clear
//! a tombstone when the person holding the identity asks them to. A receipt
//! names its account only by the account id's SHA-256, and a tombstone its
//! identity only by a digest.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension,
};
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, to_value};
use tracing::error;

use pierre_core::admin::models::{AdminAction, AdminTokenUsage, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult};
use pierre_services::import_tombstones::clear_by_operator;

use super::api_keys::json_response;
use super::strava_pool::deny_if_not_super_admin;
use super::types::AdminResponse;
use crate::context::AdminApiContext;

/// The most receipts one page returns.
const MAX_RECEIPTS_PAGE: i64 = 1000;

/// The most tombstones one listing returns.
const MAX_TOMBSTONES_LISTED: i64 = 1000;

/// Query of `GET /admin/deletion-receipts`.
#[derive(Debug, Default, Deserialize)]
pub struct ReceiptPageQuery {
    /// Receipts after this `seq` (0, the default, starts at the first).
    #[serde(default)]
    pub after_seq: i64,
    /// Page size, clamped to `1..=1000` (default 1000).
    pub limit: Option<i64>,
}

/// Query of `GET /admin/import-tombstones`.
#[derive(Debug, Default, Deserialize)]
pub struct TombstoneListQuery {
    /// Include cleared tombstones (default: live ones only).
    #[serde(default)]
    pub include_cleared: bool,
}

/// `GET /admin/deletion-receipts` — one page of the receipt chain, in `seq`
/// order. Answers 403 for anything but a super-admin token.
///
/// # Errors
///
/// Returns a database error when the receipts cannot be read.
pub async fn handle_list_deletion_receipts(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(query): Query<ReceiptPageQuery>,
) -> AppResult<Response> {
    if let Some(denied) = deny_if_not_super_admin(&admin_token) {
        return Ok(denied);
    }
    let limit = query
        .limit
        .unwrap_or(MAX_RECEIPTS_PAGE)
        .clamp(1, MAX_RECEIPTS_PAGE);
    let receipts = context
        .repos
        .account_deletion
        .list_deletion_receipts(query.after_seq.max(0), limit)
        .await?;
    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("{} deletion receipts", receipts.len()),
            data: to_value(json!({ "receipts": receipts, "limit": limit })).ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}

/// `GET /admin/import-tombstones` — the newest tombstones, live ones only
/// unless `include_cleared`. Answers 403 for anything but a super-admin
/// token.
///
/// # Errors
///
/// Returns a database error when the tombstones cannot be read.
pub async fn handle_list_import_tombstones(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(query): Query<TombstoneListQuery>,
) -> AppResult<Response> {
    if let Some(denied) = deny_if_not_super_admin(&admin_token) {
        return Ok(denied);
    }
    let tombstones = context
        .repos
        .import_tombstones
        .list(query.include_cleared, MAX_TOMBSTONES_LISTED)
        .await?;
    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("{} import tombstones", tombstones.len()),
            data: to_value(json!({ "tombstones": tombstones })).ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}

/// Append one audit row for a tombstone clear.
async fn record_clear(
    context: &AdminApiContext,
    admin_token: &ValidatedAdminToken,
    tombstone_id: &str,
    failure: Option<&AppError>,
) -> AppResult<()> {
    context
        .repos
        .admin
        .record_token_usage(&AdminTokenUsage {
            id: None,
            admin_token_id: admin_token.token_id.clone(),
            timestamp: Utc::now(),
            action: AdminAction::ClearImportTombstone,
            target_resource: Some(format!("import_tombstone:{tombstone_id}")),
            ip_address: None,
            user_agent: None,
            request_size_bytes: None,
            success: failure.is_none(),
            error_message: failure.map(AppError::internal_details),
            response_time_ms: None,
        })
        .await
}

/// `POST /admin/import-tombstones/{id}/clear` — clear one tombstone.
///
/// Cleared as this operator, so its identity is accepted again. Idempotent:
/// one already cleared is answered as its first clearing left it. Audited
/// before it runs. Answers 403 for anything but a super-admin token.
///
/// # Errors
///
/// Returns a not-found error for an unknown tombstone, and a database error
/// when the audit row cannot be written or the clear fails.
pub async fn handle_clear_import_tombstone(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(tombstone_id): Path<String>,
) -> AppResult<Response> {
    if let Some(denied) = deny_if_not_super_admin(&admin_token) {
        return Ok(denied);
    }
    record_clear(&context, &admin_token, &tombstone_id, None).await?;
    let cleared = match clear_by_operator(
        context.repos.import_tombstones.as_ref(),
        &tombstone_id,
        &admin_token.token_id,
    )
    .await
    {
        Ok(cleared) => cleared,
        Err(e) => {
            if let Err(audit) = record_clear(&context, &admin_token, &tombstone_id, Some(&e)).await
            {
                error!(
                    tombstone_id = %tombstone_id,
                    token_id = %admin_token.token_id,
                    error = %audit.internal_details(),
                    "Could not record the failed tombstone clear"
                );
            }
            return Err(e);
        }
    };
    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("Tombstone {} cleared", cleared.id),
            data: to_value(json!({ "tombstone": cleared })).ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}
