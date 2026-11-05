// ABOUTME: GET /admin/audit-events and /api/admin/audit-events — the admin audit trail, newest first
// ABOUTME: Tenant-scoped for a tenant admin, platform-wide for a super-admin; cursor-paged by timestamp

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Reading the admin audit trail (carnet#1391).
//!
//! What operators did through the console and `pierre-cli`, as the recorder
//! in [`crate::audit`] wrote it. The console's Audit page and
//! `pierre-cli audit list` read it here.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, to_value};
use uuid::Uuid;

use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_database::repositories::{AuditEventFilter, AuditOutcome, AUDIT_PAGE_MAX};

use super::api_keys::json_response;
use super::types::AdminResponse;
use super::users::deny_without_manage_users;
use crate::context::AdminApiContext;

/// The rows one page returns when the request names no limit.
const DEFAULT_PAGE: u32 = 50;

/// What a trail read asks for.
#[derive(Debug, Deserialize, Default)]
pub struct AuditTrailQuery {
    /// At most this many rows (capped at the trail's page maximum).
    pub limit: Option<u32>,
    /// Only rows older than this instant: the previous page's last `occurred_at`.
    pub before: Option<DateTime<Utc>>,
    /// Only actions starting with this text (`user.`, `DELETE`).
    pub action: Option<String>,
    /// Only `success`, `refused` or `failed`.
    pub outcome: Option<String>,
    /// Only this operator's actions.
    pub actor_user_id: Option<Uuid>,
}

fn parse_outcome(raw: &str) -> AppResult<AuditOutcome> {
    match raw {
        "success" => Ok(AuditOutcome::Success),
        "refused" => Ok(AuditOutcome::Refused),
        "failed" => Ok(AuditOutcome::Failed),
        other => Err(AppError::invalid_input(format!(
            "outcome must be success, refused or failed, not {other:?}"
        ))),
    }
}

/// `GET /admin/audit-events` — the trail, newest first.
///
/// A super-admin reads every tenant's rows; any other admin reads only the
/// rows of the tenant its credential acts in.
///
/// # Errors
/// Returns an invalid-input error for an unknown outcome, a permission error
/// for a credential without `ManageUsers` or a tenant, and a database error
/// when the read fails.
pub async fn handle_list_audit_events(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(query): Query<AuditTrailQuery>,
) -> AppResult<Response> {
    if let Some(denied) = deny_without_manage_users(&admin_token) {
        return Ok(denied.into_response());
    }
    let tenant_id = if admin_token.is_super_admin {
        None
    } else {
        Some(admin_token.tenant_id.clone().ok_or_else(|| {
            AppError::new(
                ErrorCode::PermissionDenied,
                "A tenant admin's credential names no tenant to read the audit trail of",
            )
        })?)
    };
    let filter = AuditEventFilter {
        tenant_id,
        before: query.before,
        action_prefix: query.action.filter(|a| !a.trim().is_empty()),
        outcome: query.outcome.as_deref().map(parse_outcome).transpose()?,
        actor_user_id: query.actor_user_id,
    };
    let limit = query.limit.unwrap_or(DEFAULT_PAGE).clamp(1, AUDIT_PAGE_MAX);
    let events = context.repos.admin_audit.list(&filter, limit).await?;
    let next_before = if events.len() == limit as usize {
        events.last().map(|e| e.occurred_at)
    } else {
        None
    };
    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("{} audit events", events.len()),
            data: to_value(json!({ "events": events, "next_before": next_before })).ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_three_outcomes_parse() {
        assert_eq!(
            parse_outcome("refused").ok(),
            Some(AuditOutcome::Refused),
            "bug: a stored outcome must parse"
        );
        assert!(
            parse_outcome("ok").is_err(),
            "bug: an unknown outcome must be refused, not read as no filter"
        );
    }
}
