// ABOUTME: RFC 8628 Device Authorization Grant endpoints backing `pierre-cli auth login`
// ABOUTME: authorization + token are public (the CLI isn't authenticated yet); approve is super-admin-gated and mints the admin token
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Device Authorization Grant (RFC 8628) for the authenticated admin CLI.
//!
//! Flow:
//! 1. `POST /admin/device/authorization` (public) — the CLI gets a `device_code`
//!    + short `user_code` and starts polling.
//! 2. A super-admin approves via `POST /admin/device/approve` (super-admin admin
//!    token), naming the `user_code`. Dravr's admin authority is the token
//!    (there is no super-admin browser identity), so approval reuses
//!    `admin_auth_middleware` rather than a cookie session.
//! 3. `POST /admin/device/token` (public) — once approved, the CLI's next poll
//!    mints a fresh super-admin admin token (the same [`create_token`] path
//!    `pierre-cli token generate --super-admin` uses) and the row is deleted
//!    (single-use). The unguessable `device_code` is the bearer secret.
//!
//! [`create_token`]: pierre_database::repositories::AdminRepository::create_token

use std::env;
use std::fmt::Write as _;
use std::sync::Arc;

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::Utc;
use rand::RngCore;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tracing::info;

use pierre_core::admin::models::{CreateAdminTokenRequest, ValidatedAdminToken};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::DeviceAuthorization;

use super::api_keys::json_response;
use super::strava_pool::deny_if_not_super_admin;
use super::types::AdminResponse;
use crate::context::AdminApiContext;

/// Lifetime of a pending device authorization before it expires.
const DEVICE_CODE_TTL_SECS: i64 = 600;
/// Minimum seconds the CLI should wait between token polls (RFC 8628 `interval`).
const DEVICE_POLL_INTERVAL_SECS: i64 = 5;
/// The RFC 8628 grant type the token endpoint requires.
const DEVICE_GRANT_TYPE: &str = "urn:ietf:params:oauth:grant-type:device_code";

const STATUS_PENDING: &str = "pending";
const STATUS_APPROVED: &str = "approved";
const STATUS_DENIED: &str = "denied";

/// Unambiguous `user_code` alphabet — no vowels (avoids words) and no
/// look-alikes (0/O, 1/I), so the operator can read and type it reliably.
const USER_CODE_ALPHABET: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ23456789";

/// RFC 8628 §3.2 device-authorization response.
#[derive(Serialize)]
struct DeviceAuthorizationResponse {
    device_code: String,
    user_code: String,
    verification_uri: String,
    verification_uri_complete: String,
    expires_in: i64,
    interval: i64,
}

/// RFC 8628 §3.5 successful token response (an admin bearer token).
#[derive(Serialize)]
struct DeviceTokenResponse {
    access_token: String,
    token_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    subject: Option<String>,
}

/// RFC 8628 §3.5 error response (`authorization_pending`, `access_denied`, …).
#[derive(Serialize)]
struct DeviceErrorResponse {
    error: String,
    error_description: String,
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// 256-bit random `device_code`, hex-encoded — the bearer secret the CLI keeps.
fn generate_device_code() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    to_hex(&bytes)
}

/// Short human-typeable `user_code`, e.g. `WDJB-MJHT`, from [`USER_CODE_ALPHABET`].
fn generate_user_code() -> String {
    let mut bytes = [0u8; 8];
    rand::rng().fill_bytes(&mut bytes);
    let mut out = String::with_capacity(9);
    for (i, b) in bytes.iter().enumerate() {
        if i == 4 {
            out.push('-');
        }
        let idx = usize::from(*b) % USER_CODE_ALPHABET.len();
        out.push(USER_CODE_ALPHABET[idx] as char);
    }
    out
}

/// SHA-256 of the raw `device_code`, hex-encoded — only the hash is stored.
fn hash_device_code(device_code: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(device_code.as_bytes());
    to_hex(&hasher.finalize())
}

/// Build an [`AdminResponse`] JSON body as a concrete [`Response`] (so approval
/// branches unify with the `Response` returned by [`deny_if_not_super_admin`]).
fn admin_json(response: AdminResponse, status: StatusCode) -> Response {
    json_response(response, status).into_response()
}

fn device_error(status: StatusCode, error: &str, description: &str) -> Response {
    (
        status,
        Json(DeviceErrorResponse {
            error: error.to_owned(),
            error_description: description.to_owned(),
        }),
    )
        .into_response()
}

/// `POST /admin/device/authorization` — begin a device login (public).
///
/// Creates a pending authorization and returns the `device_code`, the short
/// `user_code` an operator approves, and the polling `interval`.
///
/// # Errors
/// Returns an error if the authorization cannot be persisted.
pub async fn handle_device_authorization(
    State(context): State<Arc<AdminApiContext>>,
) -> AppResult<Response> {
    let device_code = generate_device_code();
    let user_code = generate_user_code();
    let now = Utc::now().timestamp();

    let record = DeviceAuthorization {
        device_code_hash: hash_device_code(&device_code),
        user_code: user_code.clone(),
        status: STATUS_PENDING.to_owned(),
        approved_by: None,
        created_at: now,
        expires_at: now + DEVICE_CODE_TTL_SECS,
    };
    context
        .repos
        .oauth2_server
        .create_device_authorization(&record)
        .await?;

    let base = env::var("BASE_URL").unwrap_or_default();
    let verification_uri = format!("{base}/admin/device");
    let verification_uri_complete = format!("{verification_uri}?user_code={user_code}");

    info!(user_code = %user_code, "Device authorization started");

    Ok((
        StatusCode::OK,
        Json(DeviceAuthorizationResponse {
            device_code,
            user_code,
            verification_uri,
            verification_uri_complete,
            expires_in: DEVICE_CODE_TTL_SECS,
            interval: DEVICE_POLL_INTERVAL_SECS,
        }),
    )
        .into_response())
}

/// `POST /admin/device/token` — poll for the minted admin token (public).
///
/// Body: `{ grant_type, device_code }`. Returns `authorization_pending` until a
/// super-admin approves, then a fresh super-admin admin token (single-use: the
/// row is deleted so a duplicate poll can never mint twice).
///
/// # Errors
/// Returns an error only on a repository or token-minting failure; RFC 8628
/// protocol conditions are returned as `400` bodies, not `Err`.
pub async fn handle_device_token(
    State(context): State<Arc<AdminApiContext>>,
    Json(request): Json<Value>,
) -> AppResult<Response> {
    let grant_type = request.get("grant_type").and_then(Value::as_str);
    if grant_type != Some(DEVICE_GRANT_TYPE) {
        return Ok(device_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "grant_type must be urn:ietf:params:oauth:grant-type:device_code",
        ));
    }
    let device_code = request
        .get("device_code")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::invalid_input("device_code is required"))?;

    let hash = hash_device_code(device_code);
    let now = Utc::now().timestamp();

    let Some(record) = context
        .repos
        .oauth2_server
        .get_device_authorization_by_code_hash(&hash)
        .await?
    else {
        return Ok(device_error(
            StatusCode::BAD_REQUEST,
            "expired_token",
            "Unknown or already-consumed device_code",
        ));
    };

    if record.expires_at <= now {
        context
            .repos
            .oauth2_server
            .delete_device_authorization(&hash)
            .await?;
        return Ok(device_error(
            StatusCode::BAD_REQUEST,
            "expired_token",
            "The device_code has expired; start a new login",
        ));
    }

    match record.status.as_str() {
        STATUS_PENDING => Ok(device_error(
            StatusCode::BAD_REQUEST,
            "authorization_pending",
            "Waiting for a super-admin to approve this login",
        )),
        STATUS_DENIED => {
            context
                .repos
                .oauth2_server
                .delete_device_authorization(&hash)
                .await?;
            Ok(device_error(
                StatusCode::BAD_REQUEST,
                "access_denied",
                "The login request was denied",
            ))
        }
        STATUS_APPROVED => mint_approved_token(&context, &hash, record.approved_by).await,
        other => Err(AppError::internal(format!(
            "device authorization in unexpected state: {other}"
        ))),
    }
}

/// Single-use consume: delete the approved row, and only mint if we won the
/// delete (so concurrent polls can never mint two tokens), then mint a fresh
/// super-admin admin token attributed to the approver.
async fn mint_approved_token(
    context: &Arc<AdminApiContext>,
    hash: &str,
    approved_by: Option<String>,
) -> AppResult<Response> {
    let won = context
        .repos
        .oauth2_server
        .delete_device_authorization(hash)
        .await?;
    if !won {
        return Ok(device_error(
            StatusCode::BAD_REQUEST,
            "expired_token",
            "Unknown or already-consumed device_code",
        ));
    }

    let approver = approved_by.unwrap_or_else(|| "unknown".to_owned());
    let request = CreateAdminTokenRequest::super_admin(format!("device-cli:{approver}"));
    let minted = context
        .repos
        .admin
        .create_token(
            &request,
            &context.admin_jwt_secret,
            context.jwks_manager.as_ref(),
        )
        .await?;

    info!(
        approved_by = %approver,
        token_id = %minted.token_id,
        "Device login approved; minted super-admin admin token"
    );

    Ok((
        StatusCode::OK,
        Json(DeviceTokenResponse {
            access_token: minted.jwt_token,
            token_type: "Bearer".to_owned(),
            subject: Some(approver),
        }),
    )
        .into_response())
}

/// `POST /admin/device/approve` — approve or deny a device login (super-admin).
///
/// Body: `{ user_code, action? }` where `action` is `approve` (default) or
/// `deny`. Gated on a super-admin admin token; the approver's `service_name`
/// is recorded so the minted token is attributable.
///
/// # Errors
/// Returns an error if the lookup or status update fails.
pub async fn handle_device_approve(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Json(request): Json<Value>,
) -> AppResult<Response> {
    if let Some(denied) = deny_if_not_super_admin(&admin_token) {
        return Ok(denied);
    }

    let user_code = request
        .get("user_code")
        .and_then(Value::as_str)
        .ok_or_else(|| AppError::invalid_input("user_code is required"))?;
    let action = request
        .get("action")
        .and_then(Value::as_str)
        .unwrap_or("approve");

    let Some(record) = context
        .repos
        .oauth2_server
        .get_device_authorization_by_user_code(user_code)
        .await?
    else {
        return Ok(admin_json(
            AdminResponse {
                success: false,
                message: format!("Unknown user_code: {user_code}"),
                data: None,
            },
            StatusCode::NOT_FOUND,
        ));
    };

    if record.expires_at <= Utc::now().timestamp() {
        return Ok(admin_json(
            AdminResponse {
                success: false,
                message: "This device code has expired; ask the operator to start a new login"
                    .to_owned(),
                data: None,
            },
            StatusCode::GONE,
        ));
    }
    if record.status != STATUS_PENDING {
        return Ok(admin_json(
            AdminResponse {
                success: false,
                message: format!("Device login is already {}", record.status),
                data: None,
            },
            StatusCode::CONFLICT,
        ));
    }

    if action.eq_ignore_ascii_case("deny") {
        context
            .repos
            .oauth2_server
            .deny_device_authorization(user_code)
            .await?;
        info!(user_code = %user_code, approver = %admin_token.service_name, "Device login denied");
        return Ok(admin_json(
            AdminResponse {
                success: true,
                message: format!("Device login {user_code} denied"),
                data: None,
            },
            StatusCode::OK,
        ));
    }

    context
        .repos
        .oauth2_server
        .approve_device_authorization(user_code, &admin_token.service_name)
        .await?;
    info!(user_code = %user_code, approver = %admin_token.service_name, "Device login approved");

    Ok(admin_json(
        AdminResponse {
            success: true,
            message: format!("Device login {user_code} approved"),
            data: None,
        },
        StatusCode::OK,
    ))
}
