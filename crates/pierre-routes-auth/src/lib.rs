// ABOUTME: Authentication route group — credential login, OAuth callback, Sciotte hosted login, provider-link webhook
// ABOUTME: Decoupled from pierre-server via AuthRoutesContext; mounted by the composition root
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Auth Routes
//!
//! Hosts every authentication-adjacent REST surface that ships with the
//! platform:
//!
//! - User-lifecycle endpoints (`/api/auth/*`, `/api/user/*`) — registration,
//!   credential login, Firebase SSO, session restore, profile update,
//!   password change, password reset, analytics consent, locale, coaching
//!   persona, plus the unified `OAuth2` ROPC token endpoint at `/oauth/token`.
//! - OAuth callback + provider connect/disconnect endpoints
//!   (`/api/oauth/*`, `/api/providers/*`) — handles fitness provider OAuth
//!   flows (Strava, Fitbit, Garmin, …) including mobile in-app browser
//!   initiation, callback redirect to web/mobile, and explicit sync triggers.
//! - Sciotte (Strava-mirror) credential-login endpoints
//!   (`/api/providers/sciotte/*`) and the channel-initiated hosted-login UI
//!   (`/providers/sciotte/{login,success,error}` plus the service-to-service
//!   `POST /api/channels/provider/sciotte/link-token` mint).
//!
//! Everything is wired through [`AuthRoutesContext`] — a concrete state
//! struct collecting the Arc handles every auth handler needs (auth manager,
//! JWKS, CSRF, repos, config, email + Firebase services, provider registry,
//! tenant OAuth client, sync notifier, optional sync orchestrator, Cache,
//! plus the `provider-sciotte`-gated link-token rate limiter and nonce
//! store). The composition root in `pierre-server` builds the context once
//! from its `ServerContext` and passes it as Axum state.
//!
//! The context-struct pattern (rather than a 13-method `AuthRoutesCtx`
//! trait) matches the precedent set by [`pierre_routes_identity::oauth2`]
//! (`OAuth2Context`) and [`pierre_routes_admin::AdminApiContext`] — fields
//! pulled from disparate optional crates (Firebase, Resend, Sciotte) where
//! growing the trait would force `pierre-runtime-context` to depend on the
//! union of all of them.

#![warn(missing_docs)]

use std::sync::Arc;

use axum::{
    routing::{delete, get, post, put},
    Router,
};
use tokio::sync::broadcast;

use pierre_auth::admin::jwks::JwksManager;
use pierre_auth::auth::AuthManager;
use pierre_auth::firebase::FirebaseAuth;
use pierre_auth::security::csrf::CsrfTokenManager;
use pierre_auth::tenant::TenantOAuthClient;
use pierre_cache::Cache;
use pierre_config::environment::ServerConfig;
use pierre_database::RepositoryRegistry;
use pierre_email::ResendEmailService;
use pierre_mcp_schema::OAuthCompletedNotification;
#[cfg(feature = "provider-sciotte")]
use pierre_middleware::provider_link_token::{MintRateLimiter, NonceStore};
use pierre_middleware::McpAuthMiddleware;
use pierre_providers::ProviderRegistry;
use pierre_runtime_context::DataContext;
use pierre_services::provider_refresh::SyncNotifier;

mod login;
mod oauth;
#[cfg(feature = "provider-sciotte")]
mod provider_link_webhook;
#[cfg(feature = "provider-sciotte")]
mod sciotte;
#[cfg(feature = "provider-sciotte")]
mod sciotte_hosted;
#[cfg(feature = "provider-sciotte")]
mod sciotte_hosted_templates;

#[cfg(feature = "provider-sciotte")]
pub use sciotte::init_sciotte_limiter;

// Re-export the cross-cutting auth DTOs so existing
// `crate::routes::auth::*` paths in pierre-server tests + callers continue
// to compile after the move (`pub use pierre_routes_auth as auth;` in
// `routes/mod.rs`).
pub use pierre_auth::dto::auth::{
    AnalyticsConsentRequest, ChangePasswordRequest, CompleteResetRequest, ConnectionStatus,
    FirebaseLoginRequest, ForgotPasswordRequest, ForgotPasswordResponse, LoginRequest,
    LoginResponse, OAuth2ErrorResponse, OAuth2TokenRequest, OAuth2TokenResponse,
    OAuthAuthorizationResponse, OAuthStatus, ProviderStatus, ProvidersStatusResponse,
    RefreshTokenRequest, RegisterRequest, RegisterResponse, SessionResponse, UpdateProfileRequest,
    UpdateProfileResponse, UserInfo, UserStatsResponse,
};

pub use pierre_mcp_transport::OAuthCallbackResponse;
pub use pierre_services::auth::AuthService;
pub use pierre_services::oauth_flow::OAuthService;

/// Alias for the `OAuthService` to match existing test expectations.
pub type OAuthRoutes = OAuthService;

/// Shared state for every auth route handler in this crate.
///
/// Collects the Arc handles the handlers need from the composition root's
/// `ServerContext`. The struct is `Clone` (every field is an `Arc` or
/// trivially cloneable) so Axum can pass it as state through the router.
#[derive(Clone)]
pub struct AuthRoutesContext {
    /// JWT auth manager (token mint / parse).
    pub auth_manager: Arc<AuthManager>,
    /// JWKS manager — signing-key source for `auth_manager`.
    pub jwks_manager: Arc<JwksManager>,
    /// Stateless CSRF token manager.
    pub csrf_manager: Arc<CsrfTokenManager>,
    /// Inbound `Authorization` header / cookie auth pipeline.
    pub auth_middleware: Arc<McpAuthMiddleware>,
    /// Repository registry — auth handlers span `AuthRepos` (`users`,
    /// `tenants`, `admin`, `oauth_tokens`) plus a thin slice of
    /// `FitnessRepos` (`provider_connections` is written when an OAuth
    /// callback succeeds), so the full registry is the canonical fit.
    pub repos: Arc<RepositoryRegistry>,
    /// Server config (base URL, frontend URL, security policy, admin TTL).
    pub config: Arc<ServerConfig>,
    /// Data context bundle (database + repos + cache + provider registry)
    /// — handed to `AuthService` and `OAuthService` constructors.
    pub data: DataContext,
    /// Firebase auth service. `None` when Firebase is not configured.
    pub firebase_auth: Option<Arc<FirebaseAuth>>,
    /// Outbound email service (Resend). `None` when not configured.
    pub email_service: Option<Arc<ResendEmailService>>,
    /// OAuth-completed notification broadcast (for MCP `sampling/createMessage`
    /// listeners that wait on OAuth completion).
    pub oauth_notification_sender: Option<broadcast::Sender<OAuthCompletedNotification>>,
    /// Tenant OAuth client — generates per-tenant authorization URLs.
    pub tenant_oauth_client: Arc<TenantOAuthClient>,
    /// Provider registry — descriptor lookups (PKCE capability, OAuth params).
    pub provider_registry: Arc<ProviderRegistry>,
    /// Background sync notifier (the SSE manager in production).
    pub sync_notifier: Arc<dyn SyncNotifier>,
    /// Health-data sync orchestrator (enforme). `None` when health-sync
    /// feature is disabled or orchestrator init failed.
    #[cfg(feature = "health-sync")]
    pub sync_orchestrator: Option<Arc<pierre_enforme::SyncOrchestrator>>,
    /// Shared cache handle — used by the Sciotte prefetch path.
    pub cache: Arc<Cache>,
    /// Admin-token JWT signing secret — also used to verify the Sciotte
    /// hosted-login link-token.
    pub admin_jwt_secret: Arc<str>,
    /// Per-user rate limiter for hosted-login link-token mint requests.
    #[cfg(feature = "provider-sciotte")]
    pub mint_rate_limiter: Arc<MintRateLimiter<Cache>>,
    /// Single-use nonce store for hosted-login link-token jti claims.
    #[cfg(feature = "provider-sciotte")]
    pub nonce_store: Arc<NonceStore<Cache>>,
}

/// Authentication route group entry point.
///
/// Builds the Axum router covering every endpoint documented in the
/// crate-level summary above. `Router<()>` is returned with the
/// [`AuthRoutesContext`] already installed as state so the composition root
/// can merge it directly into the main application router.
pub struct AuthRoutes;

impl AuthRoutes {
    /// Create all authentication routes (Axum).
    pub fn routes(context: AuthRoutesContext) -> Router {
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
            .route(
                "/api/user/analytics-consent",
                put(login::handle_analytics_consent),
            )
            .route("/api/user/locale", put(login::handle_update_locale))
            .route(
                "/api/user/coaching-persona",
                put(login::handle_update_coaching_persona),
            )
            // OAuth2 ROPC endpoint (RFC 6749 Section 4.3) — unified login for all clients
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
            // Mobile OAuth initiation — returns OAuth URL in JSON (requires auth)
            .route(
                "/api/oauth/mobile/init/{provider}",
                get(oauth::handle_mobile_oauth_init),
            )
            // Disconnect a provider (requires auth)
            .route(
                "/api/oauth/providers/{provider}/disconnect",
                delete(oauth::handle_disconnect_provider_rest),
            )
            // Trigger provider data sync (requires auth)
            .route(
                "/api/providers/{provider}/sync",
                post(oauth::handle_sync_provider),
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
            )
            // Channel-initiated hosted Sciotte login: mint + serve pages.
            // The POST endpoint is service-to-service (admin auth) and mints a
            // short-lived link-token. The GET endpoints render the hosted UI.
            .route(
                "/api/channels/provider/sciotte/link-token",
                post(sciotte_hosted::handle_mint_sciotte_link_token),
            )
            .route(
                "/providers/sciotte/login",
                get(sciotte_hosted::handle_sciotte_hosted_login_page),
            )
            .route(
                "/providers/sciotte/success",
                get(sciotte_hosted::handle_sciotte_hosted_success_page),
            )
            .route(
                "/providers/sciotte/error",
                get(sciotte_hosted::handle_sciotte_hosted_error_page),
            );

        router.with_state(context)
    }
}
