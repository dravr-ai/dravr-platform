// ABOUTME: Axum extractors for common authentication patterns in route handlers
// ABOUTME: Eliminates duplicated auth extraction code by providing FromRequestParts implementations
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::ops::Deref;
use std::sync::Arc;

use async_trait::async_trait;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::HeaderMap;
use uuid::Uuid;

use crate::auth::AuthResult;
use crate::errors::AppError;
use crate::mcp::resources::ServerResources;
use crate::security::cookies::get_cookie_value;

/// Axum extractor that authenticates a user from the `Authorization` header or `auth_token` cookie.
///
/// Tries the `Authorization` header first, then falls back to the `auth_token` cookie.
/// Returns the full [`AuthResult`] including `user_id`, `auth_method`, `rate_limit`,
/// and `active_tenant_id`.
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

#[async_trait]
impl FromRequestParts<Arc<ServerResources>> for AuthenticatedUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &Arc<ServerResources>,
    ) -> Result<Self, Self::Rejection> {
        let auth_result = extract_auth_from_headers(&parts.headers, state).await?;
        Ok(Self(auth_result))
    }
}

/// Extract and authenticate user from `Authorization` header or `auth_token` cookie.
///
/// Shared logic used by the [`AuthenticatedUser`] extractor. Tries the `Authorization` header
/// first, then falls back to the `auth_token` cookie formatted as a Bearer token.
///
/// # Errors
///
/// Returns [`AppError`] if:
/// - No `Authorization` header or `auth_token` cookie is present
/// - The token is invalid or expired
/// - The user lookup or rate limit check fails
pub async fn extract_auth_from_headers(
    headers: &HeaderMap,
    resources: &Arc<ServerResources>,
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
        .auth_middleware
        .authenticate_request(Some(&auth_value))
        .await
        .map_err(|e| AppError::auth_invalid(format!("Authentication failed: {e}")))
}

/// Convenience function to get `user_id` from an [`AuthenticatedUser`].
///
/// Equivalent to `auth.user_id` — provided for handlers that only need the user ID.
#[must_use]
pub fn auth_user_id(auth: &AuthenticatedUser) -> Uuid {
    auth.user_id
}
