// ABOUTME: OAuth 2.0 dynamic client registration implementation (RFC 7591)
// ABOUTME: Handles client registration endpoint for MCP clients and other OAuth clients
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::cimd::is_metadata_document_id;
use super::models::{
    ClientRegistrationRequest, ClientRegistrationResponse, OAuth2Client, OAuth2Error,
};
use crate::config::oauth::ClientRetentionConfig;
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Duration, Utc};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::OAuth2ClientSweep;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::redaction::redact_url;
use pierre_database::backends::OAuth2ServerRepository;
use ring::rand::{SecureRandom, SystemRandom};
use secrecy::SecretString;
use std::env;
use std::fmt;
use std::sync::Arc;
use tracing::{error, info, warn};
use url::Url;
use uuid::Uuid;

/// The `client_secret_hash` of a public client (`token_endpoint_auth_method`
/// `none`): no secret exists, and the empty string is no valid Argon2 hash, so
/// no presented secret can ever authenticate the row.
pub(super) const PUBLIC_CLIENT_SECRET_HASH: &str = "";

/// Days a registration stays valid without being used.
///
/// Each refresh token issued through it slides the expiry forward, so a client its users keep
/// connecting through does not expire, while one nobody uses does.
pub(super) const INACTIVITY_EXPIRY_DAYS: i64 = 90;

/// The `token_endpoint_auth_method` a registration that names none gets
/// (RFC 7591 §2).
const DEFAULT_AUTH_METHOD: &str = "client_secret_basic";

/// Whether `client` is a public client: it holds no secret, authenticates with
/// nothing, and is held to PKCE instead.
#[must_use]
pub(super) fn is_public_client(client: &OAuth2Client) -> bool {
    client.client_secret_hash == PUBLIC_CLIENT_SECRET_HASH
}

/// Whether a request's `redirect_uri` is one the client registered.
///
/// Exact match (RFC 6749 §3.1.2.3), except for a registered loopback
/// redirect (`http` on `localhost`, `127.0.0.1` or `[::1]`), whose port the
/// request may change (RFC 8252 §7.3): a native app binds a free port at
/// run time. Scheme, host, path and query must still be the registered ones.
#[must_use]
pub(super) fn redirect_uri_matches(registered: &[String], requested: &str) -> bool {
    if registered.iter().any(|uri| uri == requested) {
        return true;
    }
    let Ok(requested) = Url::parse(requested) else {
        return false;
    };
    if requested.scheme() != "http"
        || !requested.username().is_empty()
        || requested.password().is_some()
        || requested.fragment().is_some()
    {
        return false;
    }
    registered.iter().any(|uri| {
        let Ok(registered) = Url::parse(uri) else {
            return false;
        };
        registered.scheme() == "http"
            && is_loopback_host(&registered)
            && registered.host_str() == requested.host_str()
            && registered.path() == requested.path()
            && registered.query() == requested.query()
    })
}

/// Whether a code for this request could reach any process on the athlete's machine.
///
/// A public client has no secret to prove who redeems the code, and a loopback
/// redirect's port is the request's to pick. Any local program can name
/// such a client, so its identity is not assured and a remembered grant must
/// not mint a code unseen (RFC 8252 §8.6).
#[must_use]
pub(super) fn redirects_to_any_local_process(client: &OAuth2Client, redirect_uri: &str) -> bool {
    is_public_client(client) && redirects_to_loopback(redirect_uri)
}

/// The host to show beside a client's self-chosen name: its metadata
/// document's, which only its owner can serve from, else where it receives its
/// authorization code.
#[must_use]
pub fn client_display_host(client_id: &str, redirect_uris: &[String]) -> Option<String> {
    let host_of = |uri: &str| Url::parse(uri).ok()?.host_str().map(str::to_owned);
    is_metadata_document_id(client_id)
        .then(|| host_of(client_id))
        .flatten()
        .or_else(|| redirect_uris.iter().find_map(|uri| host_of(uri)))
}

/// Whether `redirect_uri` returns to the machine the request came from.
#[must_use]
pub fn redirects_to_loopback(redirect_uri: &str) -> bool {
    Url::parse(redirect_uri).is_ok_and(|url| is_loopback_host(&url))
}

/// Whether a URL's host is a loopback name or address.
fn is_loopback_host(url: &Url) -> bool {
    matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"))
}

/// OAuth 2.0 Client Registration Manager
pub struct ClientRegistrationManager {
    oauth2: Arc<dyn OAuth2ServerRepository>,
}

impl fmt::Debug for ClientRegistrationManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ClientRegistrationManager")
            .finish_non_exhaustive()
    }
}

impl ClientRegistrationManager {
    /// Creates a new client registration manager
    #[must_use]
    pub fn new(oauth2: Arc<dyn OAuth2ServerRepository>) -> Self {
        Self { oauth2 }
    }

    /// Register a new OAuth 2.0 client (RFC 7591)
    ///
    /// `max_pending_registrations` caps the registrations no user has
    /// authorized yet; at the cap this one is refused with `too_many_requests`
    /// (see [`OAuth2Error::http_status`]) and nothing is stored.
    ///
    /// # Errors
    /// Returns an error if client registration validation fails, the pending
    /// ceiling is reached, or database storage fails
    pub async fn register_client(
        &self,
        request: ClientRegistrationRequest,
        max_pending_registrations: u64,
    ) -> Result<ClientRegistrationResponse, OAuth2Error> {
        self.register_client_with_auth_method(request, None, max_pending_registrations)
            .await
    }

    /// Register a client that asked for the token endpoint authentication
    /// method `auth_method` (`none`, `client_secret_basic` or
    /// `client_secret_post`; `client_secret_basic` when `None`, RFC 7591 §2).
    /// A `none` client is public: it is issued no secret and is held to PKCE.
    ///
    /// # Errors
    /// As [`Self::register_client`], and `invalid_client_metadata` for a
    /// method this server does not support
    pub async fn register_client_with_auth_method(
        &self,
        request: ClientRegistrationRequest,
        auth_method: Option<String>,
        max_pending_registrations: u64,
    ) -> Result<ClientRegistrationResponse, OAuth2Error> {
        // Validate request
        Self::validate_registration_request(&request, auth_method.as_deref())?;

        // The grant this client may ever be issued, persisted exactly as the
        // response advertises it (RFC 7591 §3.2.1): the scope it asked for,
        // every name checked against the vocabulary and `admin` refused, or
        // the read-only default when it asked for none. The authorization
        // endpoint checks every request against this row, so what is stored
        // here is the ceiling of every grant the client can obtain.
        // A client that names no scope gets every scope a third party can be
        // delegated as its ceiling: what each authorization grants is still
        // what it asks for (defaulting to the read-only grant), and the
        // consent screen shows it, so a client asking for write scopes is not
        // refused `invalid_scope` at authorize.
        let named_scope = request
            .scope
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let scope = if named_scope.is_some() {
            OAuthScope::requested_grant(named_scope)
                .map(|granted| OAuthScope::render_granted(&granted))
                .map_err(|e| OAuth2Error::invalid_client_metadata(&e.message))?
        } else {
            OAuthScope::render_granted(&OAuthScope::delegable_grant())
        };

        // Generate client credentials: a public client (`none`) holds no
        // secret, and is held to PKCE instead.
        let auth_method = auth_method.unwrap_or_else(|| DEFAULT_AUTH_METHOD.to_owned());
        let public = auth_method == "none";
        let client_id = Self::generate_client_id();
        let (client_secret, client_secret_hash) = if public {
            (String::new(), PUBLIC_CLIENT_SECRET_HASH.to_owned())
        } else {
            let secret = Self::generate_client_secret()?;
            let hash = Self::hash_client_secret(&secret)?;
            (secret, hash)
        };

        // Set default values - only authorization_code by default for security (RFC 8252 best practices)
        // Clients must explicitly request client_credentials if needed
        let grant_types = request
            .grant_types
            .unwrap_or_else(|| vec!["authorization_code".to_owned()]);

        let response_types = request
            .response_types
            .unwrap_or_else(|| vec!["code".to_owned()]);

        let created_at = Utc::now();
        // Every registration carries an expiry: it is what marks a row as written
        // here, and the retention sweep and the pending ceiling touch no other.
        // `check_client_expiry` refuses the client from this instant on; the
        // sweep deletes the row once the configured grace has passed as well.
        // Expiry is on inactivity: it slides forward whenever a user
        // authorizes through the client (a refresh token is issued).
        let expires_at = Some(created_at + Duration::days(INACTIVITY_EXPIRY_DAYS));

        // Create client record
        let client = OAuth2Client {
            id: Uuid::new_v4().to_string(),
            client_id: client_id.clone(), // Safe: String ownership for OAuth client struct
            client_secret_hash,
            redirect_uris: request.redirect_uris.clone(), // Safe: Vec ownership for OAuth client struct
            grant_types: grant_types.clone(),             // Safe: Vec ownership for OAuth client
            response_types: response_types.clone(),       // Safe: Vec ownership for OAuth client
            client_name: request.client_name.clone(),     // Safe: String ownership for OAuth client
            client_uri: request.client_uri.clone(), // Safe: Option<String> ownership for OAuth client
            scope: Some(scope.clone()), // Safe: String ownership, echoed in the response
            created_at,
            expires_at,
        };

        // Store in database, unless the pending ceiling is already reached
        let stored = self
            .oauth2
            .store_client_within_ceiling(&client, max_pending_registrations)
            .await
            .map_err(|e| {
                error!(error = %e, client_id = %client_id, "Failed to store OAuth2 client registration in database");
                OAuth2Error::server_error("Failed to store client registration")
            })?;
        if !stored {
            warn!(
                ceiling = max_pending_registrations,
                "OAuth2 client registration refused: the pending-registration ceiling is reached"
            );
            return Err(OAuth2Error::too_many_requests(
                "Too many registered clients are awaiting authorization; retry later",
            ));
        }

        // Return registration response
        // Build default client_uri from actual server configuration (if initialized)
        // Falls back to localhost:8081 for test environments
        let default_client_uri = Self::get_default_client_uri();

        Ok(ClientRegistrationResponse {
            client_id,
            client_secret: SecretString::from(client_secret),
            client_id_issued_at: Some(created_at.timestamp()),
            client_secret_expires_at: if public {
                None
            } else {
                expires_at.map(|dt| dt.timestamp())
            },
            auth_method,
            redirect_uris: request.redirect_uris,
            grant_types,
            response_types,
            client_name: request.client_name,
            // RFC 7591: client_uri is OPTIONAL but Claude Code requires it to be non-null
            // Provide actual server URL when not specified by the client
            client_uri: request.client_uri.or(Some(default_client_uri)),
            // What was persisted above. RFC 7591 §3.1.1: a client that
            // requests no scope gets the server's default — read-only, and
            // read-only deliberately: a client that never asked for anything
            // has not been consented to writing.
            scope: Some(scope),
        })
    }

    /// Provision a `client_credentials`-only registration for a client the
    /// server registered through its own surface (an A2A client), under the
    /// `client_id` that surface already issued, and return its secret.
    ///
    /// The secret is generated and Argon2-hashed exactly as a dynamic
    /// registration's is, so `/oauth2/token` verifies it through
    /// [`Self::authenticate_client`] like any other. The registration may use no
    /// other grant and has no redirect URI, and its scope ceiling is the
    /// read-only default a client that asks for nothing is given. It carries
    /// no `expires_at`: it lives as long as the client that owns it, which
    /// deletes it through [`Self::delete_client`], so neither the pending
    /// ceiling nor the retention sweep ever counts or deletes it.
    ///
    /// # Errors
    /// Returns an error if the name holds a control character, the system RNG
    /// or Argon2 fails, or the registration cannot be stored (a `client_id`
    /// already registered included)
    pub async fn register_client_credentials_client(
        &self,
        client_id: &str,
        client_name: &str,
    ) -> Result<String, OAuth2Error> {
        if client_id.chars().any(char::is_control) || client_name.chars().any(char::is_control) {
            return Err(OAuth2Error::invalid_client_metadata(
                "client_id and client_name must not contain control characters",
            ));
        }
        let scope = OAuthScope::render_granted(&OAuthScope::default_grant());

        let client_secret = Self::generate_client_secret()?;
        let client = OAuth2Client {
            id: Uuid::new_v4().to_string(),
            client_id: client_id.to_owned(),
            client_secret_hash: Self::hash_client_secret(&client_secret)?,
            redirect_uris: Vec::new(),
            grant_types: vec!["client_credentials".to_owned()],
            response_types: Vec::new(),
            client_name: Some(client_name.to_owned()),
            client_uri: None,
            scope: Some(scope),
            created_at: Utc::now(),
            expires_at: None,
        };

        self.oauth2.store_client(&client).await.map_err(|e| {
            error!(error = %e, client_id = %client_id, "Failed to store client_credentials registration");
            OAuth2Error::server_error("Failed to store client registration")
        })?;
        info!(client_id = %client_id, "client_credentials registration provisioned");
        Ok(client_secret)
    }

    /// Delete the registration `client_id` names, so `/oauth2/token` refuses
    /// it from then on as an unknown client. Returns `true` when one was
    /// deleted.
    ///
    /// # Errors
    /// Returns the repository's error when the delete fails
    pub async fn delete_client(&self, client_id: &str) -> AppResult<bool> {
        self.oauth2.delete_client(client_id).await
    }

    /// Verify client secret using Argon2 password hash
    fn verify_client_secret(
        client_id: &str,
        client_secret: &str,
        client_secret_hash: &str,
    ) -> Result<(), OAuth2Error> {
        let parsed_hash = PasswordHash::new(client_secret_hash).map_err(|e| {
            error!("Failed to parse stored password hash: {}", e);
            OAuth2Error::server_error("Client authentication could not complete")
        })?;

        let argon2 = Argon2::default();
        if argon2
            .verify_password(client_secret.as_bytes(), &parsed_hash)
            .is_err()
        {
            warn!("OAuth client {} secret validation failed", client_id);
            return Err(OAuth2Error::invalid_client());
        }

        Ok(())
    }

    /// Check if client is expired
    fn check_client_expiry(
        client_id: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> Result<(), OAuth2Error> {
        if let Some(expires_at) = expires_at {
            if Utc::now() > expires_at {
                warn!("OAuth client {} has expired", client_id);
                return Err(OAuth2Error::invalid_client());
            }
        }
        Ok(())
    }

    /// Authenticate the client of a token request.
    ///
    /// A confidential client proves its secret. A public client has none and
    /// must present none: a secret sent for it is refused as it is for an
    /// unknown client, so a public client cannot be made to look
    /// authenticated. Expiry is checked either way.
    ///
    /// # Errors
    /// Returns an error if the client is unknown or expired, or its
    /// credentials do not match what it registered with
    pub async fn authenticate_client(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> Result<OAuth2Client, OAuth2Error> {
        let client = self.get_client(client_id).await.map_err(|e| {
            Self::log_lookup_failure(client_id, &e);
            Self::lookup_refusal(&e)
        })?;
        if is_public_client(&client) {
            if !client_secret.is_empty() {
                warn!("OAuth public client {} presented a secret", client_id);
                return Err(OAuth2Error::invalid_client());
            }
            Self::check_client_expiry(client_id, client.expires_at)?;
            return Ok(client);
        }
        if client_secret.is_empty() {
            warn!("OAuth client {} presented no secret", client_id);
            return Err(OAuth2Error::invalid_client());
        }
        Self::verify_client_secret(client_id, client_secret, &client.client_secret_hash)?;
        Self::check_client_expiry(client_id, client.expires_at)?;
        Ok(client)
    }

    /// Slide the expiry of a registration forward after a user authorized
    /// through it, so an active client does not expire. A failure is logged
    /// and not returned: the grant it follows has already been issued.
    pub(super) async fn record_activity(&self, client: &OAuth2Client) {
        if client.expires_at.is_none() {
            return;
        }
        let expires_at = Utc::now() + Duration::days(INACTIVITY_EXPIRY_DAYS);
        if let Err(e) = self
            .oauth2
            .extend_client_expiry(&client.client_id, expires_at)
            .await
        {
            warn!(client_id = %client.client_id, error = %e, "Could not slide the OAuth client expiry");
        }
    }

    /// Get client by `client_id`
    ///
    /// A `client_id` carrying a control character names no client this server
    /// issued, and is refused as unknown without a query: `PostgreSQL` rejects
    /// a NUL byte in a text parameter, which would otherwise report the
    /// caller's malformed id as a database failure.
    ///
    /// # Errors
    /// Returns `ErrorCode::ResourceNotFound` when no such client exists, and the
    /// repository's error when the lookup itself fails
    pub async fn get_client(&self, client_id: &str) -> AppResult<OAuth2Client> {
        if client_id.chars().any(char::is_control) {
            return Err(AppError::not_found("OAuth2 client"));
        }
        self.oauth2
            .get_client(client_id)
            .await?
            .ok_or_else(|| AppError::not_found("OAuth2 client"))
    }

    /// The `OAuth2` error a failed [`Self::get_client`] is answered with: an
    /// unknown client is `invalid_client`, anything else is `server_error`.
    #[must_use]
    pub fn lookup_refusal(error: &AppError) -> OAuth2Error {
        if error.code == ErrorCode::ResourceNotFound {
            OAuth2Error::invalid_client()
        } else {
            OAuth2Error::server_error("The client could not be looked up")
        }
    }

    /// Log a failed [`Self::get_client`] at the level its cause deserves.
    ///
    /// An unknown `client_id` is the caller's mistake, anyone can send one, so
    /// it is a warning. Anything else is the server failing to answer (the
    /// database, or a stored row that no longer decodes) and is an error,
    /// which is what pages the operators.
    pub fn log_lookup_failure(client_id: &str, error: &AppError) {
        if error.code == ErrorCode::ResourceNotFound {
            warn!(client_id = ?client_id, "OAuth2 client refused: unknown client_id");
        } else {
            error!(client_id = %client_id, error = %error, "OAuth2 client lookup failed");
        }
    }

    /// Delete the registrations the retention policy no longer keeps, as of `now`.
    ///
    /// Two kinds go: a registration whose `expires_at` is more than
    /// `expired_grace_secs` before `now`, and one no user ever authorized that
    /// was made more than `abandoned_after_secs` before `now`. An age reaching
    /// back past the Unix epoch keeps every row of its kind.
    ///
    /// # Errors
    /// Returns an error if the database delete fails; nothing is deleted then.
    pub async fn sweep_stale_clients(
        &self,
        retention: &ClientRetentionConfig,
        now: DateTime<Utc>,
    ) -> AppResult<OAuth2ClientSweep> {
        self.oauth2
            .delete_stale_clients(
                retention_cutoff(now, retention.expired_grace_secs),
                retention_cutoff(now, retention.abandoned_after_secs),
            )
            .await
    }

    /// Validate registration request
    fn validate_registration_request(
        request: &ClientRegistrationRequest,
        auth_method: Option<&str>,
    ) -> Result<(), OAuth2Error> {
        // Validate redirect URIs
        if request.redirect_uris.is_empty() {
            return Err(OAuth2Error::invalid_request(
                "At least one redirect_uri is required",
            ));
        }

        for uri in &request.redirect_uris {
            if !Self::is_valid_redirect_uri(uri) {
                return Err(OAuth2Error::invalid_request(&format!(
                    "Invalid redirect_uri: {uri}"
                )));
            }
        }

        // Validate the client authentication method (RFC 7591 §2)
        if let Some(method) = auth_method {
            if !matches!(
                method,
                "none" | "client_secret_basic" | "client_secret_post"
            ) {
                return Err(OAuth2Error::invalid_client_metadata(
                    "token_endpoint_auth_method must be none, client_secret_basic or client_secret_post",
                ));
            }
        }

        // Validate grant types
        if let Some(ref grant_types) = request.grant_types {
            for grant_type in grant_types {
                if !Self::is_supported_grant_type(grant_type) {
                    return Err(OAuth2Error::invalid_request(&format!(
                        "Unsupported grant_type: {grant_type}"
                    )));
                }
            }
        }

        // Validate response types
        if let Some(ref response_types) = request.response_types {
            for response_type in response_types {
                if !Self::is_supported_response_type(response_type) {
                    return Err(OAuth2Error::invalid_request(&format!(
                        "Unsupported response_type: {response_type}"
                    )));
                }
            }
        }

        // The name and URI are stored as given. `PostgreSQL` rejects a NUL byte
        // in a text value, so a control character here would fail the insert
        // and read as a database outage; it is the caller's metadata instead.
        let named = [
            request.client_name.as_deref(),
            request.client_uri.as_deref(),
        ];
        if named
            .into_iter()
            .flatten()
            .any(|v| v.chars().any(char::is_control))
        {
            return Err(OAuth2Error::invalid_client_metadata(
                "client_name and client_uri must not contain control characters",
            ));
        }

        Ok(())
    }

    /// Check if redirect URI is valid
    pub(super) fn is_valid_redirect_uri(uri: &str) -> bool {
        // OAuth 2.0 Security Best Practices (RFC 6749 Section 3.1.2.2)
        // - MUST be absolute URI
        // - MUST NOT include fragment component
        // - SHOULD use https:// except for localhost/loopback

        if !Self::validate_uri_format(uri) {
            return false;
        }

        // The out-of-band URN (`urn:ietf:wg:oauth:2.0:oob`) is refused: OAuth
        // 2.1 drops it, and it is not an http(s) URI, so it fails the scheme
        // check below.
        // Parse and validate HTTP(S) URIs
        Self::validate_http_uri(uri)
    }

    /// Validate basic URI format requirements
    fn validate_uri_format(uri: &str) -> bool {
        // Reject empty or whitespace-only URIs
        if uri.trim().is_empty() {
            return false;
        }

        // Reject URIs with fragments (security risk - RFC 6749 Section 3.1.2)
        if uri.contains('#') {
            warn!("Rejected redirect_uri with fragment: {}", redact_url(uri));
            return false;
        }

        // Reject wildcard patterns (subdomain bypass attack prevention)
        if uri.contains('*') {
            warn!("Rejected redirect_uri with wildcard: {}", redact_url(uri));
            return false;
        }

        // Reject a backslash, whitespace or a control character: URL parsers
        // disagree on them (WHATWG reads `\` as `/` in an http(s) URL, and
        // strips tabs and newlines), so one registered string could name one
        // host to a reader and send the browser to another.
        if uri
            .chars()
            .any(|c| c == '\\' || c.is_whitespace() || c.is_control())
        {
            warn!(
                "Rejected redirect_uri with a backslash, whitespace or control character: {}",
                redact_url(uri)
            );
            return false;
        }

        true
    }

    /// Validate HTTP(S) URI scheme and host
    fn validate_http_uri(uri: &str) -> bool {
        let Ok(parsed_uri) = Url::parse(uri) else {
            warn!("Rejected malformed redirect_uri: {}", redact_url(uri));
            return false;
        };

        // A user-info part (`https://trusted.example@evil.example/`) makes the
        // URI read as naming a host it does not go to.
        if !parsed_uri.username().is_empty() || parsed_uri.password().is_some() {
            warn!("Rejected redirect_uri with user info: {}", redact_url(uri));
            return false;
        }

        let scheme = parsed_uri.scheme();
        let is_localhost = is_loopback_host(&parsed_uri);

        if scheme == "https" {
            // HTTPS is always allowed
            return true;
        }

        if scheme == "http" && is_localhost {
            // HTTP only allowed for localhost/loopback
            return true;
        }

        warn!(
            "Rejected redirect_uri with non-HTTPS scheme for non-localhost: {}",
            redact_url(uri)
        );
        false
    }

    /// Check if grant type is supported by self-service registration.
    ///
    /// `client_credentials` is not: it mints a token bound to no user, and an
    /// anonymous endpoint has no business issuing one. A machine client is
    /// provisioned by the server through its own surface
    /// ([`Self::register_client_credentials_client`]).
    fn is_supported_grant_type(grant_type: &str) -> bool {
        matches!(grant_type, "authorization_code" | "refresh_token")
    }

    /// Check if response type is supported
    fn is_supported_response_type(response_type: &str) -> bool {
        matches!(response_type, "code")
    }

    /// Generate client ID
    fn generate_client_id() -> String {
        format!("mcp_client_{}", Uuid::new_v4().simple())
    }

    /// Get default `client_uri` for OAuth client registration
    ///
    /// Uses `BASE_URL` environment variable if set (production), falls back to localhost:8081 (tests).
    /// Respects `BASE_URL` scheme for TLS/reverse-proxy deployments.
    fn get_default_client_uri() -> String {
        env::var("BASE_URL").unwrap_or_else(|_| "http://localhost:8081".to_owned())
    }

    /// Generate client secret
    ///
    /// # Errors
    /// Returns an error if the system RNG fails to generate cryptographically secure random bytes
    fn generate_client_secret() -> Result<String, OAuth2Error> {
        let rng = SystemRandom::new();
        let mut secret = [0u8; 32];
        rng.fill(&mut secret).map_err(|e| {
            error!(error = ?e, "System RNG failure - cannot generate secure client secret (CRITICAL SECURITY ISSUE)");
            OAuth2Error::server_error("System RNG failure - cannot generate secure client secret")
        })?;

        // Base64 encode the secret
        Ok(general_purpose::STANDARD.encode(secret))
    }

    /// Hash client secret for storage using Argon2id
    ///
    /// Uses Argon2id with a random salt for secure password hashing.
    /// Argon2id provides resistance against GPU-based attacks and side-channel attacks.
    ///
    /// # Errors
    /// Returns an error if Argon2 password hashing fails
    fn hash_client_secret(secret: &str) -> Result<String, OAuth2Error> {
        let salt = SaltString::generate(&mut OsRng);
        let argon2 = Argon2::default();

        let hash = argon2
            .hash_password(secret.as_bytes(), &salt)
            .map_err(|e| {
                // A server fault: logged here, and kept out of the response.
                error!(error = %e, "Argon2 could not hash a new client secret");
                OAuth2Error::server_error("Failed to secure the client secret")
            })?;

        Ok(hash.to_string())
    }
}

/// The instant `age_secs` before `now`, floored at the Unix epoch.
///
/// No registration is stamped before the epoch, and the epoch is inside the
/// range both engines' timestamps accept — `PostgreSQL` refuses a year before
/// 4713 BC outright — so an age reaching back past it keeps every row rather
/// than failing the sweep or deleting everything.
fn retention_cutoff(now: DateTime<Utc>, age_secs: u64) -> DateTime<Utc> {
    i64::try_from(age_secs)
        .ok()
        .and_then(Duration::try_seconds)
        .and_then(|age| now.checked_sub_signed(age))
        .unwrap_or(DateTime::UNIX_EPOCH)
        .max(DateTime::UNIX_EPOCH)
}
