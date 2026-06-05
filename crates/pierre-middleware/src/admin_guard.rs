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
//! use pierre_middleware::admin_guard::require_admin;
//! use std::sync::Arc;
//!
//! async fn admin_handler(
//!     auth: AuthResult,
//!     users: Arc<dyn UserRepository>,
//! ) -> Result<String, pierre_core::errors::AppError> {
//!     let admin_user = require_admin(auth.user_id, &users).await?;
//!     Ok(format!("Welcome admin: {}", admin_user.email))
//! }
//! ```

use super::extractors::extract_auth_from_headers;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, Request};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use pierre_core::admin::models::{AdminPermissions, ValidatedAdminToken};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::User;
use pierre_database::database::repositories::UserRepository;
use pierre_runtime_context::MiddlewareCtx;
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
/// use pierre_middleware::admin_guard::require_admin;
///
/// # async fn example(auth: AuthResult, users: std::sync::Arc<dyn pierre_database::database::repositories::UserRepository>) -> Result<(), pierre_core::errors::AppError> {
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
pub async fn cookie_admin_middleware<C: MiddlewareCtx>(
    State(resources): State<Arc<C>>,
    headers: HeaderMap,
    mut request: Request<Body>,
    next: Next,
) -> Result<Response, Response> {
    let auth = extract_auth_from_headers(&headers, &resources)
        .await
        .map_err(IntoResponse::into_response)?;
    let user = require_admin(auth.user_id, &resources.repos().users)
        .await
        .map_err(IntoResponse::into_response)?;
    // Synthesize a `ValidatedAdminToken` so handlers downstream that
    // expect `Extension<ValidatedAdminToken>` (set by the programmatic
    // `admin_auth_middleware`) work identically when the request comes
    // through cookie auth.
    let admin_token = cookie_admin_token(&user, auth.active_tenant_id);
    request.extensions_mut().insert(admin_token);
    Ok(next.run(request).await)
}

/// Build the `ValidatedAdminToken` granted to a cookie-authenticated admin.
///
/// Permissions mirror the user's actual role so the granular
/// `require_permission`/`is_super_admin` checks inside cookie-mounted handlers
/// stay meaningful: a plain `Admin` gets only `default_admin` (key management),
/// while `SuperAdmin` gets the full set and `is_super_admin = true`. Granting
/// `super_admin` to every admin role would flatten RBAC and silently always-pass
/// the downstream gates (store moderation, admin-token management, impersonation).
///
/// This mirrors the programmatic admin-token regime, where a non-super admin
/// token also carries `default_admin` permissions — cookie and token auth now
/// grant identical authority for the same role.
#[must_use]
pub fn cookie_admin_token(user: &User, active_tenant_id: Option<Uuid>) -> ValidatedAdminToken {
    let is_super_admin = user.role.is_super_admin();
    let permissions = if is_super_admin {
        AdminPermissions::super_admin()
    } else {
        AdminPermissions::default_admin()
    };
    ValidatedAdminToken {
        token_id: format!("cookie:{}", user.id),
        service_name: user.email.clone(),
        permissions,
        is_super_admin,
        tenant_id: active_tenant_id.map(|t| t.to_string()),
        user_info: None,
    }
}
