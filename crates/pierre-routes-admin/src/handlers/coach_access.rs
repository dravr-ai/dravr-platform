// ABOUTME: Admin handlers for the coach-access queue — list requests, grant or decline one (carnet#738)
// ABOUTME: Super-admin only, like the manages-roster grant they stand in for; logic in pierre_services::coach_access
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The console's coach-access queue.
//!
//! `GET /admin/coach-access-requests[?status=pending|granted|declined]`,
//! `POST /admin/coach-access-requests/{id}/grant` and
//! `POST /admin/coach-access-requests/{id}/decline`, mounted for the admin
//! token and, under `/api/admin/…`, for the console session. A grant is the
//! same super-admin decision as `POST /admin/users/{id}/manages-roster`, plus
//! attaching the coach to the onboarding group they asked from, so every
//! route here is super-admin only.

use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Extension,
};
use serde::Deserialize;
use serde_json::{json, to_value};
use tracing::info;
use uuid::Uuid;

use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::CoachAccessStatus;
use pierre_services::coach_access;

use super::api_keys::json_response;
use super::types::AdminResponse;
use crate::context::AdminApiContext;

/// Query of the queue listing.
#[derive(Debug, Default, Deserialize)]
pub(crate) struct CoachAccessListQuery {
    /// `pending` (the default), `granted` or `declined`.
    #[serde(default)]
    pub status: Option<String>,
}

/// The refusal every route here returns to a token that is not super-admin.
fn deny_without_super_admin(token: &ValidatedAdminToken) -> Option<impl IntoResponse> {
    (!token.is_super_admin).then(|| {
        json_response(
            AdminResponse {
                success: false,
                message: "Permission denied: super-admin token required".to_owned(),
                data: None,
            },
            StatusCode::FORBIDDEN,
        )
    })
}

fn parse_request_id(raw: &str) -> AppResult<Uuid> {
    Uuid::parse_str(raw)
        .map_err(|e| AppError::invalid_input(format!("Invalid coach access request id: {e}")))
}

/// `GET /admin/coach-access-requests` — the queue, oldest first.
pub(crate) async fn handle_list_coach_access_requests(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Query(query): Query<CoachAccessListQuery>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_super_admin(&admin_token) {
        return Ok(denied.into_response());
    }
    let status = match query.status.as_deref() {
        None | Some("") => CoachAccessStatus::Pending,
        Some(raw) => CoachAccessStatus::from_stored(raw).ok_or_else(|| {
            AppError::invalid_input(format!(
                "Unknown status: {raw}. Supported: pending, granted, declined"
            ))
        })?,
    };

    let requests = coach_access::list_requests(&context.repos, status).await?;
    info!(
        count = requests.len(),
        status = status.as_str(),
        token_id = %admin_token.token_id,
        "Coach access requests listed"
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("{} coach access request(s)", requests.len()),
            data: to_value(json!({
                "requests": requests,
                "total": requests.len(),
            }))
            .ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}

/// `POST /admin/coach-access-requests/{id}/grant` — grant coach access and
/// attach the coach to the group they asked from.
pub(crate) async fn handle_grant_coach_access_request(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(request_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_super_admin(&admin_token) {
        return Ok(denied.into_response());
    }
    let request_id = parse_request_id(&request_id)?;

    let outcome =
        coach_access::grant(&context.repos, request_id, admin_token.operator_user_id).await?;

    Ok(json_response(
        AdminResponse {
            success: true,
            message: if outcome.attached_group.is_some() {
                "Coach access granted; the coach now coaches their group".to_owned()
            } else {
                "Coach access granted".to_owned()
            },
            data: to_value(json!({
                "request": outcome.request,
                "attached_group_id": outcome.attached_group,
            }))
            .ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}

/// `POST /admin/coach-access-requests/{id}/decline` — decline a request.
pub(crate) async fn handle_decline_coach_access_request(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(request_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_super_admin(&admin_token) {
        return Ok(denied.into_response());
    }
    let request_id = parse_request_id(&request_id)?;

    let request =
        coach_access::decline(&context.repos, request_id, admin_token.operator_user_id).await?;

    Ok(json_response(
        AdminResponse {
            success: true,
            message: "Coach access request declined".to_owned(),
            data: to_value(json!({ "request": request })).ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}
