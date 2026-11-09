// ABOUTME: OAuth 2.0 authorization and token endpoints implementation
// ABOUTME: Handles OAuth 2.0 flow with JWT tokens as access tokens for MCP client compatibility
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// NOTE: All `.clone()` calls in this file are Safe - they are necessary for:
// - String ownership transfers to struct constructors (OAuth2AuthCode, TokenResponse)
// - Arc clone for database manager creation

use super::cimd::{self, MetadataFetcher, MetadataResolver};
use super::client_registration::{
    is_public_client, redirect_uri_matches, redirects_to_any_local_process,
    ClientRegistrationManager,
};
use super::first_party::{is_first_party, FirstPartyRedirects};
use super::models::{
    AuthorizeRejection, AuthorizeRequest, AuthorizeResponse, OAuth2AuthCode, OAuth2Client,
    OAuth2Error, TokenRequest, TokenResponse,
};
use super::pkce::{check_code_challenge, check_pkce};
use super::rate_limiting::OAuth2RateLimiter;
use super::request_text::{refuse_control_characters, refuse_oversized_state};
use super::resource::bound_audience;
use crate::admin::jwks::JwksManager;
use crate::auth::{AuthManager, DelegatedGrant};
use crate::config::oauth::ClientRetentionConfig;
use crate::config::OAuth2ServerConfig;
use crate::refresh_rotation::{generate_refresh_token, refresh_token_lifetime};
use base64::{engine::general_purpose, Engine as _};
use chrono::{Duration, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::redaction;
use pierre_database::backends::{OAuth2ServerRepository, TenantRepository, UserRepository};
use ring::rand::{SecureRandom, SystemRandom};
use secrecy::{ExposeSecret, SecretString};
use std::fmt;
use std::sync::Arc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// Access-token validation with refresh, `validate_and_refresh`
mod first_party_code;
mod grant_checks;
pub use grant_checks::TokenGrant;
mod rotate_refresh;
pub use first_party_code::RedeemedFirstPartyCode;
mod validate_refresh;

/// Parameters for authorization code generation
struct AuthCodeParams<'a> {
    client_id: &'a str,
    user_id: Uuid,
    tenant_id: &'a str,
    redirect_uri: &'a str,
    scope: Option<&'a str>,
    state: Option<&'a str>,
    code_challenge: Option<&'a str>,
    code_challenge_method: Option<&'a str>,
    resource: Option<&'a str>,
}

/// What an accepted authorization request is granted: the scope the code
/// carries and the RFC 8707 resource its tokens are bound to, if it named one.
#[derive(Debug, Clone)]
pub struct CheckedAuthorization {
    /// The granted scope, space-delimited
    pub scope: String,
    /// The audience the tokens are bound to; `None` when the request named no
    /// resource, which mints the platform audience
    pub resource: Option<String>,
    /// The name the client registered, shown on the consent screen
    pub client_name: Option<String>,
    /// Whether this request must show consent even for a remembered grant
    pub consent_every_time: bool,
    /// The client a metadata document describes, when it has no row yet:
    /// [`OAuth2AuthorizationServer::enroll_checked_client`] stores it once an
    /// athlete is signed in
    pub unstored_client: Option<OAuth2Client>,
}

/// OAuth 2.0 Authorization Server
pub struct OAuth2AuthorizationServer {
    client_manager: ClientRegistrationManager,
    auth_manager: Arc<AuthManager>,
    jwks_manager: Arc<JwksManager>,
    /// `OAuth2` server repository for auth codes, tokens, clients, and state
    oauth2_server: Arc<dyn OAuth2ServerRepository>,
    /// Tenant repository for user-tenant lookups during authorization
    tenants: Arc<dyn TenantRepository>,
    /// User repository for token validation user lookups
    users: Arc<dyn UserRepository>,
    /// How long each issued refresh token stays exchangeable
    refresh_token_lifetime: Duration,
    /// Identifiers of the MCP resource server this authorization server mints
    /// audience-bound tokens for (RFC 8707): each origin that serves `/mcp`,
    /// the one its protected-resource metadata publishes to a client first
    resources: Vec<String>,
    /// Identifiers of the A2A resource server this authorization server also
    /// mints for: `/a2a` on those origins; empty until
    /// [`Self::with_a2a_resources`] names them
    a2a_resources: Vec<String>,
    /// Identifiers of the satellite MCP servers this authorization server also
    /// mints for, one each; empty until [`Self::with_satellite_resources`]
    satellites: Vec<String>,
    /// Where Dravr's own apps may receive a code; empty until
    /// [`Self::with_first_party_redirects`] names it, which refuses them
    first_party: FirstPartyRedirects,
    /// Reads the client metadata document a URL `client_id` names
    metadata_fetcher: Arc<dyn MetadataFetcher>,
    /// Pending registrations that may exist at once, new metadata-document
    /// clients included
    max_pending_registrations: u64,
    /// Meters the grants each user's tokens redeem; `None` until
    /// [`Self::with_token_limiter`] names it, which meters nobody per user.
    /// Shared with the routes' limiter, so one store counts every window
    token_limiter: Option<Arc<OAuth2RateLimiter>>,
}

impl fmt::Debug for OAuth2AuthorizationServer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuth2AuthorizationServer")
            .field("client_manager", &self.client_manager)
            .field("auth_manager", &self.auth_manager)
            .field("jwks_manager", &self.jwks_manager)
            .field("refresh_token_lifetime", &self.refresh_token_lifetime)
            .field("resources", &self.resources)
            .field("satellites", &self.satellites)
            .field("first_party", &self.first_party)
            .finish_non_exhaustive()
    }
}

impl OAuth2AuthorizationServer {
    /// Creates a new `OAuth2` authorization server instance.
    ///
    /// `refresh_token_expiry_days` is the one `REFRESH_TOKEN_EXPIRY_DAYS`
    /// setting first-party session refresh tokens read as well.
    ///
    /// `resources` are the MCP resource server's identifiers
    /// (`OAuth2ServerConfig::mcp_resources`): the resources a `resource`
    /// parameter may name, and the audiences of the tokens bound to them.
    #[must_use]
    pub fn new(
        oauth2_server: Arc<dyn OAuth2ServerRepository>,
        tenants: Arc<dyn TenantRepository>,
        users: Arc<dyn UserRepository>,
        auth_manager: Arc<AuthManager>,
        jwks_manager: Arc<JwksManager>,
        refresh_token_expiry_days: i64,
        resources: Vec<String>,
    ) -> Self {
        let client_manager = ClientRegistrationManager::new(oauth2_server.clone()); // Safe: Arc clone for manager construction

        Self {
            client_manager,
            auth_manager,
            jwks_manager,
            oauth2_server,
            tenants,
            users,
            refresh_token_lifetime: refresh_token_lifetime(refresh_token_expiry_days),
            resources,
            a2a_resources: Vec::new(),
            satellites: Vec::new(),
            first_party: FirstPartyRedirects::default(),
            metadata_fetcher: cimd::production_fetcher(),
            max_pending_registrations: ClientRetentionConfig::default().max_pending_registrations,
            token_limiter: None,
        }
    }

    /// The authorization server this deployment's configuration describes:
    /// minting for its MCP resource and satellites, and sending Dravr's own
    /// apps their codes only where its first-party redirects allow.
    ///
    /// Every route that issues or redeems a code builds the server here, so
    /// the issuer (`/oauth2/authorize`) and the redeemer (`/oauth/token`)
    /// cannot disagree on a policy one of them was built without.
    #[must_use]
    pub fn from_config(
        config: &OAuth2ServerConfig,
        oauth2_server: Arc<dyn OAuth2ServerRepository>,
        tenants: Arc<dyn TenantRepository>,
        users: Arc<dyn UserRepository>,
        auth_manager: Arc<AuthManager>,
        jwks_manager: Arc<JwksManager>,
        refresh_token_expiry_days: i64,
    ) -> Self {
        Self::new(
            oauth2_server,
            tenants,
            users,
            auth_manager,
            jwks_manager,
            refresh_token_expiry_days,
            config
                .mcp_resources()
                .into_iter()
                .map(str::to_owned)
                .collect(),
        )
        .with_a2a_resources(config.a2a_resources())
        .with_satellite_resources(config.satellite_resources.clone())
        .with_first_party_redirects(config.first_party_redirects.clone())
        .with_max_pending_registrations(config.client_retention.max_pending_registrations)
    }

    /// Cap the registrations no user has authorized yet, which a client
    /// metadata document client counts against like a dynamic one.
    #[must_use]
    pub const fn with_max_pending_registrations(mut self, ceiling: u64) -> Self {
        self.max_pending_registrations = ceiling;
        self
    }

    /// Meter each user's grant redemptions at [`Self::token`] in `limiter`'s
    /// per-user window. The per-address window stays the route's; this one has
    /// to be counted here, once the client authenticated and the grant named a
    /// user, and before the grant is spent.
    #[must_use]
    pub fn with_token_limiter(mut self, limiter: Arc<OAuth2RateLimiter>) -> Self {
        self.token_limiter = Some(limiter);
        self
    }

    /// Read client metadata documents through `fetcher` instead of the SSRF
    /// guarded client; for a test that serves them itself.
    #[must_use]
    pub fn with_metadata_fetcher(mut self, fetcher: Arc<dyn MetadataFetcher>) -> Self {
        self.metadata_fetcher = fetcher;
        self
    }

    /// Also mint for the A2A resource server, known by `a2a_resources`
    /// (`OAuth2ServerConfig::a2a_resources`): a grant bound to it never mints
    /// for the MCP resource server, or the reverse.
    #[must_use]
    pub fn with_a2a_resources(mut self, a2a_resources: Vec<String>) -> Self {
        self.a2a_resources = a2a_resources;
        self
    }

    /// Also mint for the satellite MCP servers `satellites` names
    /// (`OAuth2ServerConfig::satellite_resources`), each its own resource
    /// server: a grant bound to one never mints for the platform or another.
    #[must_use]
    pub fn with_satellite_resources(mut self, satellites: Vec<String>) -> Self {
        self.satellites = satellites;
        self
    }

    /// Serve Dravr's own web and mobile apps, sending their codes only where
    /// `redirects` allows (carnet#787).
    #[must_use]
    pub fn with_first_party_redirects(mut self, redirects: FirstPartyRedirects) -> Self {
        self.first_party = redirects;
        self
    }

    /// Every identifier this server mints for, the platform's then the
    /// satellites', as the resource checks take them.
    fn served(&self) -> Vec<&str> {
        self.resources
            .iter()
            .chain(&self.a2a_resources)
            .chain(&self.satellites)
            .map(String::as_str)
            .collect()
    }

    /// Check an authorization request and resolve the scope and resource it
    /// is granted, before the user is asked anything.
    ///
    /// The client and its `redirect_uri` are checked first because they decide
    /// where every later error goes (RFC 6749 Section 4.1.2.1): an unknown
    /// client or an unregistered `redirect_uri` is
    /// [`AuthorizeRejection::ShownToUser`], and any error after both are
    /// verified (`response_type`, scope, PKCE, resource) is
    /// [`AuthorizeRejection::RedirectedToClient`].
    ///
    /// # Errors
    /// Returns the rejection when the client, `redirect_uri`, `response_type`,
    /// scope, PKCE or resource parameters are refused.
    pub async fn check_authorize_request(
        &self,
        request: &AuthorizeRequest,
    ) -> Result<CheckedAuthorization, AuthorizeRejection> {
        let (client, is_stored) = if cimd::is_metadata_document_id(&request.client_id) {
            // An https URL `client_id` names its own metadata document.
            let resolved = self
                .metadata_resolver()
                .resolve(&request.client_id)
                .await
                .map_err(|e| AuthorizeRejection::ShownToUser(e.into_oauth2_error()))?;
            (resolved.client, resolved.is_stored)
        } else {
            let client = self
                .client_manager
                .get_client(&request.client_id)
                .await
                .map_err(|e| {
                    ClientRegistrationManager::log_lookup_failure(&request.client_id, &e);
                    AuthorizeRejection::ShownToUser(ClientRegistrationManager::lookup_refusal(&e))
                })?;
            (client, true)
        };

        // Exact match against the client's registration (RFC 6749 Section
        // 3.1.2.3), or for Dravr's own apps against the deployment's policy.
        let redirect_accepted = if is_first_party(&client.client_id) {
            self.first_party
                .accepts(&client.client_id, &request.redirect_uri)
        } else {
            redirect_uri_matches(&client.redirect_uris, &request.redirect_uri)
        };
        if !redirect_accepted {
            // Dravr's own apps only ever send this deployment's callbacks, so
            // a refusal here is usually an origin missing from the first-party
            // list, which locks athletes out of sign-in (carnet#1145); it names
            // the origin the `dravr-first-party-authorize-refused` alert is
            // paging about.
            if is_first_party(&client.client_id) {
                warn!(
                    client_id = %client.client_id,
                    redirect_uri = %redaction::redact_url(&request.redirect_uri),
                    "first-party redirect_uri refused"
                );
            }
            return Err(AuthorizeRejection::ShownToUser(
                OAuth2Error::invalid_request("Invalid redirect_uri"),
            ));
        }

        let mut checked = self
            .check_verified_client_request(&client, request)
            .map_err(AuthorizeRejection::RedirectedToClient)?;
        if !is_stored {
            checked.unstored_client = Some(client);
        }
        Ok(checked)
    }

    fn metadata_resolver(&self) -> MetadataResolver {
        MetadataResolver::new(
            self.metadata_fetcher.clone(),
            self.oauth2_server.clone(),
            self.max_pending_registrations,
        )
    }

    /// Store the metadata-document client a checked request found no row for.
    /// The caller has a signed-in athlete; nothing before that writes a row.
    ///
    /// # Errors
    /// Returns the error to send the client when the row cannot be stored.
    pub async fn enroll_checked_client(
        &self,
        checked: &CheckedAuthorization,
    ) -> Result<(), OAuth2Error> {
        let Some(client) = &checked.unstored_client else {
            return Ok(());
        };
        self.metadata_resolver()
            .enroll(client)
            .await
            .map_err(|error| {
                warn!(error = %error, "metadata document client could not be stored");
                error.into_oauth2_error()
            })
    }

    /// The checks that follow a verified client and `redirect_uri`: the
    /// response type, the scope, PKCE and the RFC 8707 resource. Returns what
    /// the request is granted.
    fn check_verified_client_request(
        &self,
        client: &OAuth2Client,
        request: &AuthorizeRequest,
    ) -> Result<CheckedAuthorization, OAuth2Error> {
        refuse_control_characters(&[request.state.as_deref(), request.code_challenge.as_deref()])?;
        refuse_oversized_state(request.state.as_deref())?;

        if request.response_type.is_empty() {
            return Err(OAuth2Error::invalid_request(
                "Missing response_type parameter",
            ));
        }

        // Validate response type is supported by the server
        if request.response_type != "code" {
            return Err(OAuth2Error::invalid_request(
                "Only 'code' response_type is supported",
            ));
        }

        // Validate client is registered for this response_type (RFC 6749 Section 3.1.1)
        if !client.response_types.contains(&request.response_type) {
            return Err(OAuth2Error::unauthorized_client(
                "Client is not registered for the requested response_type",
            ));
        }

        // The grant this authorization issues (RFC 6749 Section 3.3), inside
        // the client's registered scope.
        let scope = Self::authorized_scope(client, request.scope.as_deref())?;

        check_code_challenge(request)?;

        // RFC 8707 §2: a resource this server does not serve is refused with
        // `invalid_target` now, before the user is asked to consent to it.
        let resource = request
            .resource
            .as_deref()
            .map(|requested| bound_audience(&self.served(), requested))
            .transpose()?;
        Ok(CheckedAuthorization {
            scope,
            resource,
            client_name: client.client_name.clone(),
            consent_every_time: redirects_to_any_local_process(client, &request.redirect_uri),
            unstored_client: None,
        })
    }

    /// Handle authorization request (GET /oauth/authorize)
    ///
    /// The request is checked by [`Self::check_authorize_request`]; its
    /// rejection is returned as the error it carries.
    ///
    /// # Errors
    /// Returns an error if client validation fails, invalid parameters, or authorization code generation fails
    pub async fn authorize(
        &self,
        request: AuthorizeRequest,
        user_id: Option<Uuid>,     // From authentication
        tenant_id: Option<String>, // From JWT claims
    ) -> Result<AuthorizeResponse, OAuth2Error> {
        let checked = self
            .check_authorize_request(&request)
            .await
            .map_err(AuthorizeRejection::into_error)?;

        // Consent is enforced at the route layer (`OAuth2Routes::execute_authorization`
        // shows the consent screen and records the grant) before this code-minting
        // method is reached, so `authorize` mints unconditionally for the authenticated
        // user once the request has passed client, redirect_uri, scope, and PKCE checks.
        let user_id =
            user_id.ok_or_else(|| OAuth2Error::invalid_request("User authentication required"))?;
        self.enroll_checked_client(&checked).await?;

        // Generate authorization code with tenant isolation and state binding
        // Resolve tenant_id from JWT claims (active_tenant_id) or database lookup
        let tenant_id = if let Some(tid) = tenant_id {
            tid
        } else {
            // Resolve actual tenant from database - use first tenant user belongs to
            let first = self.tenants.first_for_user(user_id).await.map_err(|e| {
                error!("Failed to get tenants for user {}: {:#}", user_id, e);
                OAuth2Error::server_error("Failed to resolve user tenant")
            })?;
            first.map(|t| t.to_string()).ok_or_else(|| {
                warn!("User {} has no tenant memberships", user_id);
                OAuth2Error::invalid_request("User does not belong to any tenant")
            })?
        };
        let auth_code = self
            .generate_authorization_code(AuthCodeParams {
                client_id: &request.client_id,
                user_id,
                tenant_id: &tenant_id,
                redirect_uri: &request.redirect_uri,
                scope: Some(&checked.scope),
                state: request.state.as_deref(),
                code_challenge: request.code_challenge.as_deref(),
                code_challenge_method: request.code_challenge_method.as_deref(),
                resource: checked.resource.as_deref(),
            })
            .await
            .map_err(|e| {
                // The one refusal in here is a `state` the client already used,
                // logged where it was detected; everything else is a fault.
                if !e.is_server_fault() {
                    return OAuth2Error::invalid_request(
                        "state has already been used; start a new authorization request",
                    );
                }
                error!(
                    "Failed to generate authorization code for client_id={}: {:#}",
                    request.client_id, e
                );
                OAuth2Error::server_error("Failed to generate authorization code")
            })?;

        Ok(AuthorizeResponse {
            code: auth_code,
            state: request.state,
        })
    }

    /// Handle token request (POST /oauth/token)
    ///
    /// # Errors
    /// Returns an error if client validation fails or token generation fails
    pub async fn token(&self, request: TokenRequest) -> Result<TokenResponse, OAuth2Error> {
        // Dravr's own apps are public clients with no secret to present here;
        // they redeem their codes at the first-party `/oauth/token` (carnet#787).
        if is_first_party(&request.client_id) {
            warn!(client_id = %request.client_id, "First-party client refused at the confidential-client token endpoint");
            return Err(OAuth2Error::invalid_client());
        }
        // ALWAYS validate client credentials for ALL grant types (RFC 6749 Section 6)
        // RFC 6749 §6 states: "If the client type is confidential or the client was issued
        // client credentials, the client MUST authenticate with the authorization server"
        // MCP clients are confidential clients, so authentication is REQUIRED
        let client = self
            .client_manager
            .authenticate_client(&request.client_id, request.client_secret.expose_secret())
            .await
            .inspect_err(|e| {
                // A refused client is the caller's mistake. `authenticate_client`
                // already logged each cause at its own level, the server faults
                // among them (a failed lookup, an unreadable stored hash) at
                // ERROR, so this line only adds the grant being attempted.
                warn!(
                    client_id = ?request.client_id,
                    grant_type = %request.grant_type,
                    error = %e.error,
                    "OAuth client validation failed"
                );
            })?;

        // Enforce client's registered grant_types (RFC 6749 Section 2)
        // Clients can only use grant types they were registered for.
        // Per RFC 6749 Section 6, refresh_token is implicitly allowed when the client
        // is registered for authorization_code (since the auth code flow issues refresh tokens).
        let grant_allowed = client.grant_types.contains(&request.grant_type)
            || (request.grant_type == "refresh_token"
                && client
                    .grant_types
                    .iter()
                    .any(|gt| gt == "authorization_code"));
        if !grant_allowed {
            warn!(
                "Client {} attempted grant_type '{}' but is only registered for {:?}",
                request.client_id, request.grant_type, client.grant_types
            );
            return Err(OAuth2Error::unauthorized_client(
                "Client is not registered for the requested grant_type",
            ));
        }

        // A public client proves possession of the authorization code with
        // PKCE alone, so its code exchange without a verifier is refused here
        // whatever the stored code says (RFC 9700 §2.1.1).
        if is_public_client(&client) {
            if request.grant_type == "client_credentials" {
                return Err(OAuth2Error::unauthorized_client(
                    "A public client cannot use the client_credentials grant",
                ));
            }
            if request.grant_type == "authorization_code" && request.code_verifier.is_none() {
                return Err(OAuth2Error::invalid_request(
                    "code_verifier is required for a public client (PKCE S256)",
                ));
            }
        }

        refuse_control_characters(&[
            request.code.as_deref(),
            request.redirect_uri.as_deref(),
            request.code_verifier.as_deref(),
            request
                .refresh_token
                .as_ref()
                .map(ExposeSecret::expose_secret),
            request.scope.as_deref(),
        ])?;

        let response = match request.grant_type.as_str() {
            "authorization_code" => self.handle_authorization_code_grant(request).await,
            "client_credentials" => self.handle_client_credentials_grant(&client, request).await,
            "refresh_token" => self.handle_refresh_token_grant(request).await,
            _ => Err(OAuth2Error::unsupported_grant_type()),
        }?;
        // A refresh token was issued through this client: it is in use.
        if response.refresh_token.is_some() {
            self.client_manager.record_activity(&client).await;
        }
        Ok(response)
    }

    /// Handle authorization code grant
    async fn handle_authorization_code_grant(
        &self,
        request: TokenRequest,
    ) -> Result<TokenResponse, OAuth2Error> {
        let code = request
            .code
            .ok_or_else(|| OAuth2Error::invalid_request("Missing authorization code"))?;

        let redirect_uri = request
            .redirect_uri
            .ok_or_else(|| OAuth2Error::invalid_request("Missing redirect_uri"))?;

        // A resource this server does not serve is refused before the code is
        // spent, so a client that named the wrong one can still correct it.
        self.check_requested_resource(request.resource.as_deref())?;

        self.admit_code_user(&code, &request.client_id, &redirect_uri)
            .await?;

        // Validate and consume authorization code (with PKCE verification)
        let auth_code = self
            .validate_and_consume_auth_code(
                &code,
                &request.client_id,
                &redirect_uri,
                request.code_verifier.as_deref(),
            )
            .await?;

        // The audience: the resource the authorization was bound to, or the
        // one this request narrows an unbound authorization to (RFC 8707 §2.2).
        let audience = self.audience(request.resource.as_deref(), auth_code.resource.as_deref())?;
        self.require_grant_tenant(auth_code.user_id, &auth_code.tenant_id)
            .await?;

        // Generate JWT access token
        let granted = Self::delegated_grant(auth_code.scope.as_deref());
        let access_token = self
            .generate_access_token(
                &request.client_id,
                Some((auth_code.user_id, auth_code.tenant_id.as_str())),
                &granted,
                audience.as_deref(),
            )
            .await
            .map_err(|e| {
                error!(
                    "Failed to generate access token for client_id={}: {:#}",
                    request.client_id, e
                );
                OAuth2Error::server_error("Failed to generate access token")
            })?;

        // Generate refresh token: the first of a new rotation chain
        let refresh_token_value = generate_refresh_token().map_err(|e| {
            error!("Failed to generate secure refresh token: {:#}", e);
            OAuth2Error::server_error("Failed to generate secure refresh token")
        })?;
        let now = Utc::now();

        let refresh_token = super::models::OAuth2RefreshToken {
            token: SecretString::from(refresh_token_value.clone()), // Safe: Clone for storage
            client_id: request.client_id.clone(),                   // Safe: Clone for ownership
            user_id: auth_code.user_id,
            tenant_id: auth_code.tenant_id.clone(), // Safe: Clone for tenant isolation
            scope: Self::rendered_scope(&granted),
            expires_at: now + self.refresh_token_lifetime,
            created_at: now,
            revoked: false,
            family_id: Uuid::new_v4().to_string(),
            // The first token of a chain stamps when the chain began.
            family_created_at: Some(now),
            // The grant keeps the resource it was authorized for; a token
            // request that narrowed an unbound grant bound only that token.
            resource: auth_code.resource,
        };

        // Store refresh token
        self.store_refresh_token(&refresh_token)
            .await
            .map_err(|e| {
                error!(
                    "Failed to store refresh token for client_id={}: {:#}",
                    request.client_id, e
                );
                OAuth2Error::server_error("Failed to store refresh token")
            })?;

        Ok(TokenResponse {
            access_token: SecretString::from(access_token),
            token_type: "Bearer".to_owned(),
            expires_in: 3600, // 1 hour
            scope: Self::rendered_scope(&granted),
            refresh_token: Some(SecretString::from(refresh_token_value)),
        })
    }

    /// Handle client credentials grant
    ///
    /// The requested scope is checked exactly as an authorization request's
    /// is (RFC 6749 §3.3, §5.2): an unknown or undelegable name, or one
    /// outside what the client registered, is `invalid_scope` — never dropped
    /// while the rest is minted.
    async fn handle_client_credentials_grant(
        &self,
        client: &OAuth2Client,
        request: TokenRequest,
    ) -> Result<TokenResponse, OAuth2Error> {
        let audience = self.audience(request.resource.as_deref(), None)?;

        let scope = Self::authorized_scope(client, request.scope.as_deref())?;
        let granted = OAuthScope::parse_granted(&scope);
        let access_token = self
            .generate_access_token(
                &request.client_id,
                None, // No user for client credentials
                &granted,
                audience.as_deref(),
            )
            .await
            .map_err(|e| {
                error!(
                    "Failed to generate client credentials access token for client_id={}: {:#}",
                    request.client_id, e
                );
                OAuth2Error::server_error("Failed to generate access token")
            })?;

        Ok(TokenResponse {
            access_token: SecretString::from(access_token),
            token_type: "Bearer".to_owned(),
            expires_in: 3600, // 1 hour
            scope: Self::rendered_scope(&granted),
            refresh_token: None,
        })
    }

    /// Handle refresh token grant with rotation
    async fn handle_refresh_token_grant(
        &self,
        request: TokenRequest,
    ) -> Result<TokenResponse, OAuth2Error> {
        let refresh_token_value = request
            .refresh_token
            .ok_or_else(|| OAuth2Error::invalid_request("Missing refresh_token"))?;

        // A resource this server does not serve, or not the grant's resource
        // server, is refused before the refresh token is rotated away, and a
        // grant whose tenant the user left is revoked.
        self.check_requested_resource(request.resource.as_deref())?;
        let grant = self
            .check_refresh_grant(
                refresh_token_value.expose_secret(),
                request.resource.as_deref(),
            )
            .await?;
        self.admit_refresh_user(grant.as_ref(), &request.client_id)
            .await?;

        // Consume the presented token and issue its successor; a replayed one
        // revokes its whole chain instead
        let (old_refresh_token, new_refresh_token_value) = self
            .rotate_refresh_token(refresh_token_value.expose_secret(), &request.client_id)
            .await
            .map_err(|e| {
                // Every cause in here (the consume, the family revocation, the
                // successor's RNG and store) is the server failing, not the
                // presented token being refused: that is the `None` below.
                error!(
                    "Failed to rotate refresh token for client_id={}: {:#}",
                    request.client_id, e
                );
                OAuth2Error::server_error("Failed to rotate refresh token")
            })?
            .ok_or_else(|| {
                warn!(
                    "Refresh token validation failed for client_id={}: token not found, already revoked, expired, or mismatched client",
                    request.client_id
                );
                OAuth2Error::invalid_grant("Invalid or expired refresh token")
            })?;

        let audience = self.audience(
            request.resource.as_deref(),
            old_refresh_token.resource.as_deref(),
        )?;

        // Generate new access token
        let granted = Self::delegated_grant(old_refresh_token.scope.as_deref());
        let access_token = self
            .generate_access_token(
                &request.client_id,
                Some((
                    old_refresh_token.user_id,
                    old_refresh_token.tenant_id.as_str(),
                )),
                &granted,
                audience.as_deref(),
            )
            .await
            .map_err(|e| {
                error!(
                    "Failed to generate access token from refresh for client_id={}: {:#}",
                    request.client_id, e
                );
                OAuth2Error::server_error("Failed to generate access token")
            })?;

        info!(
            "Refresh token rotated for client {} and user {}",
            request.client_id, old_refresh_token.user_id
        );

        Ok(TokenResponse {
            access_token: SecretString::from(access_token),
            token_type: "Bearer".to_owned(),
            expires_in: 3600, // 1 hour
            scope: Self::rendered_scope(&granted),
            refresh_token: Some(SecretString::from(new_refresh_token_value)),
        })
    }

    /// Generate authorization code
    async fn generate_authorization_code(&self, params: AuthCodeParams<'_>) -> AppResult<String> {
        let code = Self::generate_random_string(32)?;
        let expires_at = Utc::now() + Duration::minutes(10); // 10 minute expiry

        let auth_code = OAuth2AuthCode {
            code: code.clone(), // Safe: String ownership for OAuth2AuthCode struct
            client_id: params.client_id.to_owned(),
            user_id: params.user_id,
            tenant_id: params.tenant_id.to_owned(),
            redirect_uri: params.redirect_uri.to_owned(),
            scope: params.scope.map(str::to_owned),
            expires_at,
            used: false,
            state: params.state.map(str::to_owned),
            code_challenge: params.code_challenge.map(str::to_owned),
            code_challenge_method: params.code_challenge_method.map(str::to_owned),
            resource: params.resource.map(str::to_owned),
        };

        self.store_auth_code(&auth_code).await?;

        // Server-Side State Validation (Defense-in-Depth CSRF Protection)
        //
        // RFC 6749 § 10.12 BASELINE: State is client-side CSRF protection. Server echoes state unchanged.
        // The state parameter is OPAQUE to the server - clients generate it, store it in their session,
        // and validate it matches on callback. Server's only job is to echo it back.
        //
        // OWASP ENHANCEMENT: We ALSO validate state server-side for defense-in-depth security.
        //
        // Why defense-in-depth?
        // 1. Early CSRF Detection: Detects attacks at the server level before client validation
        // 2. Replay Prevention: 10-minute TTL + single-use flag prevents state reuse
        // 3. Client Binding: State bound to client_id prevents cross-client attacks
        // 4. Tenant Isolation: State bound to tenant_id enforces multi-tenant security
        // 5. Audit Trail: Server-side validation provides security event logging
        //
        // Implementation: oauth2_states table (src/database/mod.rs:232)
        // Consumption: validate_and_consume_auth_code() below (line ~457)
        // Tests: tests/oauth2_state_validation_test.rs (7 security scenarios)
        if let Some(state_value) = params.state {
            let oauth2_state = super::models::OAuth2State {
                state: state_value.to_owned(),
                client_id: params.client_id.to_owned(),
                user_id: Some(params.user_id),
                tenant_id: Some(params.tenant_id.to_owned()),
                redirect_uri: params.redirect_uri.to_owned(),
                scope: params.scope.map(str::to_owned),
                code_challenge: params.code_challenge.map(str::to_owned),
                code_challenge_method: params.code_challenge_method.map(str::to_owned),
                created_at: Utc::now(),
                expires_at,
                used: false,
            };

            self.store_state(&oauth2_state).await?;
        }

        Ok(code)
    }

    /// Store the state an authorization binds its code to.
    ///
    /// `state` is the client's own value and the table's primary key, so a
    /// repeat is the client reusing one (a reloaded authorize URL): refused at
    /// WARN. The insert failing otherwise is the server's, and pages.
    async fn store_state(&self, state: &super::models::OAuth2State) -> AppResult<()> {
        if let Err(e) = self.oauth2_server.store_state(state).await {
            if e.is_server_fault() {
                error!(
                    "Failed to store OAuth2 state for client_id={}: {:#}",
                    state.client_id, e
                );
            } else {
                warn!(client_id = %state.client_id, "OAuth2 authorization refused: state already used");
            }
            return Err(e);
        }
        debug!(
            "Stored OAuth2 state for server-side validation: client_id={}, state_length={}",
            state.client_id,
            state.state.len()
        );
        Ok(())
    }

    /// Validate and consume authorization code
    async fn validate_and_consume_auth_code(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        code_verifier: Option<&str>,
    ) -> Result<OAuth2AuthCode, OAuth2Error> {
        // Atomically consume authorization code (prevents TOCTOU race conditions)
        // This validates client_id, redirect_uri, expiration, and used status in a single atomic operation
        let auth_code = self
            .oauth2_server
            .consume_auth_code(code, client_id, redirect_uri, Utc::now())
            .await
            .map_err(|e| {
                error!(
                    "Failed to atomically consume authorization code for client_id={}: {:#}",
                    client_id,
                    e
                );
                OAuth2Error::server_error("Failed to consume authorization code")
            })?
            .ok_or_else(|| {
                warn!(
                    "Authorization code validation failed for client_id={}: code not found, already used, expired, or mismatched credentials",
                    client_id
                );
                OAuth2Error::invalid_grant("Invalid or expired authorization code")
            })?;

        // Server-Side State Consumption (Atomic CSRF Validation)
        //
        // This is the validation counterpart to state storage above (line ~389).
        //
        // consume_oauth2_state() performs ATOMIC validation with these security checks:
        // 1. State EXISTS in database (prevents fake states)
        // 2. State NOT EXPIRED (10-minute TTL, prevents replay of old states)
        // 3. State NOT USED (single-use flag, prevents replay attacks)
        // 4. client_id MATCHES (prevents cross-client state theft)
        // 5. Marks state as USED atomically (prevents TOCTOU race conditions)
        //
        // Why atomic consumption matters:
        // - Prevents race condition where two concurrent requests could reuse same state
        // - Database transaction ensures state marked used in same operation as retrieval
        // - Implementation: src/database_plugins/sqlite.rs:1796-1849 (with UPDATE ... WHERE used=0)
        //
        // Rejection scenarios (returns None):
        // - State not found in database
        // - State expired (created_at + TTL < now)
        // - State already used (used=true)
        // - client_id mismatch
        //
        // Tests: tests/oauth2_state_validation_test.rs:
        //   - test_state_replay_attack_prevention (line 127)
        //   - test_state_client_id_mismatch (line 288)
        //   - test_expired_state_rejection (line 190)
        if let Some(state_value) = &auth_code.state {
            let consumed_state = self
                .oauth2_server
                .consume_state(state_value, client_id, Utc::now())
                .await
                .map_err(|e| {
                    error!(
                        "Failed to consume OAuth2 state for client_id={}: {:#}",
                        client_id, e
                    );
                    OAuth2Error::server_error("Failed to validate state parameter")
                })?;

            // None indicates validation failure (state not found, expired, used, or client_id mismatch)
            if consumed_state.is_none() {
                warn!(
                    "OAuth2 state validation failed for client_id={}: state not found, already used, expired, or client_id mismatch",
                    client_id
                );
                return Err(OAuth2Error::invalid_grant(
                    "Invalid state parameter - possible CSRF attack detected",
                ));
            }

            debug!(
                "OAuth2 state validation successful for client_id={}, state_length={}",
                client_id,
                state_value.len()
            );
        }

        // Verify PKCE code_verifier (RFC 7636)
        // Note: PKCE verification happens AFTER atomic consumption to prevent code reuse on verification failure
        check_pkce(
            auth_code.code_challenge.as_deref(),
            code_verifier,
            auth_code.code_challenge_method.as_deref(),
            client_id,
        )?;

        Ok(auth_code)
    }

    /// Refuse a token request's `resource` this server does not serve, before
    /// anything is consumed (RFC 8707 §2).
    fn check_requested_resource(&self, requested: Option<&str>) -> Result<(), OAuth2Error> {
        requested.map_or(Ok(()), |requested| {
            bound_audience(&self.served(), requested).map(|_| ())
        })
    }

    /// Resolve the scope an authorization request is granted, or refuse it.
    ///
    /// The requested scope, or the default grant when the client named none —
    /// the same default the consent screen shows, so what the athlete approves
    /// is what the code carries. Every name must be in the vocabulary and
    /// delegable, and the whole grant must sit inside what the client
    /// registered.
    fn authorized_scope(
        client: &OAuth2Client,
        requested: Option<&str>,
    ) -> Result<String, OAuth2Error> {
        let granted = OAuthScope::requested_grant(requested)
            .map_err(|e| OAuth2Error::invalid_scope(&e.message))?;
        let registered = Self::registered_scope(client);
        if let Some(outside) = granted.iter().find(|scope| !registered.contains(scope)) {
            return Err(OAuth2Error::invalid_scope(&format!(
                "Client is not authorized for scope '{outside}'"
            )));
        }
        Ok(OAuthScope::render_granted(&granted))
    }

    /// The scope a client registered.
    ///
    /// Registration persists the default grant when a client asks for none. A
    /// row that holds NULL reads as that same default — the grant the
    /// registration response told the client it had — and never as "no
    /// restriction": an anonymously registered client is not authorized
    /// beyond what it was told.
    fn registered_scope(client: &OAuth2Client) -> Vec<OAuthScope> {
        client
            .scope
            .as_deref()
            .map_or_else(OAuthScope::default_grant, OAuthScope::parse_granted)
    }

    /// The grant a token minted for a stored or requested `scope` carries: the
    /// names this server defines, less any that cannot be delegated.
    ///
    /// Registration, authorization and the client-credentials grant already
    /// refuse `admin`, so this bites only on a stored code or refresh token
    /// that predates those checks. It is also the one place that keeps every
    /// delegated token strictly narrower than the self grant, which is how a
    /// route that reads no scope tells a third party from the athlete.
    fn delegated_grant(scope: Option<&str>) -> Vec<OAuthScope> {
        OAuthScope::parse_granted(scope.unwrap_or_default())
            .into_iter()
            .filter(|scope| scope.is_delegable())
            .collect()
    }

    /// A grant in the token response's `scope` form: absent when empty.
    fn rendered_scope(granted: &[OAuthScope]) -> Option<String> {
        (!granted.is_empty()).then(|| OAuthScope::render_granted(granted))
    }

    /// The grant shown on the consent screen when the client requested none.
    ///
    /// The same [`OAuthScope::default_grant`] the registration endpoint issues,
    /// so what the athlete is shown and what the client receives cannot
    /// disagree — they were two separate literals before, and both still named
    /// a scope this server never checked.
    #[must_use]
    pub fn default_scope_display() -> String {
        OAuthScope::render_granted(&OAuthScope::default_grant())
    }

    /// Generate JWT access token with RS256 asymmetric signing.
    ///
    /// Async because a user-bound token has to carry the athlete's real
    /// connected providers. That field used to be filled with the granted
    /// scopes, which is how an OAuth-minted token came to report `fitness:read`
    /// as a connected provider; the grant now rides in the token's own `scope`
    /// claim and `providers` means what it means everywhere else.
    ///
    /// `audience` is the RFC 8707 resource the token is bound to, `None` for
    /// the platform audience. `subject` is the user the grant acts for and the
    /// tenant it was authorized in, which the caller confirmed the user still
    /// belongs to ([`Self::require_grant_tenant`]); the token carries it as
    /// `active_tenant_id`. A user token is never minted without one.
    async fn generate_access_token(
        &self,
        client_id: &str,
        subject: Option<(Uuid, &str)>,
        granted: &[OAuthScope],
        audience: Option<&str>,
    ) -> AppResult<String> {
        if granted.is_empty() {
            debug!(
                client_id = %client_id,
                user_id = ?subject.map(|(user_id, _)| user_id),
                "Minting an access token with an empty grant"
            );
        }
        let scopes: Vec<String> = granted
            .iter()
            .map(|scope| scope.as_str().to_owned())
            .collect();

        let Some((uid, tenant_id)) = subject else {
            return self
                .auth_manager
                .generate_client_credentials_token(
                    &self.jwks_manager,
                    client_id,
                    &scopes,
                    None, // tenant_id for client credentials
                    audience,
                )
                .map_err(|e| {
                    AppError::internal(format!("Failed to generate client credentials token: {e}"))
                });
        };

        // SECURITY: Global lookup — OAuth2 token minting, no tenant context.
        // A user who cannot be read has no providers to report; the token is
        // still minted, because the grant is what authorizes it and the
        // provider list is descriptive.
        let providers = self
            .users
            .get_global(uid)
            .await
            .ok()
            .flatten()
            .map(|user| user.available_providers())
            .unwrap_or_default();
        self.auth_manager
            .generate_oauth_access_token(
                &self.jwks_manager,
                &DelegatedGrant {
                    client_id,
                    scopes: &scopes,
                    audience,
                },
                &uid,
                &providers,
                Some(tenant_id.to_owned()),
            )
            .map_err(|e| AppError::internal(format!("Failed to generate OAuth access token: {e}")))
    }

    /// Generate random string for codes
    ///
    /// # Errors
    /// Returns an error if system RNG fails - this is a critical security failure
    /// and the server cannot operate securely without working RNG
    fn generate_random_string(length: usize) -> AppResult<String> {
        let rng = SystemRandom::new();
        let mut bytes = vec![0u8; length];

        rng.fill(&mut bytes).map_err(|e| {
            error!(
                "CRITICAL: SystemRandom failed - cannot generate secure random bytes: {}",
                e
            );
            AppError::internal("System RNG failure - server cannot operate securely")
        })?;

        // Convert to URL-safe base64
        Ok(general_purpose::URL_SAFE_NO_PAD.encode(&bytes))
    }

    /// Store authorization code (database operation)
    async fn store_auth_code(&self, auth_code: &OAuth2AuthCode) -> AppResult<()> {
        self.oauth2_server.store_auth_code(auth_code).await
    }

    /// Store refresh token (database operation)
    async fn store_refresh_token(
        &self,
        refresh_token: &super::models::OAuth2RefreshToken,
    ) -> AppResult<()> {
        self.oauth2_server.store_refresh_token(refresh_token).await
    }
}
