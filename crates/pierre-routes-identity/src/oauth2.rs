// ABOUTME: OAuth 2.0 server route handlers for RFC-compliant authorization server endpoints
// ABOUTME: Provides OAuth 2.0 protocol endpoints including client registration, authorization, and token exchange
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// NOTE: All `.clone()` calls in this file are Safe - they are necessary for:
// - OAuth client field ownership transfers for registration and token requests
// - Resource Arc sharing for HTTP route handlers
// - String ownership for OAuth protocol responses

use axum::{
    extract::{ConnectInfo, Form, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use pierre_auth::admin::jwks::{JsonWebKeySet, JwksManager};
use pierre_auth::auth::AuthManager;
use pierre_auth::config::oauth::OAuth2ServerConfig;
use pierre_auth::dto::auth::LoginRequest;
use pierre_auth::oauth2_server::{
    client_registration::ClientRegistrationManager,
    endpoints::OAuth2AuthorizationServer,
    models::{
        AuthorizeRequest, ClientRegistrationRequest, OAuth2Error, TokenRequest,
        ValidateRefreshRequest,
    },
    rate_limiting::OAuth2RateLimiter,
};
use pierre_auth::rate_limiting::OAuth2Endpoint;
use pierre_auth::security::cookies::{host_cookie_name, SameSitePolicy, SecureCookieConfig};
use pierre_auth::security::csrf::CsrfTokenManager;
use pierre_core::errors::AppError;
use pierre_database::backends::{factory::Database, OAuth2ServerRepository};
use pierre_database::database::repositories::{TenantRepository, UserRepository};
use pierre_middleware::redaction::mask_email;
use pierre_services::auth::AuthService;
use sha2::{Digest, Sha256};
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::task::spawn_blocking;
use tracing::{debug, error, info, trace, warn};

use crate::authorize_redirect::{code_redirect, error_redirect, rejection_response};
use crate::oauth2_rate_limited::{page_refusal, refusal};

/// The consent form's submission: the user's decision on an authorization
mod consent;
/// "Continue with Google": server-side Google sign-in on the hosted login page
mod google_login;
/// Server-rendered login, consent and error pages of the authorization flow
mod pages;
/// The RFC 8414 and RFC 9728 discovery documents
mod well_known;

pub use google_login::GoogleSignIn;
pub use pages::{ConsentHtmlParams, LoginHtmlParams};

/// Name of the authorization server's own session cookie, before the
/// `__Host-` prefix an HTTPS issuer gives it
const SESSION_COOKIE: &str = "pierre_session";

/// Lifetime of the session cookie: the 24 hours of the JWT it carries
const SESSION_COOKIE_MAX_AGE_SECS: i64 = 86_400;

/// OAuth 2.0 server context shared across all handlers
#[derive(Clone)]
pub struct OAuth2Context {
    /// Database for lifecycle and system settings
    pub database: Arc<Database>,
    /// `OAuth2` server repository for client registration, auth codes, and tokens
    pub oauth2_server: Arc<dyn OAuth2ServerRepository>,
    /// Tenant repository for user-tenant lookups
    pub tenants: Arc<dyn TenantRepository>,
    /// User repository for credential and profile lookups
    pub users: Arc<dyn UserRepository>,
    /// Authentication manager for JWT operations
    pub auth_manager: Arc<AuthManager>,
    /// JWKS manager for public key operations
    pub jwks_manager: Arc<JwksManager>,
    /// OAuth 2.0 authorization-server configuration (issuer URL + dev-only
    /// default login form values). Replaces the prior reference to the
    /// pierre-server `ServerConfig` so this crate has no dependency on
    /// pierre-server-internal config types.
    pub config: Arc<OAuth2ServerConfig>,
    /// Rate limiter for OAuth endpoints
    pub rate_limiter: Arc<OAuth2RateLimiter>,
    /// Refresh-token lifetime in days, the `REFRESH_TOKEN_EXPIRY_DAYS` sessions read too
    pub refresh_token_expiry_days: i64,
    /// Mints and checks the consent form's synchronizer token: the form is
    /// plain HTML, so it cannot carry the `X-CSRF-Token` header the API's
    /// cookie sessions use
    pub csrf_manager: Arc<CsrfTokenManager>,
    /// Account rules shared with the web app's sign-in: both buttons of the
    /// hosted login page sign athletes in through them
    pub accounts: AuthService,
    /// Google sign-in on the hosted login page; `None` when unconfigured,
    /// which hides the button
    pub google_sign_in: Option<GoogleSignIn>,
}

/// OAuth 2.0 routes implementation
pub struct OAuth2Routes;

impl OAuth2Routes {
    /// Create all OAuth 2.0 routes with context
    pub fn routes(context: OAuth2Context) -> Router {
        Router::new()
            // RFC 8414: OAuth 2.0 Authorization Server Metadata
            .route(
                "/.well-known/oauth-authorization-server",
                get(Self::handle_discovery),
            )
            // RFC 9728: OAuth 2.0 Protected Resource Metadata (MCP resource server)
            .route(
                "/.well-known/oauth-protected-resource",
                get(Self::handle_protected_resource_metadata),
            )
            // RFC 7517: JWKS endpoint
            .route("/.well-known/jwks.json", get(Self::handle_jwks))
            // RFC 7591: Dynamic Client Registration
            .route("/oauth2/register", post(Self::handle_client_registration))
            // OAuth 2.0 Authorization endpoint
            .route("/oauth2/authorize", get(Self::handle_authorization))
            // OAuth 2.0 Token endpoint
            .route("/oauth2/token", post(Self::handle_token))
            // Login page and submission
            .route("/oauth2/login", get(Self::handle_oauth_login_page))
            .route("/oauth2/login", post(Self::handle_oauth_login_submit))
            .route("/oauth2/login/google", get(Self::handle_google_login_start))
            .route(
                "/oauth2/login/google/callback",
                get(Self::handle_google_login_callback),
            )
            .route("/oauth2/consent", post(Self::handle_consent_submit))
            // Token validation endpoints
            .route(
                "/oauth2/validate-and-refresh",
                post(Self::handle_validate_and_refresh),
            )
            .route("/oauth2/token-validate", post(Self::handle_token_validate))
            // JWKS also available at /oauth2/jwks
            .route("/oauth2/jwks", get(Self::handle_jwks))
            .with_state(context)
    }

    /// Handle client registration (RFC 7591)
    async fn handle_client_registration(
        State(context): State<OAuth2Context>,
        ConnectInfo(addr): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
        Json(request): Json<ClientRegistrationRequest>,
    ) -> Response {
        let limiter = &context.rate_limiter;
        if let Some(refused) = refusal(limiter, OAuth2Endpoint::Register, addr, &headers).await {
            return refused;
        }

        // Registrations no user has authorized yet are capped; past the cap the
        // refusal is a 429 like the rate limiter's, in the same body shape.
        let ceiling = context.config.client_retention.max_pending_registrations;
        let client_manager = ClientRegistrationManager::new(context.oauth2_server.clone());

        match client_manager.register_client(request, ceiling).await {
            Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
            Err(error) => (error.http_status(), Json(error)).into_response(),
        }
    }

    /// Handle authorization request (GET /oauth2/authorize)
    async fn handle_authorization(
        State(context): State<OAuth2Context>,
        ConnectInfo(addr): ConnectInfo<SocketAddr>,
        Query(params): Query<HashMap<String, String>>,
        headers: HeaderMap,
    ) -> Response {
        let limiter = &context.rate_limiter;
        let render = Self::render_oauth_error_response;
        if let Some(refused) = page_refusal(limiter, addr, &headers, render).await {
            return refused;
        }

        // Parse query parameters into AuthorizeRequest
        let request = match Self::parse_authorize_request(&params) {
            Ok(req) => req,
            Err(error) => return Self::render_oauth_error_response(&error),
        };

        // Check the request before the user is asked to log in or consent, so
        // a refusal reaches the client the way RFC 6749 Section 4.1.2.1 sends it.
        let checked = match Self::authorization_server(&context)
            .check_authorize_request(&request)
            .await
        {
            Ok(checked) => checked,
            Err(rejection) => return rejection_response(rejection, &request),
        };

        let redirect_uri = request.redirect_uri.clone();

        // Check if user is authenticated via session cookie
        let (user_id, tenant_id) = Self::extract_authenticated_user(&headers, &context);

        // If no authenticated user, redirect to login page with OAuth parameters
        let Some(authenticated_user_id) = user_id else {
            info!("No authenticated session for OAuth authorization, redirecting to login");
            let login_url = Self::build_login_url_with_oauth_params(&request);
            return Redirect::to(&login_url).into_response();
        };

        Self::execute_authorization(
            &context,
            request,
            authenticated_user_id,
            tenant_id,
            redirect_uri,
            checked.client_name.as_deref(),
        )
        .await
    }

    fn extract_authenticated_user(
        headers: &HeaderMap,
        context: &OAuth2Context,
    ) -> (Option<uuid::Uuid>, Option<String>) {
        headers
            .get(header::COOKIE)
            .and_then(|cookie_value| {
                cookie_value.to_str().ok().and_then(|cookie_str| {
                    Self::extract_session_token(cookie_str, &Self::session_cookie_name(context))
                        .and_then(|token| Self::validate_session_token(&token, context))
                })
            })
            .map_or((None, None), |(uid, tid)| (Some(uid), tid))
    }

    fn validate_session_token(
        token: &str,
        context: &OAuth2Context,
    ) -> Option<(uuid::Uuid, Option<String>)> {
        match context
            .auth_manager
            .validate_session_token(token, &context.jwks_manager)
        {
            Ok(claims) => {
                info!(
                    "OAuth authorization for authenticated user_id: {}",
                    claims.sub
                );
                if let Ok(user_uuid) = uuid::Uuid::parse_str(&claims.sub) {
                    // Get active tenant from claims
                    Some((user_uuid, claims.active_tenant_id.clone()))
                } else {
                    warn!("Invalid user ID format in JWT: {}", claims.sub);
                    None
                }
            }
            Err(e) => {
                warn!("Invalid session token in OAuth authorization: {}", e);
                None
            }
        }
    }

    async fn execute_authorization(
        context: &OAuth2Context,
        request: AuthorizeRequest,
        authenticated_user_id: uuid::Uuid,
        tenant_id: Option<String>,
        redirect_uri: String,
        client_name: Option<&str>,
    ) -> Response {
        // Only an active account authorizes a client: a pending one would get
        // a connector every call of which is refused, a suspended one none.
        let user = match Self::account_gate(context, authenticated_user_id).await {
            Ok(user) => user,
            Err(refused) => return *refused,
        };

        // Skip the consent screen only when the user has already approved this client
        // for the requested scope; otherwise show consent before any code is minted.
        let scope = request.scope.clone().unwrap_or_default();
        let grant_tenant =
            Self::resolve_grant_tenant(context, authenticated_user_id, tenant_id.clone()).await;
        let has_grant = match &grant_tenant {
            Some(tid) => context
                .oauth2_server
                .find_active_client_grant(
                    &authenticated_user_id.to_string(),
                    tid,
                    &request.client_id,
                    &scope,
                )
                .await
                .unwrap_or_else(|e| {
                    error!("Failed to look up the client grant, showing consent: {e}");
                    None
                })
                .is_some(),
            None => false,
        };

        if has_grant {
            return Self::mint_authorization_code(
                context,
                request,
                authenticated_user_id,
                tenant_id,
                redirect_uri,
            )
            .await;
        }

        // The consent form's synchronizer token, bound to this user: a page
        // on another origin cannot read it, so it cannot post an approval.
        match context.csrf_manager.generate_token(authenticated_user_id) {
            Ok(csrf_token) => {
                Self::render_consent_page(&request, &csrf_token, client_name, &user.email)
            }
            Err(e) => {
                error!("Failed to mint the consent form token: {e}");
                Self::render_oauth_error_response(&OAuth2Error::server_error(
                    "The consent screen could not be prepared",
                ))
            }
        }
    }

    /// Resolve the tenant used to key an OAuth client grant.
    ///
    /// Uses the tenant from the session claims when present, otherwise the user's
    /// first tenant — mirroring the resolution done when minting the code.
    async fn resolve_grant_tenant(
        context: &OAuth2Context,
        user_id: uuid::Uuid,
        tenant_id: Option<String>,
    ) -> Option<String> {
        if tenant_id.is_some() {
            return tenant_id;
        }
        context
            .tenants
            .list_for_user(user_id)
            .await
            .inspect_err(|e| error!("Failed to list tenants for the client grant: {e}"))
            .ok()
            .and_then(|tenants| tenants.first().map(|t| t.id.to_string()))
    }

    /// The authorization server every route checks, mints and validates
    /// through, serving the configured MCP resource.
    fn authorization_server(context: &OAuth2Context) -> OAuth2AuthorizationServer {
        OAuth2AuthorizationServer::new(
            context.oauth2_server.clone(),
            context.tenants.clone(),
            context.users.clone(),
            context.auth_manager.clone(),
            context.jwks_manager.clone(),
            context.refresh_token_expiry_days,
            context
                .config
                .mcp_resources()
                .into_iter()
                .map(str::to_owned)
                .collect(),
        )
    }

    /// Mint an authorization code and redirect back to the client.
    ///
    /// Reached only with a request [`OAuth2AuthorizationServer::check_authorize_request`]
    /// accepted, so the client and `redirect_uri` are verified and a failure
    /// here goes back to the client too.
    async fn mint_authorization_code(
        context: &OAuth2Context,
        request: AuthorizeRequest,
        authenticated_user_id: uuid::Uuid,
        tenant_id: Option<String>,
        redirect_uri: String,
    ) -> Response {
        let state = request.state.clone();
        match Self::authorization_server(context)
            .authorize(request, Some(authenticated_user_id), tenant_id)
            .await
        {
            Ok(response) => {
                info!(
                    "OAuth authorization successful for user {}, redirecting with code",
                    authenticated_user_id
                );
                code_redirect(&redirect_uri, &response.code, response.state.as_deref())
            }
            Err(error) => {
                warn!(
                    "OAuth authorization failed for user {}: {:?}",
                    authenticated_user_id, error
                );
                error_redirect(&redirect_uri, state.as_deref(), &error)
            }
        }
    }

    /// Handle token request (POST /oauth2/token)
    async fn handle_token(
        State(context): State<OAuth2Context>,
        ConnectInfo(addr): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
        Form(form): Form<HashMap<String, String>>,
    ) -> Response {
        let limiter = &context.rate_limiter;
        if let Some(refused) = refusal(limiter, OAuth2Endpoint::Token, addr, &headers).await {
            return refused;
        }

        let request = match Self::parse_and_log_token_request(&form) {
            Ok(req) => req,
            Err(error) => return (StatusCode::BAD_REQUEST, Json(error)).into_response(),
        };

        let auth_server = Self::authorization_server(&context);

        Self::execute_token_exchange(auth_server, request, &form).await
    }

    fn parse_and_log_token_request(
        form: &HashMap<String, String>,
    ) -> Result<TokenRequest, OAuth2Error> {
        debug!(
            "OAuth token request received with grant_type: {:?}, client_id: {:?}",
            form.get("grant_type"),
            form.get("client_id")
        );

        Self::parse_token_request(form).map_err(|error| {
            warn!("OAuth token request parsing failed: {:?}", error);
            error
        })
    }

    async fn execute_token_exchange(
        auth_server: OAuth2AuthorizationServer,
        request: TokenRequest,
        form: &HashMap<String, String>,
    ) -> Response {
        match auth_server.token(request).await {
            Ok(response) => {
                info!(
                    "OAuth token exchange successful for client: {}",
                    form.get("client_id").map_or("unknown", |v| v)
                );
                (StatusCode::OK, Json(response)).into_response()
            }
            Err(error) => {
                warn!(
                    "OAuth token exchange failed for client {}: {:?}",
                    form.get("client_id").map_or("unknown", |v| v),
                    error
                );
                (error.http_status(), Json(error)).into_response()
            }
        }
    }

    /// Handle validate and refresh request (POST /oauth2/validate-and-refresh)
    async fn handle_validate_and_refresh(
        State(context): State<OAuth2Context>,
        headers: HeaderMap,
        Json(request): Json<ValidateRefreshRequest>,
    ) -> Response {
        // Extract Bearer token from Authorization header
        let access_token = match Self::extract_bearer_token(&headers) {
            Ok(token) => token,
            Err(response) => return *response,
        };

        debug!(
            "Validate-and-refresh request received (token_length: {})",
            access_token.len()
        );

        let auth_server = Self::authorization_server(&context);

        match auth_server
            .validate_and_refresh(&access_token, request)
            .await
        {
            Ok(response) => {
                info!(
                    "Token validation completed with status: {:?}",
                    response.status
                );
                (StatusCode::OK, Json(response)).into_response()
            }
            Err(error) => {
                error!("Validate-and-refresh failed: {}", error);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({
                        "error": "internal_error",
                        "error_description": "Failed to validate token"
                    })),
                )
                    .into_response()
            }
        }
    }

    /// Build validation response for valid credentials
    fn validation_success_response() -> Response {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "valid": true
            })),
        )
            .into_response()
    }

    /// Build validation response for invalid client
    fn validation_invalid_client_response() -> Response {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "valid": false,
                "error": "invalid_client",
                "error_description": "Client ID not found or invalid"
            })),
        )
            .into_response()
    }

    /// Build validation response for missing credentials
    fn validation_missing_credentials_response() -> Response {
        (
            StatusCode::OK,
            Json(serde_json::json!({
                "valid": false,
                "error": "invalid_request",
                "error_description": "Either access token or client_id must be provided"
            })),
        )
            .into_response()
    }

    /// Validate `client_id` and return appropriate response
    async fn validate_client_id_response(
        oauth2_server: Arc<dyn OAuth2ServerRepository>,
        client_id: &str,
    ) -> Response {
        let client_manager = ClientRegistrationManager::new(oauth2_server);
        match client_manager.get_client(client_id).await {
            Ok(_) => {
                info!(
                    "Credentials validated successfully for client_id: {}",
                    client_id
                );
                Self::validation_success_response()
            }
            Err(e) => {
                ClientRegistrationManager::log_lookup_failure(client_id, &e);
                Self::validation_invalid_client_response()
            }
        }
    }

    /// Handle token validation request (POST /oauth2/token-validate)
    async fn handle_token_validate(
        State(context): State<OAuth2Context>,
        headers: HeaderMap,
        Json(request): Json<serde_json::Value>,
    ) -> Response {
        debug!("Token validation request received");

        // Extract client_id from request body (optional)
        let client_id = request.get("client_id").and_then(|v| v.as_str());

        // Validate access token if provided
        let token_valid = match Self::validate_bearer_token_for_validate_endpoint(
            &headers,
            &context.auth_manager,
            &context.jwks_manager,
            &context.config.mcp_resources(),
        ) {
            Ok(valid) => valid,
            Err(response) => return *response,
        };

        // Validate client_id if provided
        if let Some(cid) = client_id {
            return Self::validate_client_id_response(context.oauth2_server, cid).await;
        }

        if token_valid {
            Self::validation_success_response()
        } else {
            Self::validation_missing_credentials_response()
        }
    }

    /// Handle OAuth login page (GET /oauth2/login)
    async fn handle_oauth_login_page(
        State(context): State<OAuth2Context>,
        Query(params): Query<HashMap<String, String>>,
    ) -> Html<String> {
        // Extract OAuth parameters to preserve them through login flow (including PKCE)
        let client_id = params
            .get("client_id")
            .map_or_else(String::new, ToString::to_string);
        let redirect_uri = params
            .get("redirect_uri")
            .map_or_else(String::new, ToString::to_string);
        let response_type = params
            .get("response_type")
            .map_or_else(String::new, ToString::to_string);
        let state = params
            .get("state")
            .map_or_else(String::new, ToString::to_string);
        let scope = params
            .get("scope")
            .map_or_else(String::new, ToString::to_string);
        let code_challenge = params
            .get("code_challenge")
            .map_or_else(String::new, ToString::to_string);
        let code_challenge_method = params
            .get("code_challenge_method")
            .map_or_else(String::new, ToString::to_string);
        let resource = params
            .get("resource")
            .map_or_else(String::new, ToString::to_string);

        // Get default form values from OAuth2ServerConfig (for dev/test only)
        // Safe: Option<String> ownership for HTML template
        let default_email = context
            .config
            .default_login_email
            .clone()
            .unwrap_or_default();
        let default_password = context
            .config
            .default_login_password
            .clone()
            .unwrap_or_default();

        // "Continue with Google", on the issuer's host where Google returns,
        // for a pending authorization this page can name
        let google_start_url = context
            .google_sign_in
            .as_ref()
            .and_then(|_| Self::parse_authorize_request(&params).ok())
            .map(|request| {
                Self::authorize_params_url(
                    &format!(
                        "{}/oauth2/login/google",
                        context.config.issuer_url.trim_end_matches('/')
                    ),
                    &request,
                )
            });

        // Use spawn_blocking for HTML generation (CPU-bound string formatting)
        let html = spawn_blocking(move || {
            Self::generate_login_html(LoginHtmlParams {
                client_id: &client_id,
                redirect_uri: &redirect_uri,
                response_type: &response_type,
                state: &state,
                scope: &scope,
                code_challenge: &code_challenge,
                code_challenge_method: &code_challenge_method,
                resource: &resource,
                default_email: &default_email,
                default_password: &default_password,
                google_start_url: google_start_url.as_deref(),
            })
        })
        .await
        .unwrap_or_else(|_| Self::generic_error_html());

        Html(html)
    }

    /// Handle OAuth login form submission (POST /oauth2/login)
    async fn handle_oauth_login_submit(
        State(context): State<OAuth2Context>,
        Form(form): Form<HashMap<String, String>>,
    ) -> Response {
        // Extract credentials from form
        let Some(email) = form.get("email") else {
            return (StatusCode::BAD_REQUEST, "Missing email").into_response();
        };

        let Some(password) = form.get("password") else {
            return (StatusCode::BAD_REQUEST, "Missing password").into_response();
        };

        // The account rules every password login follows: a suspended account
        // is refused here, and the login is recorded as the web app's is
        let login = context
            .accounts
            .login(LoginRequest {
                email: email.clone(),
                password: password.clone(),
                timezone: None,
            })
            .await
            .and_then(|login| {
                // Every successful sign-in raises user.login, as the web app's does
                info!(
                    target: "notify",
                    event = "user.login",
                    user_id = %login.user.user_id,
                    tenant_id = %login.user.tenant_id.as_deref().unwrap_or_default(),
                    "user authenticated"
                );
                login
                    .jwt_token
                    .ok_or_else(|| AppError::internal("Login minted no session token"))
            });

        match login {
            Ok(token) => {
                // Continue the authorization flow with the OAuth parameters the
                // form carried (PKCE and the RFC 8707 resource included)
                let auth_url = Self::build_authorization_url_from_form(&form);

                info!(
                    "User {} authenticated successfully for OAuth, redirecting to authorization",
                    mask_email(email)
                );

                (
                    StatusCode::FOUND,
                    [
                        (header::LOCATION, auth_url),
                        (header::SET_COOKIE, Self::session_cookie(&context, &token)),
                    ],
                )
                    .into_response()
            }
            Err(e) => {
                if e.is_server_fault() {
                    error!("OAuth login could not complete: {}", e);
                } else {
                    warn!("Authentication failed for OAuth login: {}", e);
                }

                Self::login_failure_response(&form)
            }
        }
    }

    /// Handle JWKS endpoint (GET /oauth2/jwks or GET /.well-known/jwks.json)
    async fn handle_jwks(State(context): State<OAuth2Context>, headers: HeaderMap) -> Response {
        // Return JWKS with RS256 public keys for token validation
        let jwks = match context.jwks_manager.get_jwks() {
            Ok(jwks) => jwks,
            Err(e) => {
                error!("Failed to generate JWKS: {}", e);
                // Return empty JWKS on error (graceful degradation)
                return (
                    StatusCode::OK,
                    [(header::CACHE_CONTROL, "public, max-age=3600")],
                    Json(serde_json::json!({ "keys": [] })),
                )
                    .into_response();
            }
        };

        debug!("JWKS endpoint accessed, returning {} keys", jwks.keys.len());

        // Calculate ETag from JWKS content for efficient caching
        let (_jwks_json, etag) = match Self::compute_jwks_etag(jwks.clone()).await {
            Ok(result) => result,
            Err(response) => return *response,
        };

        // Check if client's cached version matches current version
        if Self::check_etag_match(&headers, &etag) {
            debug!("JWKS ETag match, returning 304 Not Modified");
            return (StatusCode::NOT_MODIFIED, [(header::ETAG, etag)]).into_response();
        }

        // Return JWKS with ETag and Cache-Control headers
        (
            StatusCode::OK,
            [
                (header::CACHE_CONTROL, "public, max-age=3600".to_owned()),
                (header::ETAG, etag),
            ],
            Json(jwks),
        )
            .into_response()
    }

    // ============================================================================
    // Helper Functions
    // ============================================================================

    /// Compute JWKS `ETag` from JSON content
    async fn compute_jwks_etag(jwks: JsonWebKeySet) -> Result<(String, String), Box<Response>> {
        let etag_result = spawn_blocking(move || {
            let jwks_json = serde_json::to_string(&jwks)?;
            let mut hasher = Sha256::new();
            hasher.update(jwks_json.as_bytes());
            let hash = hasher.finalize();
            let etag = format!(r#""{}""#, hex::encode(&hash[..16]));
            Ok::<(String, String), serde_json::Error>((jwks_json, etag))
        })
        .await;

        match etag_result {
            Ok(Ok((json, tag))) => Ok((json, tag)),
            Ok(Err(_)) => {
                error!("Failed to serialize JWKS for ETag calculation");
                Err(Box::new(Self::jwks_error_response()))
            }
            Err(_) => {
                error!("Spawn blocking task panicked during JWKS serialization");
                Err(Box::new(Self::jwks_error_response()))
            }
        }
    }

    /// Create error response for JWKS endpoint
    fn jwks_error_response() -> Response {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "keys": []
            })),
        )
            .into_response()
    }

    /// Check if client has current JWKS version (`ETag` match)
    fn check_etag_match(headers: &HeaderMap, etag: &str) -> bool {
        headers
            .get(header::IF_NONE_MATCH)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|client_etag| client_etag == etag)
    }

    /// Extract Bearer token from Authorization header
    fn extract_bearer_token(headers: &HeaderMap) -> Result<String, Box<Response>> {
        let header = headers.get(header::AUTHORIZATION).ok_or_else(|| {
            warn!("Missing Authorization header");
            Box::new(
                (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "error": "invalid_request",
                        "error_description": "Authorization header is required"
                    })),
                )
                    .into_response(),
            )
        })?;

        let header_str = header.to_str().map_err(|_| {
            Box::new(
                (
                    StatusCode::UNAUTHORIZED,
                    Json(serde_json::json!({
                        "error": "invalid_request",
                        "error_description": "Invalid Authorization header encoding"
                    })),
                )
                    .into_response(),
            )
        })?;

        header_str
            .strip_prefix("Bearer ")
            .map(str::to_owned)
            .ok_or_else(|| {
                warn!("Invalid Authorization header format - missing Bearer prefix");
                Box::new(
                    (
                        StatusCode::UNAUTHORIZED,
                        Json(serde_json::json!({
                            "error": "invalid_request",
                            "error_description": "Authorization header must use Bearer scheme"
                        })),
                    )
                        .into_response(),
                )
            })
    }

    /// Validate Bearer token for token-validate endpoint (returns OK with valid:false on errors).
    ///
    /// A token audience-bound to `resource`, the MCP resource this server mints
    /// for, is as valid as one carrying the platform audience.
    fn validate_bearer_token_for_validate_endpoint(
        headers: &HeaderMap,
        auth_manager: &AuthManager,
        jwks_manager: &JwksManager,
        resources: &[&str],
    ) -> Result<bool, Box<Response>> {
        let Some(header) = headers.get(header::AUTHORIZATION) else {
            // No token provided - not an error, just return false
            return Ok(false);
        };

        let header_str = header.to_str().map_err(|_| {
            Box::new(
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "valid": false,
                        "error": "invalid_request",
                        "error_description": "Invalid Authorization header encoding"
                    })),
                )
                    .into_response(),
            )
        })?;

        let token = header_str.strip_prefix("Bearer ").ok_or_else(|| {
            Box::new(
                (
                    StatusCode::OK,
                    Json(serde_json::json!({
                        "valid": false,
                        "error": "invalid_request",
                        "error_description": "Authorization header must use Bearer scheme"
                    })),
                )
                    .into_response(),
            )
        })?;

        match auth_manager.validate_resource_token(token, jwks_manager, resources) {
            Ok(_) => Ok(true),
            Err(e) => {
                debug!("Token validation failed: {}", e);
                Err(Box::new(
                    (
                        StatusCode::OK,
                        Json(serde_json::json!({
                            "valid": false,
                            "error": "invalid_token",
                            "error_description": "Access token is invalid or expired"
                        })),
                    )
                        .into_response(),
                ))
            }
        }
    }

    /// Build login URL with OAuth parameters preserved for redirect
    fn build_login_url_with_oauth_params(request: &AuthorizeRequest) -> String {
        Self::authorize_params_url("/oauth2/login", request)
    }

    /// Build authorization URL from the login form's fields, with the OAuth
    /// parameters preserved for the redirect
    fn build_authorization_url_from_form(form: &HashMap<String, String>) -> String {
        let field = |name: &str| form.get(name).filter(|v| !v.is_empty()).cloned();
        let request = AuthorizeRequest {
            response_type: field("response_type").unwrap_or_default(),
            client_id: field("client_id").unwrap_or_default(),
            redirect_uri: field("redirect_uri").unwrap_or_default(),
            scope: field("scope"),
            state: field("state"),
            code_challenge: field("code_challenge"),
            code_challenge_method: field("code_challenge_method"),
            resource: field("resource"),
        };
        Self::authorize_params_url("/oauth2/authorize", &request)
    }

    /// `base` carrying the authorization request's parameters, every value
    /// URL-encoded: `client_id`, `redirect_uri`, `response_type` and `state`
    /// always, the scope, PKCE challenge and method and the RFC 8707
    /// resource when present and non-empty. The login page, the Google
    /// sign-in hop and the return to `/oauth2/authorize` all carry a pending
    /// request this one way.
    fn authorize_params_url(base: &str, request: &AuthorizeRequest) -> String {
        use std::fmt::Write;

        let mut url = format!(
            "{base}?client_id={}&redirect_uri={}&response_type={}&state={}",
            urlencoding::encode(&request.client_id),
            urlencoding::encode(&request.redirect_uri),
            urlencoding::encode(&request.response_type),
            urlencoding::encode(request.state.as_deref().unwrap_or(""))
        );
        let optional = [
            ("scope", &request.scope),
            ("code_challenge", &request.code_challenge),
            ("code_challenge_method", &request.code_challenge_method),
            ("resource", &request.resource),
        ];
        for (name, value) in optional {
            if let Some(value) = value.as_deref().filter(|v| !v.is_empty()) {
                write!(&mut url, "&{name}={}", urlencoding::encode(value)).ok();
            }
        }
        url
    }

    /// Parse query parameters into `AuthorizeRequest`
    fn parse_authorize_request(
        params: &HashMap<String, String>,
    ) -> Result<AuthorizeRequest, OAuth2Error> {
        trace!(
            "Parsing OAuth authorize request with {} parameters",
            params.len()
        );

        // A missing response_type is refused by `check_authorize_request`, once
        // the client and redirect_uri are known, so it can reach the client.
        let response_type = params.get("response_type").cloned().unwrap_or_default();

        let client_id = params
            .get("client_id")
            .ok_or_else(|| OAuth2Error::invalid_request("Missing client_id parameter"))?
            .clone(); // Safe: String ownership required for OAuth2 request struct

        let redirect_uri = params
            .get("redirect_uri")
            .ok_or_else(|| OAuth2Error::invalid_request("Missing redirect_uri parameter"))?
            .clone(); // Safe: String ownership required for OAuth2 request struct

        let scope = params.get("scope").cloned();
        let state = params.get("state").cloned();
        let code_challenge = params.get("code_challenge").cloned();
        let code_challenge_method = params.get("code_challenge_method").cloned();
        // The login and consent forms always post a `resource` field, empty
        // when the client named none; empty is absent, not a resource.
        let resource = params.get("resource").filter(|r| !r.is_empty()).cloned();

        Ok(AuthorizeRequest {
            response_type,
            client_id,
            redirect_uri,
            scope,
            state,
            code_challenge,
            code_challenge_method,
            resource,
        })
    }

    /// Parse form data into `TokenRequest`
    fn parse_token_request(form: &HashMap<String, String>) -> Result<TokenRequest, OAuth2Error> {
        let grant_type = form
            .get("grant_type")
            .ok_or_else(|| OAuth2Error::invalid_request("Missing grant_type parameter"))?
            .clone(); // Safe: String ownership required for OAuth2 request struct

        // Client credentials are REQUIRED for all grant types.
        // Pierre MCP clients are confidential clients (RFC 6749 Section 2.1),
        // so client authentication is mandatory including for refresh_token grants.
        let client_id = form
            .get("client_id")
            .ok_or_else(|| OAuth2Error::invalid_request("Missing client_id parameter"))?
            .clone(); // Safe: String ownership for OAuth validation

        let client_secret = form
            .get("client_secret")
            .ok_or_else(|| OAuth2Error::invalid_request("Missing client_secret parameter"))?
            .replace(' ', "+");

        let code = form.get("code").cloned();
        let redirect_uri = form.get("redirect_uri").cloned();
        let scope = form.get("scope").cloned();
        let refresh_token = form.get("refresh_token").cloned();
        let code_verifier = form.get("code_verifier").cloned();
        let resource = form.get("resource").filter(|r| !r.is_empty()).cloned();

        Ok(TokenRequest {
            grant_type,
            code,
            redirect_uri,
            client_id,
            client_secret,
            scope,
            refresh_token,
            code_verifier,
            resource,
        })
    }

    /// Whether this deployment's cookies are `Secure`: its issuer is HTTPS.
    fn cookies_secure(context: &OAuth2Context) -> bool {
        context.config.issuer_url.starts_with("https://")
    }

    /// The name of the authorization server's session cookie here:
    /// `__Host-pierre_session` on an HTTPS issuer, so a sibling host cannot
    /// set one, and `pierre_session` over plain HTTP.
    fn session_cookie_name(context: &OAuth2Context) -> String {
        host_cookie_name(SESSION_COOKIE, Self::cookies_secure(context))
    }

    /// The `Set-Cookie` value carrying a session JWT, the same for the
    /// password and the Google sign-in: `HttpOnly`, `Secure` on an HTTPS
    /// issuer, `SameSite=Lax` so the redirect into `/oauth2/authorize` sends
    /// it, and the JWT's own 24-hour lifetime.
    fn session_cookie(context: &OAuth2Context, token: &str) -> String {
        SecureCookieConfig {
            name: Self::session_cookie_name(context),
            value: token.to_owned(),
            max_age_secs: SESSION_COOKIE_MAX_AGE_SECS,
            http_only: true,
            secure: Self::cookies_secure(context),
            same_site: SameSitePolicy::Lax,
            path: "/".to_owned(),
        }
        .build()
    }

    /// Extract session token from cookie header
    fn extract_session_token(cookie_header: &str, session_cookie: &str) -> Option<String> {
        // Accept the authorization server's own session cookie and, as a
        // bridge, the first-party web app's `auth_token` cookie — both are the same
        // RS256 JWT type validated by the auth manager. This lets a user already
        // logged into the web app authorize an MCP client without a second login.
        // The session cookie wins when both are present.
        let mut app_token = None;
        for cookie in cookie_header.split(';') {
            let Some((name, value)) = cookie.trim().split_once('=') else {
                continue;
            };
            if name == session_cookie {
                return Some(value.to_owned());
            }
            if name == "auth_token" {
                app_token = Some(value.to_owned());
            }
        }
        app_token
    }
}
