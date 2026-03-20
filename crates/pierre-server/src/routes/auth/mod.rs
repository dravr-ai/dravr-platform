// ABOUTME: Router wiring for authentication and OAuth endpoints
// ABOUTME: Delegates to handler functions in login.rs and oauth.rs sub-modules
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Authentication routes for user management and OAuth flows
//!
//! This module handles user registration, login, and OAuth callback processing
//! for fitness providers like Strava. All handlers are thin wrappers that
//! delegate business logic to service layers.
//!
//! ## Module Structure
//! - `login` - AuthService: registration, login, Firebase auth, token refresh
//! - `oauth` - OAuthService: OAuth callbacks, provider connect/disconnect, status
//! - `types` - Request/response DTOs for auth endpoints

mod login;
mod oauth;
#[cfg(feature = "provider-sciotte")]
mod sciotte;
mod types;

pub use login::AuthService;
pub use oauth::{OAuthRoutes, OAuthService};

pub use types::{
    ChangePasswordRequest, CompleteResetRequest, ConnectionStatus, FirebaseLoginRequest,
    ForgotPasswordRequest, ForgotPasswordResponse, LoginRequest, LoginResponse,
    OAuth2ErrorResponse, OAuth2TokenRequest, OAuth2TokenResponse, OAuthAuthorizationResponse,
    OAuthStatus, ProviderStatus, ProvidersStatusResponse, RefreshTokenRequest, RegisterRequest,
    RegisterResponse, SessionResponse, UpdateProfileRequest, UpdateProfileResponse, UserInfo,
    UserStatsResponse,
};

// Re-export OAuthCallbackResponse from types module (moved for proper layering)
pub use crate::types::OAuthCallbackResponse;

use std::sync::Arc;

use axum::{
    routing::{delete, get, post, put},
    Router,
};

use crate::mcp::resources::ServerResources;

/// Authentication routes implementation (Axum)
///
/// Provides user registration, login, logout, and OAuth client authentication endpoints.
pub struct AuthRoutes;

impl AuthRoutes {
    /// Create all authentication routes (Axum)
    pub fn routes(resources: Arc<ServerResources>) -> Router {
        let router = Router::new()
            .route("/api/auth/register", post(login::handle_public_register))
            .route("/api/auth/admin/register", post(login::handle_register))
            .route("/api/auth/firebase", post(login::handle_firebase_login))
            .route("/api/auth/logout", post(login::handle_logout))
            .route("/api/auth/session", get(login::handle_session))
            .route("/api/auth/refresh", post(login::handle_refresh))
            .route("/api/user/profile", put(login::handle_update_profile))
            .route(
                "/api/user/change-password",
                put(login::handle_change_password),
            )
            .route(
                "/api/auth/forgot-password",
                post(login::handle_forgot_password),
            )
            .route(
                "/api/auth/complete-reset",
                post(login::handle_complete_reset),
            )
            .route("/api/user/stats", get(login::handle_user_stats))
            // OAuth2 ROPC endpoint (RFC 6749 Section 4.3) - unified login for all clients
            .route("/oauth/token", post(login::handle_oauth2_token))
            .route(
                "/api/oauth/callback/{provider}",
                get(oauth::handle_oauth_callback),
            )
            .route("/api/oauth/status", get(oauth::handle_oauth_status))
            .route("/api/providers", get(oauth::handle_providers_status))
            .route(
                "/api/oauth/auth/{provider}/{user_id}",
                get(oauth::handle_oauth_auth_initiate),
            )
            // Mobile OAuth initiation - returns OAuth URL in JSON (requires auth)
            .route(
                "/api/oauth/mobile/init/{provider}",
                get(oauth::handle_mobile_oauth_init),
            )
            // Disconnect a provider (requires auth)
            .route(
                "/api/oauth/providers/{provider}/disconnect",
                delete(oauth::handle_disconnect_provider_rest),
            );

        // Sciotte provider routes (credential login + session management)
        #[cfg(feature = "provider-sciotte")]
        let router = router
            .route(
                "/api/providers/sciotte/login",
                post(sciotte::handle_sciotte_login),
            )
            .route(
                "/api/providers/sciotte/select-2fa",
                post(sciotte::handle_sciotte_select_2fa),
            )
            .route(
                "/api/providers/sciotte/submit-otp",
                post(sciotte::handle_sciotte_submit_otp),
            )
            .route(
                "/api/providers/sciotte/connect",
                post(sciotte::handle_sciotte_connect),
            )
            .route(
                "/api/providers/sciotte/disconnect",
                delete(sciotte::handle_sciotte_disconnect),
            );

        router.with_state(resources)
    }
}
