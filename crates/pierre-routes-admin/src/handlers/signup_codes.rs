// ABOUTME: Admin signup-code route handlers — list every code, mint an admin code, revoke a code (admin or coach)
// ABOUTME: One handler per operation, mounted for pierre-cli's token and the console's session alike (carnet#1128)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde::Deserialize;
use serde_json::{json, to_value};
use tracing::info;
use uuid::Uuid;

use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::errors::{AppError, AppResult};
use pierre_services::signup_codes::{self, CodeKind, MintRequest};

use super::admin_scope::require_platform_reach;
use super::api_keys::json_response;
use super::types::AdminResponse;
use super::users::deny_without_manage_users;
use crate::context::AdminApiContext;

/// Body of `POST /admin/signup-codes`.
#[derive(Debug, Default, Deserialize)]
pub struct MintSignupCodeRequest {
    /// The cohort or reason, shown in the listing
    #[serde(default)]
    pub note: Option<String>,
    /// A readable prefix in front of the random part
    #[serde(default)]
    pub prefix: Option<String>,
    /// How many accounts it may approve; absent is unlimited
    #[serde(default)]
    pub max_uses: Option<i32>,
    /// Days until it stops working; absent never expires
    #[serde(default)]
    pub expires_in_days: Option<i64>,
}

/// `GET /admin/signup-codes` — every admin code and every coach's code, with
/// the accounts that came in on each.
///
/// A caller bound to one tenant is refused (403) on the three routes of
/// this file ([`require_platform_reach`]): a code lets in whoever redeems
/// it, whatever tenant that account is or will be in, and the list names
/// the accounts of every tenant.
pub(crate) async fn handle_list_signup_codes(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_manage_users(&admin_token) {
        return Ok(denied.into_response());
    }
    require_platform_reach(&admin_token, "sign-up codes are", "list them")?;
    let codes = signup_codes::list(&context.repos).await?;
    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("{} signup code(s)", codes.len()),
            data: to_value(json!({ "codes": codes, "total": codes.len() })).ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}

/// `POST /admin/signup-codes` — mint an admin code. The code is in this
/// response and nowhere else: only its hash is stored. A caller bound to
/// one tenant mints none (403).
pub(crate) async fn handle_mint_signup_code(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Json(request): Json<MintSignupCodeRequest>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_manage_users(&admin_token) {
        return Ok(denied.into_response());
    }
    require_platform_reach(&admin_token, "a sign-up code is", "mint one")?;
    let minted = signup_codes::mint(
        &context.repos,
        admin_token.operator_user_id,
        &MintRequest {
            note: request.note,
            prefix: request.prefix,
            max_uses: request.max_uses,
            expires_in_days: request.expires_in_days,
        },
    )
    .await?;
    info!(code_id = %minted.record.id, token_id = %admin_token.token_id, "Signup code minted by an admin");
    Ok(json_response(
        AdminResponse {
            success: true,
            message: "Signup code minted. Copy it now: it is not shown again.".to_owned(),
            data: to_value(json!({ "code": minted.code, "record": minted.record })).ok(),
        },
        StatusCode::CREATED,
    )
    .into_response())
}

/// `DELETE /admin/signup-codes/{kind}/{id}` — withdraw an admin code
/// (`kind` = `admin`) or a coach's code (`coach`). Accounts it already
/// approved stay approved. A caller bound to one tenant withdraws none
/// (403), the code left live.
pub(crate) async fn handle_revoke_signup_code(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path((kind, id)): Path<(String, String)>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_manage_users(&admin_token) {
        return Ok(denied.into_response());
    }
    require_platform_reach(&admin_token, "a sign-up code is", "revoke one")?;
    let kind = match kind.as_str() {
        "admin" => CodeKind::Admin,
        "coach" => CodeKind::Coach,
        _ => return Err(AppError::invalid_input("kind must be admin or coach")),
    };
    let id = Uuid::parse_str(&id).map_err(|_| AppError::invalid_input("Invalid code id"))?;
    let revoked = signup_codes::revoke(&context.repos, kind, id).await?;
    info!(code_id = %id, revoked, token_id = %admin_token.token_id, "Signup code revoke requested");
    Ok(json_response(
        AdminResponse {
            success: true,
            message: if revoked {
                "Signup code revoked. Accounts it already approved stay approved.".to_owned()
            } else {
                "That signup code was already revoked (no change)".to_owned()
            },
            data: to_value(json!({ "id": id, "revoked": revoked })).ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}
