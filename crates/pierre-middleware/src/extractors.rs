// ABOUTME: Axum extractors for common authentication patterns in route handlers
// ABOUTME: Eliminates duplicated auth extraction code by providing FromRequestParts implementations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::convert::Infallible;
use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::ops::Deref;
use std::sync::Arc;

use axum::extract::{ConnectInfo, FromRequestParts};
use axum::http::request::Parts;
use axum::http::HeaderMap;
use uuid::Uuid;

use pierre_auth::auth::AuthResult;
use pierre_auth::security::cookies::get_cookie_value;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_runtime_context::MiddlewareCtx;

/// Axum extractor that authenticates a user from the `Authorization` header or `auth_token` cookie.
///
/// Tries the `Authorization` header first, then falls back to the `auth_token` cookie.
/// Returns the full [`AuthResult`] including `user_id`, `auth_method`,
/// and `active_tenant_id`. A delegated OAuth grant is refused with 403: see
/// [`extract_auth_from_headers`].
///
/// # Usage
///
/// Add `auth: AuthenticatedUser` as a handler parameter alongside `State(resources)`.
/// The extractor reads from request headers automatically:
///
/// - `auth.user_id` — the authenticated user's UUID
/// - `auth.into_inner()` — consume the extractor to get the full [`AuthResult`]
pub struct AuthenticatedUser(pub AuthResult);

impl Deref for AuthenticatedUser {
    type Target = AuthResult;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AuthenticatedUser {
    /// Consume the extractor and return the inner [`AuthResult`]
    #[must_use]
    pub fn into_inner(self) -> AuthResult {
        self.0
    }
}

impl<C: MiddlewareCtx> FromRequestParts<Arc<C>> for AuthenticatedUser {
    type Rejection = AppError;

    fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<C>,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send {
        let headers = parts.headers.clone();
        let state = state.clone(); // Safe: Arc clone for async block
        async move {
            let auth_result = extract_auth_from_headers(&headers, &state).await?;
            Ok(Self(auth_result))
        }
    }
}

/// Extract and authenticate user from `Authorization` header or `auth_token` cookie.
///
/// Shared logic used by the [`AuthenticatedUser`] extractor. Tries the `Authorization` header
/// first, then falls back to the `auth_token` cookie formatted as a Bearer token.
///
/// Only the athlete's own credential passes: every route behind this acts with the athlete's
/// whole authority and reads no scope, so a delegated OAuth grant is refused.
///
/// # Errors
///
/// Returns [`AppError`] if:
/// - No `Authorization` header or `auth_token` cookie is present
/// - The token is invalid or expired
/// - The token is a delegated OAuth grant (403 `PermissionDenied`, passed through unchanged)
/// - The user lookup or rate limit check fails
///
/// A spent request budget stays a 429 and a server-side failure keeps its
/// 5xx ([`AppError::into_auth_refusal`]); every other failure is a 401.
pub async fn extract_auth_from_headers<C: MiddlewareCtx>(
    headers: &HeaderMap,
    resources: &Arc<C>,
) -> Result<AuthResult, AppError> {
    let auth_value =
        if let Some(auth_header) = headers.get("authorization").and_then(|h| h.to_str().ok()) {
            auth_header.to_owned()
        } else if let Some(token) = get_cookie_value(headers, "auth_token") {
            format!("Bearer {token}")
        } else {
            return Err(AppError::auth_invalid(
                "Missing authorization header or cookie",
            ));
        };

    resources
        .authenticate_request(Some(&auth_value))
        .await
        .map_err(|e| {
            // A refused delegation is a genuine credential used where it does not
            // apply: 403, carrying the sentence that says where it does. Folded
            // into a 401 it would send the application back to re-authenticate
            // for a token that would be refused again.
            if e.code == ErrorCode::PermissionDenied {
                e
            } else {
                e.into_auth_refusal("Authentication failed")
            }
        })
}

/// Convenience function to get `user_id` from an [`AuthenticatedUser`].
///
/// Equivalent to `auth.user_id` — provided for handlers that only need the user ID.
#[must_use]
pub fn auth_user_id(auth: &AuthenticatedUser) -> Uuid {
    auth.user_id
}

/// The TCP peer of a request, when the server was served with `ConnectInfo`.
///
/// Read through axum's own `ConnectInfo` extractor, so a router that supplies
/// the peer with `MockConnectInfo` reads the same way. Never rejects: a router
/// driven with neither reads as no peer. The peer is a proxy's address behind
/// the deployed chain; `TrustedProxies::client_address` turns it and
/// `X-Forwarded-For` into the client's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeerAddress(pub Option<IpAddr>);

impl<S: Send + Sync> FromRequestParts<S> for PeerAddress {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self(
            ConnectInfo::<SocketAddr>::from_request_parts(parts, state)
                .await
                .ok()
                .map(|ConnectInfo(addr)| addr.ip()),
        ))
    }
}
