// ABOUTME: Operator route that deletes every row one provider contributed, in every tenant
// ABOUTME: Super-admin only and audited: the termination purge a provider's API terms can require

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The whole-provider purge.
//!
//! A provider that terminates API access can require every copy of its data
//! deleted (WHOOP API Terms §7). An athlete's own disconnect already deletes
//! that athlete's rows through the disconnect chokepoint; this route deletes
//! the provider's rows for every user in every tenant at once, which is why it
//! is the one deliberately cross-tenant delete: see the isolation exemption
//! on [`ProviderDataRepository::purge_provider_data`].
//!
//! It deletes data, not connections: a user still connected to the provider
//! keeps syncing, so the answer reports how many connections remain.
//! Disconnect them first when the purge has to hold.
//!
//! Every call writes an `admin_token_usage` row naming the token and the
//! provider before anything is deleted, so no purge runs unrecorded; a purge
//! that then fails adds a failure row.
//!
//! [`ProviderDataRepository::purge_provider_data`]: pierre_database::repositories::ProviderDataRepository::purge_provider_data

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension,
};
use chrono::Utc;
use serde_json::{json, to_value};
use tracing::{error, info};

use pierre_core::admin::models::{AdminAction, AdminTokenUsage, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult};

use super::api_keys::json_response;
use super::strava_pool::deny_if_not_super_admin;
use super::types::AdminResponse;
use crate::context::AdminApiContext;

/// The longest provider name the purge accepts.
const MAX_PROVIDER_NAME_LEN: usize = 64;

/// The provider name as stored in every `provider` column: lowercase ASCII
/// letters, digits and `_`. Anything else cannot match a row, so it is
/// refused rather than reported as a purge that found nothing.
fn provider_name(raw: &str) -> AppResult<&str> {
    let valid = !raw.is_empty()
        && raw.len() <= MAX_PROVIDER_NAME_LEN
        && raw
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if valid {
        Ok(raw)
    } else {
        Err(AppError::invalid_input(format!(
            "Invalid provider name '{raw}': expected lowercase letters, digits and '_'"
        )))
    }
}

/// Append one audit row for this purge.
async fn record_purge(
    context: &AdminApiContext,
    admin_token: &ValidatedAdminToken,
    provider: &str,
    failure: Option<&AppError>,
) -> AppResult<()> {
    context
        .repos
        .admin
        .record_token_usage(&AdminTokenUsage {
            id: None,
            admin_token_id: admin_token.token_id.clone(),
            timestamp: Utc::now(),
            action: AdminAction::PurgeProviderData,
            target_resource: Some(format!("provider:{provider}")),
            ip_address: None,
            user_agent: None,
            request_size_bytes: None,
            success: failure.is_none(),
            error_message: failure.map(AppError::internal_details),
            response_time_ms: None,
        })
        .await
}

/// `DELETE /admin/providers/{provider}/data` — delete every row `provider`
/// contributed, for every user in every tenant.
///
/// Answers 403 for anything but a super-admin token, 400 for a malformed
/// provider name, and otherwise the rows removed per table, their total, and
/// how many connections to the provider remain (they keep syncing).
///
/// # Errors
///
/// Returns a database error when the connections cannot be counted, the
/// audit row cannot be written, or the purge fails; nothing is deleted in
/// any of those cases.
pub async fn handle_purge_provider_data(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(provider): Path<String>,
) -> AppResult<Response> {
    if let Some(denied) = deny_if_not_super_admin(&admin_token) {
        return Ok(denied);
    }
    let provider = provider_name(&provider)?;

    // The purge deletes data, never tokens, so the connections that will
    // sync again are the same before and after it: counted first, a failed
    // count refuses the purge rather than failing one that already ran.
    let connections_remaining = context
        .repos
        .sync_cursors
        .list_connected_provider_users(provider)
        .await?
        .len();

    record_purge(&context, &admin_token, provider, None).await?;
    let purge = match context
        .repos
        .provider_data
        .purge_provider_data(provider)
        .await
    {
        Ok(purge) => purge,
        Err(e) => {
            if let Err(audit) = record_purge(&context, &admin_token, provider, Some(&e)).await {
                error!(
                    provider = %provider,
                    service = %admin_token.service_name,
                    error = %audit.internal_details(),
                    "Could not record the failed provider data purge"
                );
            }
            return Err(e);
        }
    };

    let total = purge.total();
    info!(
        provider = %provider,
        service = %admin_token.service_name,
        removed = total,
        rows_removed = ?purge.rows_removed,
        connections_remaining,
        "Operator purged every row a provider contributed, across every tenant"
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("Deleted {total} {provider} rows across every tenant"),
            data: to_value(json!({
                "provider": provider,
                "rows_removed": purge.rows_removed,
                "total_removed": total,
                "connections_remaining": connections_remaining,
            }))
            .ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}
