// ABOUTME: The RFC 6749 §5.2 error bodies the password and refresh-token grant answers with
// ABOUTME: A refusal is a 400 the client can act on; a server fault is a 500 server_error without internal detail
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use pierre_auth::dto::auth::OAuth2ErrorResponse;
use pierre_core::errors::{AppError, ErrorCode};

/// An RFC 6749 §5.2 error body for a request the handler refused before any
/// grant ran.
pub fn oauth2_error(error: &str, description: &str) -> Response {
    let error_response = OAuth2ErrorResponse {
        error: error.to_owned(),
        error_description: Some(description.to_owned()),
    };
    (StatusCode::BAD_REQUEST, Json(error_response)).into_response()
}

/// The token response for a grant that failed.
///
/// A refused credential or account is a 400 naming the refusal; a server
/// fault is a 500 `server_error`, so a client retries rather than discards a
/// session that is still good, and the database or signer detail stays in the
/// log.
pub fn grant_error_response(e: AppError) -> Response {
    let error_code = match e.code {
        ErrorCode::AuthInvalid | ErrorCode::AuthRequired | ErrorCode::AuthExpired => {
            "invalid_grant"
        }
        ErrorCode::PermissionDenied | ErrorCode::AccountPending | ErrorCode::AccountSuspended => {
            "access_denied"
        }
        ErrorCode::InvalidInput | ErrorCode::InvalidFormat => "invalid_request",
        _ if e.is_server_fault() => "server_error",
        // Anything else the caller brought (the account behind a refresh
        // token is gone) is a grant this server will not honour.
        _ => "invalid_grant",
    };
    let (status, error_desc) = if e.is_server_fault() {
        (StatusCode::INTERNAL_SERVER_ERROR, e.sanitized_message())
    } else {
        (StatusCode::BAD_REQUEST, e.message)
    };
    let error_response = OAuth2ErrorResponse {
        error: error_code.to_owned(),
        error_description: Some(error_desc),
    };
    (status, Json(error_response)).into_response()
}
