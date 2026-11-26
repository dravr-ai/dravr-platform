// ABOUTME: OAuth 2.0 authorization and token endpoints implementation
// ABOUTME: Handles OAuth 2.0 flow with JWT tokens as access tokens for MCP client compatibility
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// NOTE: All `.clone()` calls in this file are Safe - they are necessary for:
// - String ownership transfers to struct constructors (OAuth2AuthCode, TokenResponse)
// - Arc clone for database manager creation

use super::authorization_details::ResolvedDetails;
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
use super::step_up_scope;
use crate::admin::jwks::JwksManager;
use crate::auth::{oauth_access_token_lifetime_secs, AuthManager, TokenLineage};
use crate::config::oauth::ClientRetentionConfig;
use crate::config::OAuth2ServerConfig;
use crate::refresh_rotation::{
    admin_family_first_expiry, generate_refresh_token, refresh_token_lifetime,
};
use base64::{engine::general_purpose, Engine as _};
use dravr_tronc::dates::{Elapsed, Timestamp};
use grant_checks::{require_refresh_dpop_binding, token_type};
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

/// Minting the access token a grant is redeemed for, under its refresh family
mod access_token;
mod auth_code;
use auth_code::{check_dpop_jkt, AuthCodeParams};
/// Admin access at consent: the one way a grant holds `admin` (carnet#1511)
mod admin_grant;
pub use admin_grant::{without_admin, AdminAccess};
/// The request's RFC 9396 details, and the details a grant's tokens echo
mod details;
/// Access-token validation with refresh, `validate_and_refresh`
mod first_party_code;
mod grant_checks;
pub use grant_checks::TokenGrant;
mod rotate_refresh;
pub use first_party_code::RedeemedFirstPartyCode;
mod validate_refresh;
pub use validate_refresh::ADMIN_REFRESHES_AT_TOKEN_ENDPOINT;

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
    /// The Host Profile the client's metadata document declared when it was
    /// read for this request (owl §6.2 carrier A); `None` when it declares
    /// none, was not re-read, or the client has no document
    pub client_host_profile: Option<String>,
    /// Whether an admin's Admin access may be granted on this request: it is
    /// bound to the platform's own `/mcp` or `/a2a` resource. No resource, or
    /// a satellite's, is never granted `admin`
    pub admin_grantable: bool,
    /// The request's RFC 9396 details, resolved: the preset grant it asks
    /// for; `None` when it asks for none
    pub details: Option<ResolvedDetails>,
    /// The resource a preset grant on this request reads from, which the
    /// consent screen's picker grants for; `None` when no preset may be
    /// granted (a request bound to a satellite)
    pub preset_source: Option<String>,
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
    refresh_token_lifetime: Elapsed,
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
    /// Whether the step-up screen is offered, so a step-up preset's request
    /// (`one_off_question`) is resolved rather than refused "step-up is not
    /// offered yet"; off until [`Self::with_step_up_offered`] says the
    /// catalogue words the screen (carnet#1544 slice 4b, dark launch)
    step_up_offered: bool,
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
            step_up_offered: false,
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

    /// Offer the step-up screen: a `one_off_question` request is resolved,
    /// and granted only as a child of a live grant after a fresh sign-in.
    #[must_use]
    pub const fn with_step_up_offered(mut self, offered: bool) -> Self {
        self.step_up_offered = offered;
        self
    }

    /// Whether the step-up screen is offered here.
    #[must_use]
    pub const fn step_up_offered(&self) -> bool {
        self.step_up_offered
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
        let mut host_profile = None;
        let (client, is_stored) = if cimd::is_metadata_document_id(&request.client_id) {
            // An https URL `client_id` names its own metadata document.
            let resolved = self
                .metadata_resolver()
                .resolve(&request.client_id)
                .await
                .map_err(|e| AuthorizeRejection::ShownToUser(e.into_oauth2_error()))?;
            host_profile = resolved.host_profile;
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
        checked.client_host_profile = host_profile;
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
            .enroll(client, checked.client_host_profile.as_deref())
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
        // the client's registered scope. `admin` is offered on the consent
        // screen, never asked for, so a request naming it is checked without
        // it rather than refused (carnet#1511, Q20).
        // A step-up scope (`pdpp:`, owl §7.6) asks for a one-off question
        // over its streams; the rest of it is the grant the code carries.
        let step_up = step_up_scope::split(request.scope.as_deref())?;
        let requested = without_admin(
            step_up
                .as_ref()
                .map_or(request.scope.as_deref(), |step_up| Some(&step_up.scope)),
        );
        let scope = Self::authorized_scope(client, requested.as_deref())?;

        check_code_challenge(request)?;
        check_dpop_jkt(request.dpop_jkt.as_deref())?;

        // RFC 8707 §2: a resource this server does not serve is refused with
        // `invalid_target` now, before the user is asked to consent to it.
        let resource = request
            .resource
            .as_deref()
            .map(|requested| bound_audience(&self.served(), requested))
            .transpose()?;
        // RFC 9396: details the server cannot resolve are refused now too.
        let asked = self.step_up_details(step_up.as_ref(), request, resource.as_deref())?;
        let details = self.resolve_request_details(
            asked
                .as_deref()
                .or(request.authorization_details.as_deref()),
            resource.as_deref(),
        )?;
        let preset_source = self
            .preset_sources(resource.as_deref())
            .first()
            .map(|s| (*s).to_owned());
        // A preset grant never carries `admin`.
        let admin_grantable = details.is_none() && self.is_platform_resource(resource.as_deref());
        Ok(CheckedAuthorization {
            scope,
            resource,
            client_name: client.client_name.clone(),
            consent_every_time: redirects_to_any_local_process(client, &request.redirect_uri),
            unstored_client: None,
            client_host_profile: None,
            admin_grantable,
            details,
            preset_source,
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
        self.authorize_with_admin_access(request, user_id, tenant_id, AdminAccess::Withheld, None)
            .await
    }

    /// [`Self::authorize`] for a request the user decided `access` on at the
    /// consent screen: the code carries [`Self::granted_scope`], and names
    /// `client_grant_id`, the consent grant it is issued under, which every
    /// token minted from it carries.
    ///
    /// # Errors
    /// As [`Self::authorize`].
    pub async fn authorize_with_admin_access(
        &self,
        request: AuthorizeRequest,
        user_id: Option<Uuid>,
        tenant_id: Option<String>,
        access: AdminAccess,
        client_grant_id: Option<String>,
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
        let scope = Self::granted_scope(&checked, access);
        let auth_code = self
            .generate_authorization_code(AuthCodeParams {
                client_id: &request.client_id,
                user_id,
                tenant_id: &tenant_id,
                redirect_uri: &request.redirect_uri,
                scope: Some(&scope),
                state: request.state.as_deref(),
                code_challenge: request.code_challenge.as_deref(),
                code_challenge_method: request.code_challenge_method.as_deref(),
                resource: checked.resource.as_deref(),
                client_grant_id: client_grant_id.as_deref(),
                dpop_jkt: request.dpop_jkt.as_deref(),
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

    /// Handle token request (POST /oauth/token) that carries no `DPoP` proof.
    ///
    /// # Errors
    /// Returns an error if client validation fails or token generation fails
    pub async fn token(&self, request: TokenRequest) -> Result<TokenResponse, OAuth2Error> {
        self.token_with_dpop(request, None).await
    }

    /// Handle a token request whose `DPoP` proof (RFC 9449 §5), when it
    /// carried one, the route already verified and burned: `dpop_jkt` is that
    /// proof's key thumbprint.
    ///
    /// - A code bound to a key (`dpop_jkt` at PAR or authorize) is redeemed
    ///   only with a proof by that key, and is left unspent otherwise.
    /// - A refresh token bound to a key is rotated only with a proof by it.
    /// - A proof binds what is minted: the access token carries `cnf.jkt` and
    ///   is answered as `token_type: DPoP`, and a code grant's refresh family
    ///   is bound to the same key. Without one the token is Bearer, as before.
    /// - The client-credentials grant ignores a proof: its token acts for no
    ///   user and stays Bearer.
    ///
    /// # Errors
    /// As [`Self::token`], plus `invalid_dpop_proof` for a missing or
    /// mismatched proof on a bound grant.
    pub async fn token_with_dpop(
        &self,
        request: TokenRequest,
        dpop_jkt: Option<&str>,
    ) -> Result<TokenResponse, OAuth2Error> {
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
            "authorization_code" => {
                self.handle_authorization_code_grant(request, dpop_jkt)
                    .await
            }
            "client_credentials" => self.handle_client_credentials_grant(&client, &request),
            "refresh_token" => self.handle_refresh_token_grant(request, dpop_jkt).await,
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
        dpop_jkt: Option<&str>,
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

        // A code bound to a key is refused without a proof by it before it is
        // spent, so the client holding the key can still redeem it.
        self.require_code_dpop_binding(&code, &request.client_id, &redirect_uri, dpop_jkt)
            .await?;
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
        let authorization_details = self
            .require_live_client_grant(
                auth_code.user_id,
                &auth_code.tenant_id,
                &request.client_id,
                auth_code.client_grant_id.as_deref(),
            )
            .await?;

        // Generate JWT access token; `admin` only while the user is an admin
        let granted = self
            .delegated_grant_for(
                auth_code.user_id,
                auth_code.scope.as_deref(),
                auth_code.resource.as_deref(),
            )
            .await;
        // The grant starts a refresh family, which its every access token
        // names (carnet#1511).
        let family_id = Uuid::new_v4().to_string();
        let access_token = self
            .generate_access_token(
                &request.client_id,
                Some((auth_code.user_id, auth_code.tenant_id.as_str())),
                Some(TokenLineage {
                    family_id: &family_id,
                    client_grant_id: auth_code.client_grant_id.as_deref(),
                    dpop_jkt,
                }),
                &granted,
                audience.as_deref(),
            )
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
        let now = Timestamp::now();
        // A family holding `admin` lives a day from this first token (Q20).
        let expires_at = if granted.contains(&OAuthScope::Admin) {
            admin_family_first_expiry(now, self.refresh_token_lifetime)
        } else {
            now + self.refresh_token_lifetime
        };

        let refresh_token = super::models::OAuth2RefreshToken {
            token: SecretString::from(refresh_token_value.clone()), // Safe: Clone for storage
            client_id: request.client_id.clone(),                   // Safe: Clone for ownership
            user_id: auth_code.user_id,
            tenant_id: auth_code.tenant_id.clone(), // Safe: Clone for tenant isolation
            scope: Self::rendered_scope(&granted),
            expires_at,
            created_at: now,
            revoked: false,
            family_id,
            // The first token of a chain stamps when the chain began.
            family_created_at: Some(now),
            // The grant keeps the resource it was authorized for; a token
            // request that narrowed an unbound grant bound only that token.
            resource: auth_code.resource,
            client_grant_id: auth_code.client_grant_id,
            // The family is bound to the key the code was redeemed with
            // (RFC 9449 §5): every refresh needs a proof by it.
            dpop_jkt: dpop_jkt.map(str::to_owned),
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
            token_type: token_type(dpop_jkt).to_owned(),
            expires_in: oauth_access_token_lifetime_secs(&request.client_id),
            scope: Self::rendered_scope(&granted),
            refresh_token: Some(SecretString::from(refresh_token_value)),
            authorization_details,
        })
    }

    /// Handle client credentials grant
    ///
    /// The requested scope is checked exactly as an authorization request's
    /// is (RFC 6749 §3.3, §5.2): an unknown or undelegable name, or one
    /// outside what the client registered, is `invalid_scope` — never dropped
    /// while the rest is minted.
    fn handle_client_credentials_grant(
        &self,
        client: &OAuth2Client,
        request: &TokenRequest,
    ) -> Result<TokenResponse, OAuth2Error> {
        let audience = self.audience(request.resource.as_deref(), None)?;

        let scope = Self::authorized_scope(client, request.scope.as_deref())?;
        let granted = OAuthScope::parse_granted(&scope);
        let access_token = self
            .generate_access_token(
                &request.client_id,
                None, // No user for client credentials
                None,
                &granted,
                audience.as_deref(),
            )
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
            authorization_details: None,
        })
    }

    /// Handle refresh token grant with rotation
    async fn handle_refresh_token_grant(
        &self,
        request: TokenRequest,
        dpop_jkt: Option<&str>,
    ) -> Result<TokenResponse, OAuth2Error> {
        let refresh_token_value = request
            .refresh_token
            .ok_or_else(|| OAuth2Error::invalid_request("Missing refresh_token"))?;

        // A resource this server does not serve, or not the grant's resource
        // server, is refused before the refresh token is rotated away, and a
        // grant whose tenant the user left is revoked.
        self.check_requested_resource(request.resource.as_deref())?;
        let (grant, authorization_details) = self
            .check_refresh_grant(
                refresh_token_value.expose_secret(),
                request.resource.as_deref(),
            )
            .await?;
        // A family bound to a key rotates only with a proof by it (RFC 9449
        // §5), checked before the token is rotated away.
        require_refresh_dpop_binding(grant.as_ref(), dpop_jkt)?;
        self.admit_refresh_user(grant.as_ref(), &request.client_id)
            .await?;

        // Consume the presented token and issue its successor; a replayed one
        // revokes its whole chain instead
        let (old_refresh_token, new_refresh_token_value, granted) = self
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

        // Generate new access token, under the grant the rotation decided
        let access_token = self
            .generate_access_token(
                &request.client_id,
                Some((
                    old_refresh_token.user_id,
                    old_refresh_token.tenant_id.as_str(),
                )),
                Some(TokenLineage {
                    family_id: &old_refresh_token.family_id,
                    client_grant_id: old_refresh_token.client_grant_id.as_deref(),
                    dpop_jkt,
                }),
                &granted,
                audience.as_deref(),
            )
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
            token_type: token_type(dpop_jkt).to_owned(),
            expires_in: oauth_access_token_lifetime_secs(&request.client_id),
            scope: Self::rendered_scope(&granted),
            refresh_token: Some(SecretString::from(new_refresh_token_value)),
            authorization_details,
        })
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
            .consume_auth_code(code, client_id, redirect_uri, Timestamp::now())
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
                .consume_state(state_value, client_id, Timestamp::now())
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
