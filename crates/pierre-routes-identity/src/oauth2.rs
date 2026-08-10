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
use pierre_auth::oauth2_server::{
    client_registration::ClientRegistrationManager,
    endpoints::OAuth2AuthorizationServer,
    models::{
        AuthorizeRequest, ClientRegistrationRequest, OAuth2Error, TokenRequest,
        ValidateRefreshRequest,
    },
    rate_limiting::OAuth2RateLimiter,
};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::OAuthClientGrant;
use pierre_database::backends::{factory::Database, OAuth2ServerRepository};
use pierre_database::database::repositories::{TenantRepository, UserRepository};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::Arc,
};
use tokio::task::spawn_blocking;
use tracing::{debug, error, info, trace, warn};

/// Escape a string for safe insertion into HTML attribute values.
///
/// Replaces the five HTML-special characters (`&`, `<`, `>`, `"`, `'`)
/// with their corresponding HTML entities to prevent attribute breakout
/// and script injection when rendering the OAuth login form.
fn escape_html_attribute(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => output.push_str("&amp;"),
            '<' => output.push_str("&lt;"),
            '>' => output.push_str("&gt;"),
            '"' => output.push_str("&quot;"),
            '\'' => output.push_str("&#x27;"),
            _ => output.push(ch),
        }
    }
    output
}

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
}

/// OAuth 2.0 routes implementation
pub struct OAuth2Routes;

/// Parameters for generating OAuth login HTML
#[derive(Clone, Copy)]
pub struct LoginHtmlParams<'a> {
    /// OAuth client identifier
    pub client_id: &'a str,
    /// OAuth redirect URI after authorization
    pub redirect_uri: &'a str,
    /// OAuth response type (typically "code")
    pub response_type: &'a str,
    /// OAuth state parameter for CSRF protection
    pub state: &'a str,
    /// OAuth scope for requested permissions
    pub scope: &'a str,
    /// PKCE code challenge
    pub code_challenge: &'a str,
    /// PKCE code challenge method (e.g., "S256")
    pub code_challenge_method: &'a str,
    /// Default email to pre-fill in login form (dev/test only)
    pub default_email: &'a str,
    /// Default password to pre-fill in login form (dev/test only)
    pub default_password: &'a str,
}

/// Parameters for rendering the OAuth consent page.
#[derive(Clone, Copy)]
pub struct ConsentHtmlParams<'a> {
    /// OAuth client identifier requesting access
    pub client_id: &'a str,
    /// OAuth redirect URI after authorization
    pub redirect_uri: &'a str,
    /// OAuth response type (typically "code")
    pub response_type: &'a str,
    /// OAuth state parameter for CSRF protection
    pub state: &'a str,
    /// OAuth scope being consented to
    pub scope: &'a str,
    /// PKCE code challenge
    pub code_challenge: &'a str,
    /// PKCE code challenge method (e.g., "S256")
    pub code_challenge_method: &'a str,
}

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

    /// Handle OAuth 2.0 discovery (RFC 8414)
    async fn handle_discovery(State(context): State<OAuth2Context>) -> Json<serde_json::Value> {
        let issuer_url = context.config.issuer_url.clone();

        // Use spawn_blocking for JSON serialization (CPU-bound operation)
        let discovery_json = spawn_blocking(move || {
            serde_json::json!({
                "issuer": issuer_url,
                "authorization_endpoint": format!("{issuer_url}/oauth2/authorize"),
                "token_endpoint": format!("{issuer_url}/oauth2/token"),
                "registration_endpoint": format!("{issuer_url}/oauth2/register"),
                "jwks_uri": format!("{issuer_url}/.well-known/jwks.json"),
                "grant_types_supported": ["authorization_code", "client_credentials", "refresh_token"],
                "response_types_supported": ["code"],
                "token_endpoint_auth_methods_supported": ["client_secret_post"],
                "scopes_supported": ["fitness:read", "activities:read", "profile:read"],
                "response_modes_supported": ["query"],
                "code_challenge_methods_supported": ["S256"]
            })
        })
        .await
        .unwrap_or_else(|_| {
            serde_json::json!({
                "error": "internal_error",
                "error_description": "Failed to generate discovery document"
            })
        });

        Json(discovery_json)
    }

    /// Handle OAuth 2.0 Protected Resource Metadata discovery (RFC 9728).
    ///
    /// The MCP authorization spec requires a protected MCP server to act as an
    /// OAuth 2.1 resource server and publish this document so clients can locate
    /// the authorization server after receiving a 401. The `WWW-Authenticate`
    /// header on `/mcp` 401 responses points here via its `resource_metadata`
    /// parameter. Pierre hosts its own authorization server, so `resource` and
    /// `authorization_servers` both resolve to the issuer base URL.
    async fn handle_protected_resource_metadata(
        State(context): State<OAuth2Context>,
    ) -> Json<serde_json::Value> {
        let issuer_url = context.config.issuer_url.clone();

        // Use spawn_blocking for JSON serialization (CPU-bound operation)
        let metadata_json = spawn_blocking(move || {
            serde_json::json!({
                "resource": issuer_url,
                "authorization_servers": [issuer_url],
                "jwks_uri": format!("{issuer_url}/.well-known/jwks.json"),
                "scopes_supported": ["fitness:read", "activities:read", "profile:read"],
                "bearer_methods_supported": ["header"]
            })
        })
        .await
        .unwrap_or_else(|_| {
            serde_json::json!({
                "error": "internal_error",
                "error_description": "Failed to generate protected resource metadata"
            })
        });

        Json(metadata_json)
    }

    /// Handle client registration (RFC 7591)
    async fn handle_client_registration(
        State(context): State<OAuth2Context>,
        ConnectInfo(addr): ConnectInfo<SocketAddr>,
        Json(request): Json<ClientRegistrationRequest>,
    ) -> Response {
        // Extract client IP from connection using Axum's ConnectInfo extractor
        let client_ip = addr.ip();
        let rate_status = context.rate_limiter.check_rate_limit("register", client_ip);

        if rate_status.is_limited {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                Json(serde_json::json!({
                    "error": "too_many_requests",
                    "error_description": "Rate limit exceeded"
                })),
            )
                .into_response();
        }

        let client_manager = ClientRegistrationManager::new(context.oauth2_server.clone());

        match client_manager.register_client(request).await {
            Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
            Err(error) => (StatusCode::BAD_REQUEST, Json(error)).into_response(),
        }
    }

    /// Handle authorization request (GET /oauth2/authorize)
    async fn handle_authorization(
        State(context): State<OAuth2Context>,
        ConnectInfo(addr): ConnectInfo<SocketAddr>,
        Query(params): Query<HashMap<String, String>>,
        headers: HeaderMap,
    ) -> Response {
        // Extract client IP from connection using Axum's ConnectInfo extractor
        let client_ip = addr.ip();
        let rate_status = context
            .rate_limiter
            .check_rate_limit("authorize", client_ip);

        if rate_status.is_limited {
            return Self::render_oauth_error_response(&OAuth2Error {
                error: "too_many_requests".to_owned(),
                error_description: Some("Rate limit exceeded".to_owned()),
                error_uri: None,
            });
        }

        // Parse query parameters into AuthorizeRequest
        let request = match Self::parse_authorize_request(&params) {
            Ok(req) => req,
            Err(error) => return Self::render_oauth_error_response(&error),
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
                    Self::extract_session_token(cookie_str)
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
            .validate_token(token, &context.jwks_manager)
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
    ) -> Response {
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
                .unwrap_or(None)
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

        Self::render_consent_page(&request)
    }

    /// Render the consent screen for a not-yet-granted authorization request.
    fn render_consent_page(request: &AuthorizeRequest) -> Response {
        let html = Self::generate_consent_html(ConsentHtmlParams {
            client_id: &request.client_id,
            redirect_uri: &request.redirect_uri,
            response_type: &request.response_type,
            state: request.state.as_deref().unwrap_or_default(),
            scope: request.scope.as_deref().unwrap_or_default(),
            code_challenge: request.code_challenge.as_deref().unwrap_or_default(),
            code_challenge_method: request.code_challenge_method.as_deref().unwrap_or_default(),
        });
        Html(html).into_response()
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
            .ok()
            .and_then(|tenants| tenants.first().map(|t| t.id.to_string()))
    }

    /// Mint an authorization code and redirect back to the client.
    async fn mint_authorization_code(
        context: &OAuth2Context,
        request: AuthorizeRequest,
        authenticated_user_id: uuid::Uuid,
        tenant_id: Option<String>,
        redirect_uri: String,
    ) -> Response {
        let auth_server = OAuth2AuthorizationServer::new(
            context.oauth2_server.clone(),
            context.tenants.clone(),
            context.users.clone(),
            context.auth_manager.clone(),
            context.jwks_manager.clone(),
        );

        match auth_server
            .authorize(request, Some(authenticated_user_id), tenant_id)
            .await
        {
            Ok(response) => {
                let mut final_redirect_url = format!(
                    "{}?code={}",
                    redirect_uri,
                    urlencoding::encode(&response.code)
                );
                if let Some(state) = response.state {
                    use std::fmt::Write;
                    write!(
                        &mut final_redirect_url,
                        "&state={}",
                        urlencoding::encode(&state)
                    )
                    .ok();
                }

                info!(
                    "OAuth authorization successful for user {}, redirecting with code",
                    authenticated_user_id
                );

                Redirect::to(&final_redirect_url).into_response()
            }
            Err(error) => {
                error!(
                    "OAuth authorization failed for user {}: {:?}",
                    authenticated_user_id, error
                );
                Self::render_oauth_error_response(&error)
            }
        }
    }

    /// Handle the OAuth consent form submission (POST /oauth2/consent).
    ///
    /// On approval, records a durable grant (so future authorizations for the same
    /// client + scope skip the prompt) and mints the authorization code. On denial,
    /// returns `access_denied` to the client's registered redirect URI.
    async fn handle_consent_submit(
        State(context): State<OAuth2Context>,
        headers: HeaderMap,
        Form(form): Form<HashMap<String, String>>,
    ) -> Response {
        let request = match Self::parse_authorize_request(&form) {
            Ok(req) => req,
            Err(error) => return Self::render_oauth_error_response(&error),
        };
        let redirect_uri = request.redirect_uri.clone();

        // The consent decision must belong to the authenticated user.
        let (user_id, tenant_id) = Self::extract_authenticated_user(&headers, &context);
        let Some(user_id) = user_id else {
            // Session lapsed between render and submit — restart through login.
            return Redirect::to(&Self::build_login_url_with_oauth_params(&request))
                .into_response();
        };

        let approved = form
            .get("decision")
            .is_some_and(|decision| decision == "approve");
        if !approved {
            return Self::deny_authorization(&context, &request, &redirect_uri).await;
        }

        // Record the grant; a duplicate active grant is a no-op at the storage layer.
        let scope = request.scope.clone().unwrap_or_default();
        if let Some(tid) = Self::resolve_grant_tenant(&context, user_id, tenant_id.clone()).await {
            let grant = OAuthClientGrant {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: user_id.to_string(),
                tenant_id: tid,
                client_id: request.client_id.clone(),
                scope,
                granted_at: chrono::Utc::now(),
                revoked_at: None,
            };
            if let Err(e) = context.oauth2_server.store_client_grant(&grant).await {
                warn!("Failed to persist OAuth client grant: {e}");
            }
        }

        Self::mint_authorization_code(&context, request, user_id, tenant_id, redirect_uri).await
    }

    /// Bounce an `access_denied` error to the client, guarding against open redirect.
    async fn deny_authorization(
        context: &OAuth2Context,
        request: &AuthorizeRequest,
        redirect_uri: &str,
    ) -> Response {
        // Only redirect to a redirect_uri actually registered for the client.
        let registered = context
            .tenants
            .get_oauth_app_by_client_id(&request.client_id)
            .await
            .is_ok_and(|app| app.redirect_uris.iter().any(|u| u == redirect_uri));
        if !registered {
            return Self::render_oauth_error_response(&OAuth2Error::invalid_request(
                "Invalid redirect_uri",
            ));
        }
        let mut url = format!("{redirect_uri}?error=access_denied");
        if let Some(state) = &request.state {
            use std::fmt::Write;
            write!(&mut url, "&state={}", urlencoding::encode(state)).ok();
        }
        Redirect::to(&url).into_response()
    }

    /// Handle token request (POST /oauth2/token)
    async fn handle_token(
        State(context): State<OAuth2Context>,
        ConnectInfo(addr): ConnectInfo<SocketAddr>,
        Form(form): Form<HashMap<String, String>>,
    ) -> Response {
        // Extract client IP from connection using Axum's ConnectInfo extractor
        let client_ip = addr.ip();

        if let Some(rate_limit_response) = Self::check_token_rate_limit(&context, client_ip) {
            return rate_limit_response;
        }

        let request = match Self::parse_and_log_token_request(&form) {
            Ok(req) => req,
            Err(error) => return (StatusCode::BAD_REQUEST, Json(error)).into_response(),
        };

        let auth_server = OAuth2AuthorizationServer::new(
            context.oauth2_server.clone(),
            context.tenants.clone(),
            context.users,
            context.auth_manager,
            context.jwks_manager,
        );

        Self::execute_token_exchange(auth_server, request, &form).await
    }

    fn check_token_rate_limit(context: &OAuth2Context, client_ip: IpAddr) -> Option<Response> {
        let rate_status = context.rate_limiter.check_rate_limit("token", client_ip);

        if rate_status.is_limited {
            Some(
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    Json(serde_json::json!({
                        "error": "too_many_requests",
                        "error_description": "Rate limit exceeded"
                    })),
                )
                    .into_response(),
            )
        } else {
            None
        }
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
                (StatusCode::BAD_REQUEST, Json(error)).into_response()
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

        let auth_server = OAuth2AuthorizationServer::new(
            context.oauth2_server.clone(),
            context.tenants.clone(),
            context.users,
            context.auth_manager,
            context.jwks_manager,
        );

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
                debug!("Client validation failed for {}: {}", client_id, e);
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

    /// Generate OAuth login page HTML
    #[must_use]
    pub fn generate_login_html(params: LoginHtmlParams<'_>) -> String {
        // Use embedded template - zero filesystem IO, guaranteed to exist at compile-time
        let displayed_scope = if params.scope.is_empty() {
            "fitness:read activities:read profile:read"
        } else {
            params.scope
        };

        Self::OAUTH_LOGIN_TEMPLATE
            .replace("{{CLIENT_ID}}", &escape_html_attribute(params.client_id))
            .replace(
                "{{REDIRECT_URI}}",
                &escape_html_attribute(params.redirect_uri),
            )
            .replace(
                "{{RESPONSE_TYPE}}",
                &escape_html_attribute(params.response_type),
            )
            .replace("{{STATE}}", &escape_html_attribute(params.state))
            .replace("{{SCOPE}}", &escape_html_attribute(displayed_scope))
            .replace(
                "{{CODE_CHALLENGE}}",
                &escape_html_attribute(params.code_challenge),
            )
            .replace(
                "{{CODE_CHALLENGE_METHOD}}",
                &escape_html_attribute(params.code_challenge_method),
            )
            .replace(
                "{{DEFAULT_EMAIL}}",
                &escape_html_attribute(params.default_email),
            )
            .replace(
                "{{DEFAULT_PASSWORD}}",
                &escape_html_attribute(params.default_password),
            )
    }

    /// Render the OAuth consent page for a client requesting access.
    #[must_use]
    pub fn generate_consent_html(params: ConsentHtmlParams<'_>) -> String {
        use std::fmt::Write;

        let displayed_scope = if params.scope.is_empty() {
            "fitness:read activities:read profile:read"
        } else {
            params.scope
        };
        // Each scope token becomes a list item; the token text is HTML-escaped
        // before the (literal) <li> markup is substituted into the template.
        let scope_items =
            displayed_scope
                .split_whitespace()
                .fold(String::new(), |mut acc, scope| {
                    write!(acc, "<li>{}</li>", escape_html_attribute(scope)).ok();
                    acc
                });

        Self::OAUTH_CONSENT_TEMPLATE
            .replace("{{CLIENT_ID}}", &escape_html_attribute(params.client_id))
            .replace(
                "{{REDIRECT_URI}}",
                &escape_html_attribute(params.redirect_uri),
            )
            .replace(
                "{{RESPONSE_TYPE}}",
                &escape_html_attribute(params.response_type),
            )
            .replace("{{STATE}}", &escape_html_attribute(params.state))
            .replace("{{SCOPE}}", &escape_html_attribute(displayed_scope))
            .replace(
                "{{CODE_CHALLENGE}}",
                &escape_html_attribute(params.code_challenge),
            )
            .replace(
                "{{CODE_CHALLENGE_METHOD}}",
                &escape_html_attribute(params.code_challenge_method),
            )
            .replace("{{SCOPE_ITEMS}}", &scope_items)
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
                default_email: &default_email,
                default_password: &default_password,
            })
        })
        .await
        .unwrap_or_else(|_| {
            "<html><body><h1>Error</h1><p>Failed to generate login page</p></body></html>"
                .to_owned()
        });

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

        // Authenticate user using database lookup and password verification
        match Self::authenticate_user_with_auth_manager(
            context.users.as_ref(),
            context.tenants.as_ref(),
            email,
            password,
            &context.auth_manager,
            &context.jwks_manager,
        )
        .await
        {
            Ok(token) => {
                // Extract OAuth parameters from form to continue authorization flow (including PKCE)
                let client_id = form.get("client_id").map_or("", |v| v);
                let redirect_uri = form.get("redirect_uri").map_or("", |v| v);
                let response_type = form.get("response_type").map_or("", |v| v);
                let state = form.get("state").map_or("", |v| v);
                let scope = form.get("scope").map_or("", |v| v);
                let code_challenge = form.get("code_challenge").map_or("", |v| v);
                let code_challenge_method = form.get("code_challenge_method").map_or("", |v| v);

                let auth_url = Self::build_authorization_url_from_form(
                    client_id,
                    redirect_uri,
                    response_type,
                    state,
                    scope,
                    code_challenge,
                    code_challenge_method,
                );

                info!(
                    "User {} authenticated successfully for OAuth, redirecting to authorization",
                    email
                );

                // Set session cookie and redirect to authorization endpoint
                // Cookie security: HttpOnly prevents XSS, Secure enforces HTTPS, SameSite=Lax prevents CSRF
                // Max-Age matches JWT expiration (24 hours = 86400 seconds)
                // Only set Secure flag when issuer URL uses HTTPS (allows HTTP in development)
                let secure_flag = if context.config.issuer_url.starts_with("https://") {
                    "; Secure"
                } else {
                    ""
                };
                let cookie_header = format!(
                    "pierre_session={token}; HttpOnly{secure_flag}; Path=/; SameSite=Lax; Max-Age=86400"
                );

                (
                    StatusCode::FOUND,
                    [
                        (header::LOCATION, auth_url),
                        (header::SET_COOKIE, cookie_header),
                    ],
                )
                    .into_response()
            }
            Err(e) => {
                warn!("Authentication failed for OAuth login: {}", e);

                // Use embedded template - zero filesystem IO, guaranteed to exist at compile-time
                // Values go into an <a href> URL attribute — URL-encode for URL
                // correctness, then HTML-escape for attribute safety (XSS prevention)
                let error_html = Self::OAUTH_LOGIN_ERROR_TEMPLATE
                    .replace(
                        "{{ERROR_MESSAGE}}",
                        &escape_html_attribute(
                            "Authentication Failed: Invalid email or password. Please try again.",
                        ),
                    )
                    .replace(
                        "{{CLIENT_ID}}",
                        &escape_html_attribute(
                            urlencoding::encode(form.get("client_id").map_or("", |v| v)).as_ref(),
                        ),
                    )
                    .replace(
                        "{{REDIRECT_URI}}",
                        &escape_html_attribute(
                            urlencoding::encode(form.get("redirect_uri").map_or("", |v| v))
                                .as_ref(),
                        ),
                    )
                    .replace(
                        "{{RESPONSE_TYPE}}",
                        &escape_html_attribute(
                            urlencoding::encode(form.get("response_type").map_or("", |v| v))
                                .as_ref(),
                        ),
                    )
                    .replace(
                        "{{STATE}}",
                        &escape_html_attribute(
                            urlencoding::encode(form.get("state").map_or("", |v| v)).as_ref(),
                        ),
                    )
                    .replace(
                        "{{SCOPE}}",
                        &escape_html_attribute(
                            urlencoding::encode(form.get("scope").map_or("", |v| v)).as_ref(),
                        ),
                    )
                    .replace(
                        "{{CODE_CHALLENGE}}",
                        &escape_html_attribute(
                            urlencoding::encode(form.get("code_challenge").map_or("", |v| v))
                                .as_ref(),
                        ),
                    )
                    .replace(
                        "{{CODE_CHALLENGE_METHOD}}",
                        &escape_html_attribute(
                            urlencoding::encode(
                                form.get("code_challenge_method").map_or("", |v| v),
                            )
                            .as_ref(),
                        ),
                    );

                (StatusCode::UNAUTHORIZED, Html(error_html)).into_response()
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
            Err(response) => return response,
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
    async fn compute_jwks_etag(jwks: JsonWebKeySet) -> Result<(String, String), Response> {
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
                Err(Self::jwks_error_response())
            }
            Err(_) => {
                error!("Spawn blocking task panicked during JWKS serialization");
                Err(Self::jwks_error_response())
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

    /// Validate Bearer token for token-validate endpoint (returns OK with valid:false on errors)
    fn validate_bearer_token_for_validate_endpoint(
        headers: &HeaderMap,
        auth_manager: &AuthManager,
        jwks_manager: &JwksManager,
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

        match auth_manager.validate_token(token, jwks_manager) {
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
        let mut login_url = format!(
            "/oauth2/login?client_id={}&redirect_uri={}&response_type={}&state={}",
            urlencoding::encode(&request.client_id),
            urlencoding::encode(&request.redirect_uri),
            urlencoding::encode(&request.response_type),
            urlencoding::encode(request.state.as_deref().unwrap_or(""))
        );

        if let Some(ref scope) = request.scope {
            use std::fmt::Write;
            write!(&mut login_url, "&scope={}", urlencoding::encode(scope)).ok();
        }

        if let Some(ref code_challenge) = request.code_challenge {
            use std::fmt::Write;
            write!(
                &mut login_url,
                "&code_challenge={}",
                urlencoding::encode(code_challenge)
            )
            .ok();
        }

        if let Some(ref code_challenge_method) = request.code_challenge_method {
            use std::fmt::Write;
            write!(
                &mut login_url,
                "&code_challenge_method={code_challenge_method}"
            )
            .ok();
        }

        login_url
    }

    /// Build authorization URL from form data with OAuth parameters preserved for redirect
    fn build_authorization_url_from_form(
        client_id: &str,
        redirect_uri: &str,
        response_type: &str,
        state: &str,
        scope: &str,
        code_challenge: &str,
        code_challenge_method: &str,
    ) -> String {
        let mut auth_url = format!(
            "/oauth2/authorize?client_id={}&redirect_uri={}&response_type={}&state={}",
            urlencoding::encode(client_id),
            urlencoding::encode(redirect_uri),
            urlencoding::encode(response_type),
            urlencoding::encode(state)
        );

        if !scope.is_empty() {
            use std::fmt::Write;
            write!(&mut auth_url, "&scope={}", urlencoding::encode(scope)).ok();
        }

        if !code_challenge.is_empty() {
            use std::fmt::Write;
            write!(
                &mut auth_url,
                "&code_challenge={}",
                urlencoding::encode(code_challenge)
            )
            .ok();
        }

        if !code_challenge_method.is_empty() {
            use std::fmt::Write;
            write!(
                &mut auth_url,
                "&code_challenge_method={code_challenge_method}"
            )
            .ok();
        }

        auth_url
    }

    /// Parse query parameters into `AuthorizeRequest`
    fn parse_authorize_request(
        params: &HashMap<String, String>,
    ) -> Result<AuthorizeRequest, OAuth2Error> {
        trace!(
            "Parsing OAuth authorize request with {} parameters",
            params.len()
        );

        let response_type = params
            .get("response_type")
            .ok_or_else(|| OAuth2Error::invalid_request("Missing response_type parameter"))?
            .clone(); // Safe: String ownership required for OAuth2 request struct

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

        Ok(AuthorizeRequest {
            response_type,
            client_id,
            redirect_uri,
            scope,
            state,
            code_challenge,
            code_challenge_method,
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

        Ok(TokenRequest {
            grant_type,
            code,
            redirect_uri,
            client_id,
            client_secret,
            scope,
            refresh_token,
            code_verifier,
        })
    }

    /// Authenticate user credentials using `AuthManager` (proper architecture)
    async fn authenticate_user_with_auth_manager(
        users: &dyn UserRepository,
        tenants_repo: &dyn TenantRepository,
        email: &str,
        password: &str,
        auth_manager: &AuthManager,
        jwks_manager: &JwksManager,
    ) -> AppResult<String> {
        // Look up user by email
        let user = users
            .get_by_email(email)
            .await
            .map_err(|e| AppError::database(e.to_string()))?
            .ok_or_else(|| AppError::not_found("User not found"))?;

        // Verify password hash
        if !Self::verify_password(password, &user.password_hash).await? {
            return Err(AppError::auth_invalid("Invalid password"));
        }

        // Look up user's default tenant to include in JWT as active_tenant_id
        let tenants = tenants_repo
            .list_for_user(user.id)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user tenants: {e}")))?;
        let active_tenant_id = tenants.first().map(|t| t.id.to_string());

        // Use AuthManager to generate JWT token with RS256 and active_tenant_id
        // This ensures consistent JWT handling across the entire system
        let token = auth_manager
            .generate_token_with_tenant(&user, jwks_manager, active_tenant_id)
            .map_err(|e| AppError::internal(format!("Token generation failed: {e}")))?;

        Ok(token)
    }

    /// Verify password against hash using bcrypt with `spawn_blocking`
    ///
    /// Uses `tokio::task::spawn_blocking` to avoid blocking the async executor
    /// with CPU-intensive bcrypt operations.
    async fn verify_password(password: &str, hash: &str) -> AppResult<bool> {
        let password = password.to_owned();
        let hash = hash.to_owned();

        let result = spawn_blocking(move || bcrypt::verify(&password, &hash))
            .await
            .map_err(|e| {
                error!("Password verification task panicked: {e}");
                AppError::internal("Password verification failed")
            })?
            .map_err(|e| {
                error!("bcrypt verification error: {e}");
                AppError::internal("Password verification failed")
            })?;

        Ok(result)
    }

    /// Extract session token from cookie header
    fn extract_session_token(cookie_header: &str) -> Option<String> {
        // Accept the authorization server's own `pierre_session` cookie and, as a
        // bridge, the first-party web app's `auth_token` cookie — both are the same
        // RS256 JWT type validated by the auth manager. This lets a user already
        // logged into the web app authorize an MCP client without a second login.
        // `pierre_session` wins when both are present.
        let mut app_token = None;
        for cookie in cookie_header.split(';') {
            let cookie = cookie.trim();
            if let Some(session_token) = cookie.strip_prefix("pierre_session=") {
                return Some(session_token.to_owned());
            }
            if let Some(auth_token) = cookie.strip_prefix("auth_token=") {
                app_token = Some(auth_token.to_owned());
            }
        }
        app_token
    }

    /// OAuth error template embedded at compile-time
    const OAUTH_ERROR_TEMPLATE: &'static str = include_str!("../templates/oauth_error.html");

    /// OAuth login page template embedded at compile-time
    /// Loaded with `include_str`!() to avoid blocking filesystem IO at runtime
    const OAUTH_LOGIN_TEMPLATE: &'static str = include_str!("../templates/oauth_login.html");

    /// OAuth consent page template embedded at compile-time
    const OAUTH_CONSENT_TEMPLATE: &'static str = include_str!("../templates/oauth_consent.html");

    /// OAuth login error template embedded at compile-time
    /// Loaded with `include_str`!() to avoid blocking filesystem IO at runtime
    const OAUTH_LOGIN_ERROR_TEMPLATE: &'static str =
        include_str!("../templates/oauth_login_error.html");

    /// Render HTML error page for OAuth errors shown in browser
    fn render_oauth_error_response(error: &OAuth2Error) -> Response {
        let error_title = match error.error.as_str() {
            "invalid_client" => "✗ Invalid Client",
            "unauthorized_client" => "✗ Unauthorized Client",
            "access_denied" => "✗ Access Denied",
            "unsupported_response_type" => "✗ Unsupported Response Type",
            "invalid_scope" => "✗ Invalid Scope",
            "server_error" => "✗ Server Error",
            "temporarily_unavailable" => "✗ Temporarily Unavailable",
            _ => "✗ OAuth Error",
        };

        let default_description =
            "An error occurred during the OAuth authorization process.".to_owned();
        let error_description = error
            .error_description
            .as_ref()
            .unwrap_or(&default_description);

        let html = Self::OAUTH_ERROR_TEMPLATE
            .replace("{{error_title}}", &escape_html_attribute(error_title))
            .replace("{{ERROR}}", &escape_html_attribute(&error.error))
            .replace("{{PROVIDER}}", "Dravr")
            .replace(
                "{{DESCRIPTION}}",
                &format!(
                    r#"<div class="description">{}</div>"#,
                    escape_html_attribute(error_description)
                ),
            );

        (StatusCode::BAD_REQUEST, Html(html)).into_response()
    }
}
