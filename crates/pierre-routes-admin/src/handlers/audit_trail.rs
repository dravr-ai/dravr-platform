// ABOUTME: GET /admin/audit-events and /api/admin/audit-events — the admin audit trail, newest first
// ABOUTME: Platform-wide for an admin or a super-admin token, tenant-scoped for a scoped token; keyset-paged by (timestamp, id)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Reading the admin audit trail (carnet#1391).
//!
//! What admins did through the console and `pierre-cli`, as the recorder
//! in [`crate::audit`] wrote it. The console's Audit page and
//! `pierre-cli audit list` read it here.

use std::sync::Arc;

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension,
};
use dravr_tronc::dates::Timestamp;
use serde::Deserialize;
use serde_json::{json, to_value};
use uuid::Uuid;

use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::errors::{AppError, AppResult};
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
    /// Resume after this instant: the previous page's `next_before`.
    pub before: Option<Timestamp>,
    /// Resume after this row among those of that instant: the previous page's
    /// `next_before_id`. Without it the read starts strictly older.
    pub before_id: Option<String>,
    /// Only actions starting with this text (`user.`, `DELETE`).
    pub action: Option<String>,
    /// Only `success`, `refused`, `failed` or `started` (an admin-only tool
    /// call whose outcome could not be written after it ran).
    pub outcome: Option<String>,
    /// Only this admin's actions.
    pub actor_user_id: Option<Uuid>,
}

fn parse_outcome(raw: &str) -> AppResult<AuditOutcome> {
    match raw {
        "success" => Ok(AuditOutcome::Success),
        "refused" => Ok(AuditOutcome::Refused),
        "failed" => Ok(AuditOutcome::Failed),
        "started" => Ok(AuditOutcome::Started),
        other => Err(AppError::invalid_input(format!(
            "outcome must be success, refused, failed or started, not {other:?}"
        ))),
    }
}

/// `GET /admin/audit-events` — the trail, newest first.
///
/// An admin's console session, like a super-admin token, reads every tenant's
/// rows; a scoped token reads only the rows of the tenant it acts in.
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
    // What the credential reaches: every tenant, its own, or none (the
    // scope rule's one reading, carnet#1589).
    let tenant_id = admin_token
        .reach()?
        .tenant_filter()
        .map(|tenant| tenant.to_string());
    let filter = AuditEventFilter {
        tenant_id,
        before: query.before,
        before_id: query.before_id,
        action_prefix: query.action.filter(|a| !a.trim().is_empty()),
        outcome: query.outcome.as_deref().map(parse_outcome).transpose()?,
        actor_user_id: query.actor_user_id,
    };
    let limit = query.limit.unwrap_or(DEFAULT_PAGE).clamp(1, AUDIT_PAGE_MAX);
    let events = context.repos.admin_audit.list(&filter, limit).await?;
    let last = if events.len() == limit as usize {
        events.last()
    } else {
        None
    };
    let next_before = last.map(|e| e.occurred_at);
    let next_before_id = last.map(|e| e.id.clone());
    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("{} audit events", events.len()),
            data: to_value(json!({
                "events": events,
                "next_before": next_before,
                "next_before_id": next_before_id,
            }))
            .ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_stored_outcomes_parse() {
        for outcome in [
            AuditOutcome::Success,
            AuditOutcome::Refused,
            AuditOutcome::Failed,
            AuditOutcome::Started,
        ] {
            assert_eq!(
                parse_outcome(outcome.as_str()).ok(),
                Some(outcome),
                "bug: a stored outcome must parse"
            );
        }
        assert!(
            parse_outcome("ok").is_err(),
            "bug: an unknown outcome must be refused, not read as no filter"
        );
    }
}
