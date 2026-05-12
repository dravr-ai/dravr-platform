// ABOUTME: Axum HTTP handlers for authentication endpoints (registration, login, session, etc.)
// ABOUTME: Thin wiring layer delegating business logic to services::auth::AuthService
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::{
    extract::{Form, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;
use tokio::task;
use tracing::{error, field, field::Empty, info, warn, Span};

use crate::{
    admin::AdminAuthService,
    constants::error_messages,
    errors::{AppError, ErrorCode},
    mcp::resources::ServerContext,
    middleware::extract_auth_from_headers,
    models::{CoachingPersona, UserStatus},
    utils::auth::extract_bearer_token_owned,
};
use pierre_auth::security::cookies::{clear_auth_cookie, set_auth_cookie, set_csrf_cookie};

use super::types::{
    AnalyticsConsentRequest, ChangePasswordRequest, CompleteResetRequest, FirebaseLoginRequest,
    LoginRequest, OAuth2ErrorResponse, OAuth2TokenRequest, OAuth2TokenResponse,
    RefreshTokenRequest, RegisterRequest, SessionResponse, UpdateCoachingPersonaRequest,
    UpdateLocaleRequest, UpdateProfileRequest, UpdateProfileResponse, UserInfo, UserStatsResponse,
};

use crate::services::analytics::{analytics, hash_id};
// Re-export AuthService from the service layer so existing `use crate::routes::auth::AuthService`
// paths continue to compile without changes across the codebase.
pub use crate::services::auth::AuthService;

// ---------------------------------------------------------------------------
// Axum handler functions — called from AuthRoutes::routes() in mod.rs
// ---------------------------------------------------------------------------

/// Handle user registration (admin-authenticated)
///
/// REQUIRES: Admin authentication (Bearer token in Authorization header)
///
/// Security: Only administrators can create new users to prevent
/// unauthorized user creation, database pollution, and `DoS` attacks.
#[tracing::instrument(
    skip(resources, headers, request),
    fields(route = "admin_register", user_id = Empty, success = Empty)
)]
pub(super) async fn handle_register(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Json(request): Json<RegisterRequest>,
) -> Result<Response, AppError> {
    // Extract and validate admin token
    let auth_header = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .ok_or_else(|| {
            AppError::auth_invalid(
                "Missing Authorization header for user registration - admin token required",
            )
        })?;

    let token = extract_bearer_token_owned(auth_header)
        .map_err(|_| AppError::auth_invalid("Invalid Authorization header format"))?;

    // Validate admin token
    let admin_auth_service = AdminAuthService::new(
        resources.repos.admin.clone(),
        resources.jwks_manager.clone(),
        resources.config.auth.admin_token_cache_ttl_secs,
    );

    // Authenticate admin (no specific permission check - any valid admin token can register users)
    admin_auth_service
        .authenticate(&token, None)
        .await
        .map_err(|e| {
            warn!(error = %e, "Failed to authenticate admin token for user registration");
            AppError::auth_invalid(format!("Admin authentication failed: {e}"))
        })?;

    info!("Admin-authenticated user registration attempt");

    let auth_service = AuthService::new(resources.auth(), resources.config(), resources.data());

    match auth_service.register(request.clone()).await {
        Ok(response) => {
            send_post_registration_email(&resources, &request.email, &response).await;
            Ok((StatusCode::CREATED, Json(response)).into_response())
        }
        Err(e) => {
            error!("Registration failed: {}", e);
            Err(e)
        }
    }
}

/// Fire-and-forget post-registration email dispatch
///
/// Sends a "pending review" email for new accounts in Pending status, or a
/// "welcome / account approved" email for accounts that were auto-approved at
/// registration time. Never fails the parent request — missing Resend config
/// or transient delivery errors are logged as warnings only.
async fn send_post_registration_email(
    resources: &ServerContext,
    email: &str,
    response: &super::types::RegisterResponse,
) {
    let Some(email_svc) = resources.email_service.as_ref() else {
        warn!(
            user_id = %response.user_id,
            "Email service not configured — skipping registration confirmation email"
        );
        return;
    };

    let display_name = response.display_name.as_deref();
    let result = if response.user_status == UserStatus::Active {
        let sign_in_url = resources.config.frontend_url.as_deref();
        email_svc
            .send_registration_approved(email, display_name, sign_in_url)
            .await
    } else {
        email_svc
            .send_registration_pending(email, display_name)
            .await
    };

    if let Err(e) = result {
        warn!(
            user_id = %response.user_id,
            error = %e,
            "Failed to send registration confirmation email — user not notified"
        );
    }
}

/// Handle public user self-registration
///
/// This endpoint allows users to register themselves without admin authentication.
/// New users are created in "Pending" status by default and require admin approval,
/// unless `AUTO_APPROVE_USERS` is true or the email domain is in `AUTO_APPROVE_DOMAINS`.
#[tracing::instrument(
    skip(resources, request),
    fields(route = "public_register", user_id = Empty, success = Empty)
)]
pub(super) async fn handle_public_register(
    State(resources): State<Arc<ServerContext>>,
    Json(request): Json<RegisterRequest>,
) -> Result<Response, AppError> {
    info!("Public self-registration attempt");

    let auth_service = AuthService::new(resources.auth(), resources.config(), resources.data());

    match auth_service.register(request.clone()).await {
        Ok(response) => {
            send_post_registration_email(&resources, &request.email, &response).await;
            Ok((StatusCode::CREATED, Json(response)).into_response())
        }
        Err(e) => {
            error!("Public registration failed: {}", e);
            Err(e)
        }
    }
}

/// Handle Firebase authentication login
///
/// Authenticates users via Firebase ID tokens (Google Sign-In, Apple, etc.)
#[tracing::instrument(
    skip(resources, request),
    fields(route = "firebase_login", user_id = Empty, auth_provider = Empty, success = Empty)
)]
pub(super) async fn handle_firebase_login(
    State(resources): State<Arc<ServerContext>>,
    Json(request): Json<FirebaseLoginRequest>,
) -> Result<Response, AppError> {
    // Check if Firebase is configured
    let firebase_auth = resources.firebase_auth.as_ref().ok_or_else(|| {
        AppError::invalid_input("Firebase authentication is not configured on this server")
    })?;

    let auth_service = AuthService::new(resources.auth(), resources.config(), resources.data());

    match auth_service
        .login_with_firebase(request, firebase_auth)
        .await
    {
        Ok(mut response) => {
            // Clone JWT for cookie (also included in JSON response for API clients)
            let jwt_token = response
                .jwt_token
                .clone() // Safe: JWT string ownership for cookie
                .ok_or_else(|| AppError::internal("JWT token missing from login response"))?;

            // Parse user ID for CSRF token generation
            let user_id = uuid::Uuid::parse_str(&response.user.user_id)
                .map_err(|e| AppError::internal(format!("Invalid user ID format: {e}")))?;

            // Generate CSRF token (stateless HMAC — no server storage)
            let csrf_token = resources
                .csrf_manager
                .generate_token(user_id)
                .map_err(|e| AppError::internal(format!("Failed to generate CSRF token: {e}")))?;

            // Set response CSRF token
            response.csrf_token.clone_from(&csrf_token);

            // Build response with secure cookies
            let mut headers = HeaderMap::new();

            // Set httpOnly auth cookie (24 hour expiry to match JWT)
            set_auth_cookie(&mut headers, &jwt_token, 24 * 60 * 60);

            // Set CSRF cookie (24 hour expiry to match CSRF token TTL)
            set_csrf_cookie(&mut headers, &csrf_token, 24 * 60 * 60);

            Ok((StatusCode::OK, headers, Json(response)).into_response())
        }
        Err(e) => {
            tracing::error!("Firebase login failed: {}", e);
            Err(e)
        }
    }
}

/// Handle token refresh
#[tracing::instrument(
    skip(resources, request),
    fields(route = "token_refresh", user_id = %request.user_id, success = Empty)
)]
pub(super) async fn handle_refresh(
    State(resources): State<Arc<ServerContext>>,
    Json(request): Json<RefreshTokenRequest>,
) -> Result<Response, AppError> {
    let auth_service = AuthService::new(resources.auth(), resources.config(), resources.data());

    match auth_service.refresh_token(request).await {
        Ok(mut response) => {
            // Clone JWT for cookie (also included in JSON response for API clients)
            let jwt_token = response
                .jwt_token
                .clone() // Safe: JWT string ownership for cookie
                .ok_or_else(|| AppError::internal("JWT token missing from refresh response"))?;

            // Parse user ID for CSRF token generation
            let user_id = uuid::Uuid::parse_str(&response.user.user_id)
                .map_err(|e| AppError::internal(format!("Invalid user ID format: {e}")))?;

            // Generate new CSRF token (stateless HMAC — no server storage)
            let csrf_token = resources
                .csrf_manager
                .generate_token(user_id)
                .map_err(|e| AppError::internal(format!("Failed to generate CSRF token: {e}")))?;

            // Set response CSRF token
            response.csrf_token.clone_from(&csrf_token);

            // Build response with secure cookies
            let mut headers = HeaderMap::new();

            // Set httpOnly auth cookie (24 hour expiry to match JWT)
            set_auth_cookie(&mut headers, &jwt_token, 24 * 60 * 60);

            // Set CSRF cookie (24 hour expiry to match CSRF token TTL)
            set_csrf_cookie(&mut headers, &csrf_token, 24 * 60 * 60);

            Ok((StatusCode::OK, headers, Json(response)).into_response())
        }
        Err(e) => {
            error!("Token refresh failed: {}", e);
            Err(e)
        }
    }
}

/// Handle user logout
pub(super) async fn handle_logout() -> Result<Response, AppError> {
    // Yield to allow async context (required for Axum handler)
    task::yield_now().await;

    // Build response with cleared cookies
    let mut headers = HeaderMap::new();
    clear_auth_cookie(&mut headers);

    Ok((
        StatusCode::OK,
        headers,
        Json(json!({ "message": "Logged out successfully" })),
    )
        .into_response())
}

/// Restore session from httpOnly cookie authentication
///
/// Returns the authenticated user's info along with a fresh JWT (for WebSocket auth)
/// and CSRF token. This allows the frontend to restore sessions on page refresh
/// without storing JWT tokens in localStorage.
#[tracing::instrument(
    skip(resources, headers),
    fields(route = "session", user_id = Empty, success = Empty)
)]
pub(super) async fn handle_session(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // Authenticate using cookie or Authorization header
    let auth_result = resources
        .auth_middleware
        .authenticate_request_with_headers(&headers)
        .await?;

    let user_id = auth_result.user_id;
    Span::current().record("user_id", user_id.to_string());

    // Look up user details from database
    let user = resources
        .repos
        .users
        .get_global(user_id)
        .await
        .map_err(|e| AppError::database(format!("Failed to fetch user: {e}")))?
        .ok_or_else(|| AppError::not_found(format!("User {user_id}")))?;

    // Warm the analytics consent cache from the durable user record so PostHog
    // events are not silently dropped after a Cloud Run cold start
    let hashed_user_id = hash_id(&user_id.to_string());
    analytics().hydrate_consent(&hashed_user_id, user.analytics_consent);

    // Preserve active_tenant_id from existing JWT, or look up user's default tenant
    let active_tenant_id = if let Some(tid) = auth_result.active_tenant_id {
        Some(tid.to_string())
    } else {
        let tenants = resources
            .repos
            .tenants
            .list_for_user(user_id)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user tenants: {e}")))?;
        tenants.first().map(|t| t.id.to_string())
    };
    let tenant_id_for_response = active_tenant_id.clone();

    // Phase 4: PostHog `$identify` on session restore so user properties
    // (tier, locale, signup-date) follow every subsequent event for this
    // user. Tier is sourced from the canonical `users.tier` column; the
    // tenant id is hashed so PostHog never sees raw IDs.
    let identify_props = serde_json::json!({
        "tier": user.tier.to_string(),
        "tenant_id_hash": tenant_id_for_response.as_deref().map(hash_id).unwrap_or_default(),
        "signup_date": user.created_at.to_rfc3339(),
        "primary_locale": user.locale.clone(),
        "analytics_consent": user.analytics_consent,
    });
    analytics().identify(&hashed_user_id, identify_props);

    // Generate a fresh JWT token for WebSocket authentication with active_tenant_id
    let jwt_token = resources
        .auth_manager
        .generate_token_with_tenant(&user, &resources.jwks_manager, active_tenant_id)
        .map_err(|e| AppError::auth_invalid(format!("Failed to generate token: {e}")))?;

    // Generate fresh CSRF token (stateless HMAC — no server storage)
    let csrf_token = resources
        .csrf_manager
        .generate_token(user_id)
        .map_err(|e| AppError::internal(format!("Failed to generate CSRF token: {e}")))?;

    // Refresh the httpOnly auth cookie with the new JWT
    let mut response_headers = HeaderMap::new();
    set_auth_cookie(&mut response_headers, &jwt_token, 24 * 60 * 60);
    set_csrf_cookie(&mut response_headers, &csrf_token, 24 * 60 * 60);

    Span::current().record("success", true);
    info!("Session restored for user: {}", user_id);

    let session_response = SessionResponse {
        user: UserInfo {
            id: user.id.to_string(),
            user_id: user.id.to_string(),
            email: user.email.clone(),
            display_name: user.display_name,
            is_admin: user.is_admin,
            role: user.role.as_str().to_owned(),
            user_status: user.user_status.to_string(),
            tenant_id: tenant_id_for_response,
            created_at: user.created_at.to_rfc3339(),
            locale: user.locale,
            coaching_persona: user.coaching_persona.as_str().to_owned(),
            manages_roster: user.manages_roster,
        },
        access_token: jwt_token,
        csrf_token,
    };

    Ok((StatusCode::OK, response_headers, Json(session_response)).into_response())
}

/// Handle user profile update
///
/// Updates the authenticated user's display name.
/// Requires valid JWT authentication via cookie or Bearer token.
#[tracing::instrument(
    skip(resources, headers, request),
    fields(route = "update_profile", success = Empty)
)]
pub(super) async fn handle_update_profile(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Json(request): Json<UpdateProfileRequest>,
) -> Result<Response, AppError> {
    // Authenticate and get user ID
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let user_id = auth.user_id;

    // Validate display name
    let display_name = request.display_name.trim();
    if display_name.is_empty() {
        return Err(AppError::invalid_input("Display name cannot be empty"));
    }
    if display_name.len() > 100 {
        return Err(AppError::invalid_input(
            "Display name must be 100 characters or less",
        ));
    }

    // Update user in database
    let updated_user = resources
        .repos
        .users
        .update_display_name(user_id, display_name)
        .await?;

    // Build response
    let response = UpdateProfileResponse {
        message: "Profile updated successfully".to_owned(),
        user: UserInfo {
            id: updated_user.id.to_string(),
            user_id: updated_user.id.to_string(),
            email: updated_user.email,
            display_name: updated_user.display_name,
            is_admin: updated_user.is_admin,
            role: updated_user.role.to_string(),
            user_status: updated_user.user_status.to_string(),
            tenant_id: None,
            created_at: updated_user.created_at.to_rfc3339(),
            locale: updated_user.locale,
            coaching_persona: updated_user.coaching_persona.as_str().to_owned(),
            manages_roster: updated_user.manages_roster,
        },
    };

    info!(user_id = %user_id, "User profile updated successfully");

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle password change for authenticated users
///
/// Verifies the current password, validates the new password,
/// then hashes and stores the new password.
#[tracing::instrument(
    skip(resources, headers, request),
    fields(route = "change_password", success = Empty)
)]
pub(super) async fn handle_change_password(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Json(request): Json<ChangePasswordRequest>,
) -> Result<Response, AppError> {
    // Authenticate and get user ID
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let user_id = auth.user_id;

    // Fetch user to get current password hash
    let user = resources
        .repos
        .users
        .get_global(user_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("User {user_id}")))?;

    // Verify current password using the service helper
    let is_valid = AuthService::verify_password(
        request.current_password,
        user.password_hash.clone(), // Safe: ownership needed for blocking task
    )
    .await?;

    if !is_valid {
        return Err(AppError::auth_invalid("Current password is incorrect"));
    }

    // Validate new password strength
    if !AuthService::is_valid_password(&request.new_password) {
        return Err(AppError::invalid_input(error_messages::PASSWORD_TOO_WEAK));
    }

    // Hash new password using the service helper
    let password_hash = AuthService::hash_password(request.new_password).await?;

    // Update password in database
    resources
        .repos
        .users
        .update_password(user_id, &password_hash)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to update user password");
            AppError::internal(format!("Failed to update password: {e}"))
        })?;

    Span::current().record("success", true);
    info!(user_id = %user_id, "User password changed successfully");

    Ok((
        StatusCode::OK,
        Json(json!({ "message": "Password changed successfully" })),
    )
        .into_response())
}

/// Complete a password reset using a one-time token
///
/// Public endpoint — no authentication required. The reset token acts as proof
/// of authorization (issued by an admin). The user provides the token and their
/// chosen new password. The token is consumed atomically to prevent replay.
#[tracing::instrument(skip(resources, request), fields(route = "complete_reset"))]
pub(super) async fn handle_complete_reset(
    State(resources): State<Arc<ServerContext>>,
    Json(request): Json<CompleteResetRequest>,
) -> Result<Response, AppError> {
    use sha2::{Digest, Sha256};

    // Validate new password strength
    if !AuthService::is_valid_password(&request.new_password) {
        return Err(AppError::invalid_input(error_messages::PASSWORD_TOO_WEAK));
    }

    // Hash the presented token to match against stored hash
    let token_hash = format!("{:x}", Sha256::digest(request.reset_token.as_bytes()));

    // Atomically consume the token (validates existence, expiry, and single-use)
    let user_id = resources
        .repos
        .password_reset
        .consume_token(&token_hash)
        .await?;

    // Hash the new password using the service helper
    let password_hash = AuthService::hash_password(request.new_password).await?;

    // Update the user's password
    resources
        .repos
        .users
        .update_password(user_id, &password_hash)
        .await
        .map_err(|e| {
            error!(error = %e, "Failed to update user password during reset");
            AppError::internal(format!("Failed to update password: {e}"))
        })?;

    // Invalidate any other outstanding reset tokens for this user
    resources
        .repos
        .password_reset
        .invalidate_tokens(user_id)
        .await
        .map_err(|e| {
            warn!(error = %e, "Failed to invalidate remaining reset tokens");
            AppError::internal(format!("Failed to cleanup reset tokens: {e}"))
        })?;

    info!(user_id = %user_id, "Password reset completed via one-time token");

    Ok((
        StatusCode::OK,
        Json(json!({ "message": "Password has been reset successfully" })),
    )
        .into_response())
}

/// Handle self-service forgot-password requests
///
/// Generates a 6-digit numeric code, stores its SHA-256 hash with a 15-minute TTL,
/// and sends it to the user via email (if Resend is configured).
///
/// Security: Always returns HTTP 200 with an identical message regardless of whether
/// the email exists, to prevent account enumeration attacks.
#[tracing::instrument(skip(resources, request), fields(route = "forgot_password"))]
pub(super) async fn handle_forgot_password(
    State(resources): State<Arc<ServerContext>>,
    Json(request): Json<super::types::ForgotPasswordRequest>,
) -> Result<Response, AppError> {
    use crate::constants::password_reset;
    use rand::Rng;
    use sha2::{Digest, Sha256};

    let anti_enum_message =
        "If an account with that email exists, a reset code has been sent.".to_owned();

    // Basic email format validation
    if !request.email.contains('@') || request.email.len() < 5 {
        return Err(AppError::invalid_input("Invalid email format"));
    }

    // Look up user — if not found, return the same success message (anti-enumeration)
    let user = resources.repos.users.get_by_email(&request.email).await?;

    let Some(user) = user else {
        info!("Forgot password requested for nonexistent email (anti-enumeration)");
        return Ok((
            StatusCode::OK,
            Json(super::types::ForgotPasswordResponse {
                message: anti_enum_message,
            }),
        )
            .into_response());
    };

    // Rate limit: max N codes per hour per user
    let one_hour_ago = chrono::Utc::now() - chrono::Duration::hours(1);
    let recent_count = resources
        .repos
        .password_reset
        .count_recent_tokens(user.id, one_hour_ago)
        .await?;

    if recent_count >= password_reset::MAX_CODES_PER_HOUR {
        info!(
            user_id = %user.id,
            recent_count = recent_count,
            "Rate limit reached for password reset codes — skipping silently"
        );
        return Ok((
            StatusCode::OK,
            Json(super::types::ForgotPasswordResponse {
                message: anti_enum_message,
            }),
        )
            .into_response());
    }

    // Generate 6-digit numeric code
    let code: u32 =
        rand::rng().random_range(password_reset::CODE_RANGE_MIN..password_reset::CODE_RANGE_MAX);
    let code_str = code.to_string();

    // Hash the code before storing (same SHA-256 approach as admin tokens)
    let code_hash = format!("{:x}", Sha256::digest(code_str.as_bytes()));

    // Store with short TTL
    resources
        .repos
        .password_reset
        .store_token_with_ttl(
            user.id,
            &code_hash,
            password_reset::CREATED_BY_SELF_SERVICE,
            password_reset::CODE_TTL_MINUTES,
        )
        .await?;

    // Send email (or log warning if Resend not configured)
    if let Some(email_svc) = &resources.email_service {
        if let Err(e) = email_svc
            .send_password_reset_code(&request.email, &code_str)
            .await
        {
            warn!(error = %e, "Failed to send password reset email — user will not receive code");
        }
    } else {
        warn!("Email service not configured — password reset code generated but not delivered");
    }

    info!(user_id = %user.id, "Password reset code issued via self-service flow");

    Ok((
        StatusCode::OK,
        Json(super::types::ForgotPasswordResponse {
            message: anti_enum_message,
        }),
    )
        .into_response())
}

/// Handle user stats request for dashboard
///
/// Returns aggregated stats: connected providers, activities synced, and days active.
#[tracing::instrument(
    skip(resources, headers),
    fields(route = "user_stats", user_id = Empty)
)]
pub(super) async fn handle_user_stats(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    // Authenticate and get user ID
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let user_id = auth.user_id;
    Span::current().record("user_id", user_id.to_string());

    // Get connected providers count from OAuth tokens (cross-tenant view for user stats)
    let oauth_tokens = resources
        .repos
        .oauth_tokens
        .get_tokens(user_id, None)
        .await?;
    let connected_providers = i64::try_from(oauth_tokens.len()).unwrap_or(0);

    // Get user creation date to calculate days active
    let user = resources.repos.users.get_global(user_id).await?;
    let days_active = match user {
        Some(u) => {
            let now = chrono::Utc::now();
            let duration = now.signed_duration_since(u.created_at);
            duration.num_days().max(1)
        }
        None => 1,
    };

    let response = UserStatsResponse {
        connected_providers,
        days_active,
    };

    Ok((StatusCode::OK, Json(response)).into_response())
}

/// Handle `OAuth2` ROPC (Resource Owner Password Credentials) token request
///
/// This endpoint implements RFC 6749 Section 4.3 for MCP and CLI clients
/// that need to obtain tokens without a browser-based OAuth flow.
///
/// Request format: `application/x-www-form-urlencoded`
/// ```text
/// grant_type=password&username=user@example.com&password=secret
/// ```
///
/// Response format: RFC 6749 Section 5.1 compliant JSON
#[tracing::instrument(
    skip(resources, request),
    fields(
        route = "oauth2_token",
        grant_type = %request.grant_type,
        username = %request.username,
        user_id = Empty,
        tenant_id = Empty,
        success = Empty,
    )
)]
pub(super) async fn handle_oauth2_token(
    State(resources): State<Arc<ServerContext>>,
    Form(request): Form<OAuth2TokenRequest>,
) -> Result<Response, AppError> {
    // Validate grant_type
    if request.grant_type != "password" {
        let error_response = OAuth2ErrorResponse {
            error: "unsupported_grant_type".to_owned(),
            error_description: Some(format!(
                "Grant type '{}' is not supported. Use 'password' for ROPC.",
                request.grant_type
            )),
        };
        return Ok((StatusCode::BAD_REQUEST, Json(error_response)).into_response());
    }

    // Delegate to existing login logic
    let login_request = LoginRequest {
        email: request.username,
        password: request.password,
    };

    let auth_service = AuthService::new(resources.auth(), resources.config(), resources.data());

    match auth_service.login(login_request).await {
        Ok(response) => {
            let jwt_token = response
                .jwt_token
                .clone()
                .ok_or_else(|| AppError::internal("JWT token missing from login response"))?;

            // Parse expiration to calculate expires_in
            let expires_at = chrono::DateTime::parse_from_rfc3339(&response.expires_at)
                .map_or_else(
                    |_| chrono::Utc::now() + chrono::Duration::hours(24),
                    |dt| dt.with_timezone(&chrono::Utc),
                );
            let expires_in = (expires_at - chrono::Utc::now()).num_seconds();

            // Generate CSRF token for web clients (stateless HMAC — no server storage)
            let user_id = uuid::Uuid::parse_str(&response.user.user_id)
                .map_err(|e| AppError::internal(format!("Invalid user ID format: {e}")))?;
            let csrf_token = resources
                .csrf_manager
                .generate_token(user_id)
                .map_err(|e| AppError::internal(format!("Failed to generate CSRF token: {e}")))?;

            // notify: record tenant/user on the current span so the NotifyLayer can
            // attribute the user.login event without the call site re-passing IDs.
            // tenant_id is optional on UserInfo; only record when present so the
            // routing layer sees an empty field rather than a literal "None".
            Span::current().record("user_id", field::display(&user_id));
            if let Some(tenant_id) = response.user.tenant_id.as_deref() {
                Span::current().record("tenant_id", field::display(&tenant_id));
            }
            info!(
                target: "notify",
                event = "user.login",
                "user authenticated"
            );

            let oauth2_response = OAuth2TokenResponse {
                access_token: jwt_token.clone(),
                token_type: "Bearer".to_owned(),
                expires_in,
                refresh_token: None,
                scope: request.scope,
                // Pierre extensions for frontend compatibility
                user: Some(response.user),
                csrf_token: Some(csrf_token.clone()),
            };

            // Build response with secure cookies for web clients
            let mut headers = HeaderMap::new();
            set_auth_cookie(&mut headers, &jwt_token, 24 * 60 * 60);
            set_csrf_cookie(&mut headers, &csrf_token, 24 * 60 * 60);

            Ok((StatusCode::OK, headers, Json(oauth2_response)).into_response())
        }
        Err(e) => {
            // Map to OAuth2 error format based on error code
            let error_code = match e.code {
                ErrorCode::AuthInvalid | ErrorCode::AuthRequired | ErrorCode::AuthExpired => {
                    "invalid_grant"
                }
                ErrorCode::PermissionDenied
                | ErrorCode::AccountPending
                | ErrorCode::AccountSuspended => "access_denied",
                ErrorCode::InvalidInput | ErrorCode::InvalidFormat => "invalid_request",
                _ => "server_error",
            };
            let error_desc = e.message;

            let error_response = OAuth2ErrorResponse {
                error: error_code.to_owned(),
                error_description: Some(error_desc),
            };

            // OAuth2 spec: invalid_grant returns 400, server_error returns 500
            let status = if error_code == "server_error" {
                StatusCode::INTERNAL_SERVER_ERROR
            } else {
                StatusCode::BAD_REQUEST
            };

            Ok((status, Json(error_response)).into_response())
        }
    }
}

/// Handle analytics consent update for authenticated users
///
/// Updates the user's analytics consent preference and records the timestamp.
/// Also updates the in-memory consent cache used by the analytics tracker.
pub(super) async fn handle_analytics_consent(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Json(request): Json<AnalyticsConsentRequest>,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let user_id = auth.user_id;

    resources
        .repos
        .users
        .update_analytics_consent(user_id, request.enabled)
        .await?;

    // Update the in-memory consent cache so subsequent analytics events
    // respect the new preference immediately
    let hashed_user = hash_id(&user_id.to_string());
    analytics().set_consent(&hashed_user, request.enabled);

    info!(
        user_id = %user_id,
        enabled = request.enabled,
        "Analytics consent updated"
    );

    Ok((
        StatusCode::OK,
        Json(json!({ "message": "Analytics consent updated", "enabled": request.enabled })),
    )
        .into_response())
}

/// Supported BCP-47 short locales.
///
/// Matches the set compiled into `MessagingStringsRegistry::new()`. The
/// PATCH handler rejects anything outside this set so the frontend can't
/// persist a locale that would silently fall back to French at every
/// registry lookup.
const SUPPORTED_LOCALES: &[&str] = &["fr", "en", "es", "de", "pt"];

/// Handle locale preference update for authenticated users
///
/// Persists the user's preferred locale on the `users` table. Validates
/// the value against the compiled-in set so downstream registry lookups
/// always match an actual translation.
pub(super) async fn handle_update_locale(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Json(request): Json<UpdateLocaleRequest>,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let user_id = auth.user_id;

    let locale = request.locale.trim().to_ascii_lowercase();
    if !SUPPORTED_LOCALES.contains(&locale.as_str()) {
        return Err(AppError::invalid_input(format!(
            "Unsupported locale: {}. Supported: {}",
            request.locale,
            SUPPORTED_LOCALES.join(", ")
        )));
    }

    resources
        .repos
        .users
        .update_locale(user_id, &locale)
        .await?;

    info!(user_id = %user_id, locale = %locale, "User locale updated");

    Ok((
        StatusCode::OK,
        Json(json!({ "message": "Locale updated", "locale": locale })),
    )
        .into_response())
}

/// Handle coaching-persona update for authenticated users.
///
/// Persists the user's chosen output-format / cadence preference on the
/// `users` table. Validates against the [`pierre_core::models::CoachingPersona`]
/// enum's `FromStr`, so unknown values surface as `400 Bad Request` with
/// the canonical error rather than silently writing a value that won't
/// resolve to a persona block at chat time.
pub(super) async fn handle_update_coaching_persona(
    State(resources): State<Arc<ServerContext>>,
    headers: HeaderMap,
    Json(request): Json<UpdateCoachingPersonaRequest>,
) -> Result<Response, AppError> {
    let auth = extract_auth_from_headers(&headers, &resources).await?;
    let user_id = auth.user_id;

    let raw = request.persona.trim();
    let persona = raw.parse::<CoachingPersona>().map_err(|_| {
        AppError::invalid_input(format!(
            "Unsupported coaching persona: {raw}. Supported: casual, enthusiast, power_athlete, coach"
        ))
    })?;

    resources
        .repos
        .users
        .set_coaching_persona(user_id, persona)
        .await?;

    info!(user_id = %user_id, persona = persona.as_str(), "User coaching persona updated");

    Ok((
        StatusCode::OK,
        Json(json!({ "message": "Coaching persona updated", "persona": persona.as_str() })),
    )
        .into_response())
}
