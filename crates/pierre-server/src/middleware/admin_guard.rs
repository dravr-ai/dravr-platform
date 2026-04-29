// ABOUTME: Central admin authorization guard for routes requiring admin privileges
// ABOUTME: Verifies user has admin role and returns 403 Forbidden if not authorized
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin Authorization Guard
//!
//! This module provides centralized admin authorization checking for route handlers.
//! Instead of each handler performing inline `user.role.is_admin_or_higher()` checks,
//! handlers can use the `require_admin` helper function.
//!
//! # Usage
//!
//! ```rust,no_run
//! use pierre_auth::auth::AuthResult;
//! use pierre_database::database::repositories::UserRepository;
//! use pierre_mcp_server::middleware::admin_guard::require_admin;
//! use std::sync::Arc;
//!
//! async fn admin_handler(
//!     auth: AuthResult,
//!     users: Arc<dyn UserRepository>,
//! ) -> Result<String, pierre_mcp_server::errors::AppError> {
//!     let admin_user = require_admin(auth.user_id, &users).await?;
//!     Ok(format!("Welcome admin: {}", admin_user.email))
//! }
//! ```

use crate::admin::models::{AdminPermissions, ValidatedAdminToken};
use crate::errors::{AppError, ErrorCode};
use crate::mcp::resources::ServerResources;
use crate::middleware::extractors::extract_auth_from_headers;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use pierre_core::models::User;
use pierre_database::database::repositories::UserRepository;
use std::sync::Arc;
use uuid::Uuid;

/// Require admin privileges for a user
///
/// Verifies that the authenticated user has admin role (admin or `super_admin`).
/// Returns the User record if authorized, or 403 Forbidden if not.
///
/// # Arguments
///
/// * `user_id` - The authenticated user's ID (from `AuthResult.user_id`)
/// * `users` - User repository for user lookup
///
/// # Errors
///
/// Returns an error if:
/// - User not found in database
/// - Database query fails
/// - User does not have admin role (returns 403 Forbidden)
///
/// # Example
///
/// ```rust,no_run
/// use pierre_auth::auth::AuthResult;
/// use pierre_mcp_server::middleware::admin_guard::require_admin;
///
/// # async fn example(auth: AuthResult, users: std::sync::Arc<dyn pierre_database::database::repositories::UserRepository>) -> Result<(), pierre_mcp_server::errors::AppError> {
/// let admin = require_admin(auth.user_id, &users).await?;
/// println!("Admin {} authorized", admin.email);
/// # Ok(())
/// # }
/// ```
pub async fn require_admin(
    user_id: Uuid,
    users: &Arc<dyn UserRepository>,
) -> Result<User, AppError> {
    // SECURITY: Global lookup — admin guard runs before tenant context is resolved
    let user = users
        .get_global(user_id)
        .await
        .map_err(|e| AppError::internal(format!("Failed to get user: {e}")))?
        .ok_or_else(|| AppError::not_found("User not found"))?;

    if !user.role.is_admin_or_higher() {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Admin privileges required",
        ));
    }

    Ok(user)
}

/// Axum middleware that gates web admin routes behind cookie/session auth.
///
/// Used by the human-facing admin tabs mounted at `/api/admin/...` so the
/// same routes that programmatic clients hit at `/admin/...` (with
/// admin-token JWT) are reachable from a logged-in admin in the browser
/// without a separate token. Returns 401 if no valid session is found,
/// 403 if the user is not an admin.
///
/// # Errors
///
/// Returns the inner `AppError` rendered as an HTTP response when:
/// - The session token is missing, invalid, or expired
/// - The looked-up user is not present
/// - The user's role is not `admin` or `super_admin`
pub async fn cookie_admin_middleware(
    State(resources): State<Arc<ServerResources>>,
    headers: HeaderMap,
    mut request: Request<Body>,
    next: Next,
) -> Result<Response, Response> {
    let auth = extract_auth_from_headers(&headers, &resources)
        .await
        .map_err(IntoResponse::into_response)?;
    let user = require_admin(auth.user_id, &resources.repos.users)
        .await
        .map_err(IntoResponse::into_response)?;
    // Synthesize a `ValidatedAdminToken` so handlers downstream that
    // expect `Extension<ValidatedAdminToken>` (set by the programmatic
    // `admin_auth_middleware`) work identically when the request comes
    // through cookie auth. Web admins always get full permissions —
    // the gate is the `is_admin` check above.
    request.extensions_mut().insert(ValidatedAdminToken {
        token_id: format!("cookie:{}", user.id),
        service_name: user.email.clone(),
        permissions: AdminPermissions::super_admin(),
        is_super_admin: true,
        tenant_id: auth.active_tenant_id.map(|t| t.to_string()),
        user_info: None,
    });
    Ok(next.run(request).await)
}
