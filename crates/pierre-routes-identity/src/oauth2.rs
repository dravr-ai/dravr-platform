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
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use dravr_tronc::dates::Timestamp;
use pierre_auth::admin::jwks::JwksManager;
use pierre_auth::auth::{AuthManager, Claims};
use pierre_auth::config::oauth::OAuth2ServerConfig;
use pierre_auth::dpop::DpopProofs;
use pierre_auth::oauth2_server::{
    client_auth::{is_basic_authorization, resolve_client_credentials},
    client_registration::ClientRegistrationManager,
    endpoints::{AdminAccess, CheckedAuthorization, OAuth2AuthorizationServer},
    first_party::is_first_party,
    models::{
        AuthorizeRequest, ClientRegistrationPayload, OAuth2Error, TokenRequest,
        ValidateRefreshRequest, ValidateRefreshResponse, ValidationStatus,
    },
    rate_limiting::OAuth2RateLimiter,
};
use pierre_auth::rate_limiting::OAuth2Endpoint;
use pierre_auth::security::cookies::auth_cookie_name_on;
use pierre_auth::security::csrf::CsrfTokenManager;
use pierre_contremaitre::MessagingStringsRegistry;
use pierre_core::errors::AppResult;
use pierre_core::models::supported_ui_locale;
use pierre_database::backends::{factory::Database, OAuth2ServerRepository};
use pierre_database::database::repositories::{TenantRepository, UserRepository};
use pierre_database::repositories::OAuthStandingPolicyRepository;
use pierre_services::auth::AuthService;
use secrecy::{ExposeSecret, SecretString};
use std::fmt;
use std::{collections::HashMap, net::SocketAddr, sync::Arc};
use tokio::task::spawn_blocking;
use tracing::{debug, error, info, warn};

use crate::authorize_redirect::{code_redirect, error_redirect, rejection_response};
use crate::oauth2_rate_limited::{page_refusal, refusal};

use self::authorize_params::requests_fresh_login;
use self::pages::ConsentReader;
use self::session::SignInCookie;

/// The consent screen's Admin access box, offered to a live admin (carnet#1511)
mod admin_access;
/// How a pending authorization request travels between the flow's pages
mod authorize_params;
/// The consent form's submission: the user's decision on an authorization
mod consent;
/// What the consent screen says about who is asking and where the data goes
mod consent_copy;
/// Which role lines the consent screen shows the signed-in user (carnet#1511)
mod consent_role;
/// The consent flow's text, in the reader's language
mod consent_text;
/// The DPoP proof a token or pushed authorization request carries (RFC 9449)
mod dpop_proof;
/// Which consent text a grant was approved under, and whether it is current.
mod consent_version;
/// "Continue with Google": server-side Google sign-in on the hosted login page
mod google_login;
/// RFC 7662 token introspection for the resource servers an admin named
mod introspect;
/// The JWKS endpoint serving the RS256 public keys
mod jwks;
/// The hosted login form's submission: a metered password sign-in
mod login_form;
/// The hosted login page's text, in the athlete's language
mod login_text;
/// Server-rendered login, consent and error pages of the authorization flow
mod pages;
/// RFC 9126 pushed authorization requests
mod par;
/// What the consent screen offers to grant, and the review digest of each choice
mod picker;
/// RFC 7009 token revocation
mod revoke;
/// The sign-in cookie: its name, value and reading.
mod session;
/// A standing policy's renewal of a preset grant without the consent screen
mod standing_policy;
/// `POST /oauth2/token-validate`: the SDK's check of its own token
mod token_validate;
/// did:web for the checkpoint key and the newest signed checkpoint (carnet#1542)
mod transparency;
/// What a hosted page shows its reader, as the consent text version reads it.
mod visible_text;
/// The RFC 8414 and RFC 9728 discovery documents
mod well_known;

pub use consent_text::ConsentText;
pub use google_login::GoogleSignIn;
pub use login_text::LoginPageLabels;
pub use pages::{ConsentHtmlParams, ConsentRole, LoginHtmlParams};
pub use picker::GrantChoice;

/// OAuth 2.0 server context shared across all handlers
#[derive(Clone)]
pub struct OAuth2Context {
    /// Database for lifecycle and system settings
    pub database: Arc<Database>,
    /// `OAuth2` server repository for client registration, auth codes, and tokens
    pub oauth2_server: Arc<dyn OAuth2ServerRepository>,
    /// The athletes' standing policies: stored from the consent screen, read
    /// where it would be skipped
    pub standing_policies: Arc<dyn OAuthStandingPolicyRepository>,
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
    /// Verifies and burns the `DPoP` proofs (RFC 9449) PAR and the token
    /// endpoint bind a grant to
    pub dpop_proofs: Arc<DpopProofs>,
    /// Refresh-token lifetime in days, the `REFRESH_TOKEN_EXPIRY_DAYS` sessions read too
    pub refresh_token_expiry_days: i64,
    /// `Max-Age` of the sign-in cookie in seconds, the configured
    /// `SESSION_COOKIE_MAX_AGE_SECS`
    pub session_cookie_max_age_secs: u64,
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
    /// The string catalogue the hosted login page speaks from, the apps' own
    pub strings: Arc<MessagingStringsRegistry>,
    /// Services whose terms keep their data on Dravr's surfaces, named on the
    /// consent screen; read-only, shared by every clone of this context
    pub first_party_only_sources: Arc<[&'static str]>,
}

impl fmt::Debug for OAuth2Context {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuth2Context")
            .field("database", &self.database)
            .field("auth_manager", &self.auth_manager)
            .field("jwks_manager", &self.jwks_manager)
            .field("config", &self.config)
            .field("rate_limiter", &self.rate_limiter)
            .field("refresh_token_expiry_days", &self.refresh_token_expiry_days)
            .field(
                "session_cookie_max_age_secs",
                &self.session_cookie_max_age_secs,
            )
            .field("csrf_manager", &self.csrf_manager)
            .field("accounts", &self.accounts)
            .field("google_sign_in", &self.google_sign_in)
            .field("strings", &self.strings)
            .field("first_party_only_sources", &self.first_party_only_sources)
            .finish_non_exhaustive()
    }
}

/// OAuth 2.0 routes implementation
#[derive(Debug)]
pub struct OAuth2Routes;

/// The `Authorization` header of a token or revocation request, when it is text.
fn authorization_header(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
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
            // RFC 9728 §3.1: the document of a path-bearing resource
            // (`/mcp`, `/a2a`) sits beneath the well-known path
            .route(
                "/.well-known/oauth-protected-resource/{*path}",
                get(Self::handle_protected_resource_metadata_at),
            )
            // RFC 7517: JWKS endpoint
            .route("/.well-known/jwks.json", get(Self::handle_jwks))
            // did:web: the key record_log checkpoints are signed with
            .route("/.well-known/did.json", get(Self::handle_did_document))
            // The newest signed record_log checkpoint (C2SP signed note)
            .route(
                "/.well-known/dravr-checkpoint",
                get(Self::handle_latest_checkpoint),
            )
            // RFC 7591: Dynamic Client Registration
            .route("/oauth2/register", post(Self::handle_client_registration))
            // OAuth 2.0 Authorization endpoint
            .route("/oauth2/authorize", get(Self::handle_authorization))
            // RFC 9126: Pushed Authorization Requests
            .route("/oauth2/par", post(Self::handle_par))
            // OAuth 2.0 Token endpoint
            .route("/oauth2/token", post(Self::handle_token))
            // RFC 7009: Token Revocation
            .route("/oauth2/revoke", post(Self::handle_revoke))
            // RFC 7662: Token Introspection
            .route("/oauth2/introspect", post(Self::handle_introspect))
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
            .with_state(context)
    }

    /// Handle client registration (RFC 7591)
    async fn handle_client_registration(
        State(context): State<OAuth2Context>,
        ConnectInfo(addr): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
        Json(payload): Json<ClientRegistrationPayload>,
    ) -> Response {
        let limiter = &context.rate_limiter;
        if let Some(refused) = refusal(limiter, OAuth2Endpoint::Register, addr, &headers).await {
            return refused;
        }

        // Registrations no user has authorized yet are capped; past the cap the
        // refusal is a 429 like the rate limiter's, in the same body shape.
        let ceiling = context.config.client_retention.max_pending_registrations;
        let client_manager = ClientRegistrationManager::new(context.oauth2_server.clone());

        match client_manager
            .register_client_with_auth_method(payload.request, payload.auth_method, ceiling)
            .await
        {
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
        let browser = ConsentText::for_browser(&context.strings, &headers);
        let render = |error: &OAuth2Error| Self::render_oauth_error_response(error, &browser);
        if let Some(refused) = page_refusal(limiter, addr, &headers, render).await {
            return refused;
        }

        // A pushed request (RFC 9126) is read from the store, and the rest of
        // the query is ignored.
        let resolved = match Self::resolve_authorize_params(&context, params).await {
            Ok(resolved) => resolved,
            Err(error) => {
                let text = Self::reader_text(&context, &headers).await;
                return Self::render_oauth_error_response(&error, &text);
            }
        };
        let params = resolved.params;
        let request = match Self::parse_authorize_request(&params) {
            Ok(req) => req,
            Err(error) => {
                let text = Self::reader_text(&context, &headers).await;
                return Self::render_oauth_error_response(&error, &text);
            }
        };

        // Check the request before the user is asked to log in or consent, so
        // a refusal reaches the client the way RFC 6749 Section 4.1.2.1 sends it.
        let checked = match Self::authorization_server(&context)
            .check_authorize_request(&request)
            .await
        {
            Ok(checked) => checked,
            Err(rejection) => {
                let text = Self::reader_text(&context, &headers).await;
                return rejection_response(&context.config.issuer_url, rejection, &request, &text);
            }
        };

        let redirect_uri = request.redirect_uri.clone();

        // Check if user is authenticated via session cookie. OpenID Connect
        // `prompt=login` asks for the password again whatever session the
        // browser holds: Dravr's apps always send it, so signing out of the
        // app and back in never lands in the account the browser remembers.
        // The login redirect below does not carry `prompt`, so the request
        // the form sends back after the password is checked proceeds.
        let (user_id, tenant_id) = if resolved.honours_prompt && requests_fresh_login(&params) {
            (None, None)
        } else {
            Self::extract_authenticated_user(&headers, &context).await
        };

        // If no authenticated user, redirect to login page with OAuth parameters
        let Some(authenticated_user_id) = user_id else {
            info!("No authenticated session for OAuth authorization, redirecting to login");
            let mut login_url = Self::build_login_url_with_oauth_params(&request);
            // The language the app asked for carries to the page (one of the
            // platform's own locale codes, so nothing to escape)
            if let Some(locale) = supported_ui_locale(params.get("ui_locales").map(String::as_str))
            {
                login_url.push_str("&ui_locales=");
                login_url.push_str(locale);
            }
            return Redirect::to(&login_url).into_response();
        };

        Self::execute_authorization(
            &context,
            request,
            authenticated_user_id,
            tenant_id,
            redirect_uri,
            &checked,
            &headers,
        )
        .await
    }

    async fn extract_authenticated_user(
        headers: &HeaderMap,
        context: &OAuth2Context,
    ) -> (Option<uuid::Uuid>, Option<String>) {
        let Some((cookie, token)) = headers
            .get(header::COOKIE)
            .and_then(|cookie_value| cookie_value.to_str().ok())
            .and_then(|cookie_str| {
                Self::extract_session_token(
                    cookie_str,
                    &Self::session_cookie_name(context),
                    &auth_cookie_name_on(Self::cookies_secure(context)),
                )
            })
        else {
            return (None, None);
        };
        Self::validate_session_token(cookie, &token, context)
            .await
            .map_or((None, None), |(uid, tid)| (Some(uid), tid))
    }

    /// The user and active tenant a sign-in cookie names. An impersonation
    /// token in the web app's cookie is refused, and the refusal written to
    /// the admin audit trail (carnet#1560, carnet#1634).
    async fn validate_session_token(
        cookie: SignInCookie,
        token: &str,
        context: &OAuth2Context,
    ) -> Option<(uuid::Uuid, Option<String>)> {
        let validated = Self::sign_in_claims(cookie, token, context).await;
        match validated {
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

    /// The claims a sign-in cookie carries, read by the validator its
    /// audience takes.
    async fn sign_in_claims(
        cookie: SignInCookie,
        token: &str,
        context: &OAuth2Context,
    ) -> AppResult<Claims> {
        match cookie {
            SignInCookie::AuthorizationServer => context
                .auth_manager
                .validate_authorization_session_token(token, &context.jwks_manager),
            SignInCookie::WebApp => {
                context
                    .auth_manager
                    .validate_session_token(
                        token,
                        &context.jwks_manager,
                        context.database.repositories().admin_audit.as_ref(),
                        "sign in to the OAuth consent screen",
                    )
                    .await
            }
        }
    }

    async fn execute_authorization(
        context: &OAuth2Context,
        request: AuthorizeRequest,
        authenticated_user_id: uuid::Uuid,
        tenant_id: Option<String>,
        redirect_uri: String,
        checked: &CheckedAuthorization,
        headers: &HeaderMap,
    ) -> Response {
        // Dravr's own app is signing the athlete in, not asking for access:
        // no consent screen (carnet#787).
        if is_first_party(&request.client_id) {
            return Self::sign_in_first_party(
                context,
                request,
                authenticated_user_id,
                tenant_id,
                redirect_uri,
                headers,
            )
            .await;
        }

        // Only an active account authorizes a client: a pending one would get
        // a connector every call of which is refused, a suspended one none.
        let user = match Self::account_gate(context, authenticated_user_id, headers).await {
            Ok(user) => user,
            Err(refused) => return *refused,
        };

        // Skip the consent screen only when the user has already approved this client
        // for the requested scope, under the consent text that language reads now;
        // otherwise show consent before any code is minted.
        let grant_tenant =
            Self::resolve_grant_tenant(context, authenticated_user_id, tenant_id.clone()).await;
        let subject = consent_version::ConsentSubject {
            client_id: &request.client_id,
            redirect_uri: &redirect_uri,
            first_party_only_sources: &context.first_party_only_sources,
        };
        let skip_grant = match &grant_tenant {
            Some(tid) => context
                .oauth2_server
                .find_active_client_grant(
                    &authenticated_user_id.to_string(),
                    tid,
                    &request.client_id,
                    &checked.scope,
                )
                .await
                .unwrap_or_else(|e| {
                    error!("Failed to look up the client grant, showing consent: {e}");
                    None
                })
                .filter(|grant| {
                    grant.is_live_at(Timestamp::now())
                        && picker::skip_accepts(&context.strings, grant, checked, subject)
                }),
            None => None,
        };

        // An admin always sees the screen: Admin access is chosen per
        // connection and never granted again silently (carnet#1511, Q20).
        // The code names the grant the screen was skipped under.
        let skippable = !checked.consent_every_time && !user.role.is_admin();
        if let Some(grant) = skip_grant.filter(|_| skippable) {
            return Self::mint_authorization_code(
                context,
                request,
                authenticated_user_id,
                tenant_id,
                redirect_uri,
                AdminAccess::Withheld,
                Some(grant.id),
            )
            .await;
        }
        // No live grant answers: a standing policy may renew the preset grant
        // the request names, under the same gate.
        if let Some(tid) = grant_tenant.as_deref().filter(|_| skippable) {
            if let Some(renewed) =
                Self::renew_under_standing_policy(context, &user, tid, checked, subject).await
            {
                return Self::mint_authorization_code(
                    context,
                    request,
                    authenticated_user_id,
                    tenant_id,
                    redirect_uri,
                    AdminAccess::Withheld,
                    Some(renewed),
                )
                .await;
            }
        }

        let text = ConsentText::for_reader(&context.strings, Some(&user.locale), headers);
        let role = Self::consent_role(context, &user, checked.admin_grantable).await;
        let raw_details = request.authorization_details.as_deref();
        let standing_policy_box = picker::offers_standing_policy_box(&user);
        let choice = match GrantChoice::for_screen(
            checked,
            raw_details,
            &text,
            subject,
            standing_policy_box,
        ) {
            Ok(choice) => choice,
            Err(error) => return Self::render_oauth_error_response(&error, &text),
        };
        // The consent form's synchronizer token, bound to this user: a page
        // on another origin cannot read it, so it cannot post an approval.
        match context.csrf_manager.generate_token(authenticated_user_id) {
            Ok(csrf_token) => Self::render_consent_page(
                &request,
                &csrf_token,
                checked.client_name.as_deref(),
                &context.first_party_only_sources,
                &text,
                ConsentReader {
                    account_email: &user.email,
                    role,
                    grant_choice: &choice,
                },
            ),
            Err(e) => {
                error!("Failed to mint the consent form token: {e}");
                Self::render_oauth_error_response(
                    &OAuth2Error::server_error(consent::CONSENT_UNAVAILABLE),
                    &text,
                )
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
            .first_for_user(user_id)
            .await
            .inspect_err(|e| error!("Failed to list tenants for the client grant: {e}"))
            .ok()
            .flatten()
            .map(|tenant_id| tenant_id.to_string())
    }

    /// The authorization server every route checks, mints and validates
    /// through, minting for the configured MCP resource and satellites.
    fn authorization_server(context: &OAuth2Context) -> OAuth2AuthorizationServer {
        OAuth2AuthorizationServer::from_config(
            &context.config,
            context.oauth2_server.clone(),
            context.tenants.clone(),
            context.users.clone(),
            context.auth_manager.clone(),
            context.jwks_manager.clone(),
            context.refresh_token_expiry_days,
        )
        .with_token_limiter(context.rate_limiter.clone())
    }

    /// Mint an authorization code and redirect back to the client.
    ///
    /// Reached only with a request [`OAuth2AuthorizationServer::check_authorize_request`]
    /// accepted, so the client and `redirect_uri` are verified and a failure
    /// here goes back to the client too. `client_grant_id` is the consent
    /// grant the code is issued under, `None` for Dravr's own apps, which no
    /// consent grant governs.
    async fn mint_authorization_code(
        context: &OAuth2Context,
        request: AuthorizeRequest,
        authenticated_user_id: uuid::Uuid,
        tenant_id: Option<String>,
        redirect_uri: String,
        access: AdminAccess,
        client_grant_id: Option<String>,
    ) -> Response {
        let state = request.state.clone();
        // A pushed request yields one code: spent before it is minted.
        if let Err(error) = Self::spend_pushed_request(context, &request).await {
            let user = context.users.get_global(authenticated_user_id).await;
            let locale = user.ok().flatten().map(|user| user.locale);
            let text = ConsentText::new(&context.strings, locale.as_deref().unwrap_or_default());
            return Self::render_oauth_error_response(&error, &text);
        }
        match Self::authorization_server(context)
            .authorize_with_admin_access(
                request,
                Some(authenticated_user_id),
                tenant_id,
                access,
                client_grant_id,
            )
            .await
        {
            Ok(response) => {
                info!(
                    "OAuth authorization successful for user {}, redirecting with code",
                    authenticated_user_id
                );
                code_redirect(
                    &context.config.issuer_url,
                    &redirect_uri,
                    &response.code,
                    response.state.as_deref(),
                )
            }
            Err(error) => {
                warn!(
                    "OAuth authorization failed for user {}: {:?}",
                    authenticated_user_id, error
                );
                error_redirect(
                    &context.config.issuer_url,
                    &redirect_uri,
                    state.as_deref(),
                    &error,
                )
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

        let basic = is_basic_authorization(authorization_header(&headers));
        let request = match Self::parse_and_log_token_request(&form, &headers) {
            Ok(req) => req,
            Err(error) => return Self::token_error_response(&error, basic),
        };

        // A proof binds what the exchange mints (RFC 9449 §5).
        let dpop_jkt = match Self::proven_dpop_key(&context, &headers, "/oauth2/token").await {
            Ok(jkt) => jkt,
            Err(error) => return Self::token_error_response(&error, basic),
        };
        let auth_server = Self::authorization_server(&context);

        Self::execute_token_exchange(auth_server, request, dpop_jkt.as_deref(), basic).await
    }

    fn parse_and_log_token_request(
        form: &HashMap<String, String>,
        headers: &HeaderMap,
    ) -> Result<TokenRequest, OAuth2Error> {
        debug!(
            "OAuth token request received with grant_type: {:?}, client_id: {:?}",
            form.get("grant_type"),
            form.get("client_id")
        );

        Self::parse_token_request(form, headers).map_err(|error| {
            warn!("OAuth token request parsing failed: {:?}", error);
            error
        })
    }

    async fn execute_token_exchange(
        auth_server: OAuth2AuthorizationServer,
        request: TokenRequest,
        dpop_jkt: Option<&str>,
        basic: bool,
    ) -> Response {
        let client_id = request.client_id.clone();
        match auth_server.token_with_dpop(request, dpop_jkt).await {
            Ok(response) => {
                info!("OAuth token exchange successful for client: {}", client_id);
                (StatusCode::OK, Json(response)).into_response()
            }
            Err(error) => {
                warn!(
                    "OAuth token exchange failed for client {}: {:?}",
                    client_id, error
                );
                Self::token_error_response(&error, basic)
            }
        }
    }

    /// The token endpoint's error response. A client that authenticated with
    /// an `Authorization` header and failed is answered 401 with the scheme it
    /// should retry with (RFC 6749 §5.2); every other refusal keeps the status
    /// the error carries, a throttled one with its wait as `Retry-After`.
    fn token_error_response(error: &OAuth2Error, basic: bool) -> Response {
        let body = Json(error);
        if basic && error.error == "invalid_client" {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Basic realm=\"oauth2\"")],
                body,
            )
                .into_response();
        }
        let mut response = (error.http_status(), body).into_response();
        if let Some(wait) = error.retry_after {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(wait));
        }
        response
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
                Self::validate_and_refresh_response(response)
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

    /// The verdict on the wire: `200` for anything said about the token, and
    /// the window's own status with its `Retry-After` when nothing was, so a
    /// client neither signs the athlete out nor retries at once.
    fn validate_and_refresh_response(verdict: ValidateRefreshResponse) -> Response {
        if verdict.status != ValidationStatus::RetryLater {
            return (StatusCode::OK, Json(verdict)).into_response();
        }
        let status = if verdict.retry_after.is_some() {
            StatusCode::TOO_MANY_REQUESTS
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        };
        let wait = verdict.retry_after;
        let mut response = (status, Json(verdict)).into_response();
        if let Some(wait) = wait {
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from(wait));
        }
        response
    }

    /// Handle OAuth login page (GET /oauth2/login)
    async fn handle_oauth_login_page(
        State(context): State<OAuth2Context>,
        Query(params): Query<HashMap<String, String>>,
        headers: HeaderMap,
    ) -> Html<String> {
        let labels = login_text::LoginText::for_client(
            &context.strings,
            &headers,
            params.get("ui_locales").map(String::as_str),
            params.get("client_id").map(String::as_str),
        )
        .labels();
        let lang = labels.lang.clone();
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
        let dpop_jkt = params
            .get("dpop_jkt")
            .map_or_else(String::new, ToString::to_string);
        let request_uri = params
            .get("request_uri")
            .map_or_else(String::new, ToString::to_string);
        let authorization_details = params
            .get("authorization_details")
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
            .as_ref()
            .map_or_default(|password| password.expose_secret().to_owned());

        // "Continue with Google", on the issuer's host where Google returns,
        // for a pending authorization this page can name
        let google_start_url = context
            .google_sign_in
            .as_ref()
            .and_then(|_| {
                Self::pushed_reference(&params)
                    .or_else(|| Self::parse_authorize_request(&params).ok())
            })
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
                dpop_jkt: &dpop_jkt,
                request_uri: &request_uri,
                authorization_details: &authorization_details,
                default_email: &default_email,
                default_password: &default_password,
                google_start_url: google_start_url.as_deref(),
                labels: &labels,
            })
        })
        .await
        .unwrap_or_else(|_| Self::generic_error_html(&ConsentText::new(&context.strings, &lang)));

        Html(html)
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

    /// Parse form data into `TokenRequest`
    fn parse_token_request(
        form: &HashMap<String, String>,
        headers: &HeaderMap,
    ) -> Result<TokenRequest, OAuth2Error> {
        let grant_type = form
            .get("grant_type")
            .ok_or_else(|| OAuth2Error::invalid_request("Missing grant_type parameter"))?
            .clone(); // Safe: String ownership required for OAuth2 request struct

        // The client identifies itself on every grant type: by an
        // `Authorization: Basic` header (`client_secret_basic`), by the form
        // body (`client_secret_post`), or, for a public client, by its
        // `client_id` alone (`none`, held to PKCE). Whether the client may
        // authenticate the way it did is decided against what it registered.
        let credentials = resolve_client_credentials(
            authorization_header(headers),
            form.get("client_id").map(String::as_str),
            form.get("client_secret").map(String::as_str),
        )?;
        let client_id = credentials.client_id;
        let client_secret = credentials.client_secret;

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
            refresh_token: refresh_token.map(SecretString::from),
            code_verifier,
            resource,
        })
    }
}
