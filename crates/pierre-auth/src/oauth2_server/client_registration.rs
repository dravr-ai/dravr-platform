// ABOUTME: OAuth 2.0 dynamic client registration implementation (RFC 7591)
// ABOUTME: Handles client registration endpoint for MCP clients and other OAuth clients
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

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
use std::env;
use std::sync::Arc;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

/// OAuth 2.0 Client Registration Manager
pub struct ClientRegistrationManager {
    oauth2: Arc<dyn OAuth2ServerRepository>,
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
        // Validate request
        Self::validate_registration_request(&request)?;

        // The grant this client may ever be issued, persisted exactly as the
        // response advertises it (RFC 7591 §3.2.1): the scope it asked for,
        // every name checked against the vocabulary and `admin` refused, or
        // the read-only default when it asked for none. The authorization
        // endpoint checks every request against this row, so what is stored
        // here is the ceiling of every grant the client can obtain.
        let scope = OAuthScope::requested_grant(request.scope.as_deref())
            .map(|granted| OAuthScope::render_granted(&granted))
            .map_err(|e| OAuth2Error::invalid_client_metadata(&e.message))?;

        // Generate client credentials
        let client_id = Self::generate_client_id();
        let client_secret = Self::generate_client_secret()?;
        let client_secret_hash = Self::hash_client_secret(&client_secret)?;

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
        let expires_at = Some(created_at + Duration::days(365)); // 1 year expiry

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
            client_secret,
            client_id_issued_at: Some(created_at.timestamp()),
            client_secret_expires_at: expires_at.map(|dt| dt.timestamp()),
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
    /// [`Self::validate_client`] like any other. The registration may use no
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
        expires_at: Option<chrono::DateTime<Utc>>,
    ) -> Result<(), OAuth2Error> {
        if let Some(expires_at) = expires_at {
            if Utc::now() > expires_at {
                warn!("OAuth client {} has expired", client_id);
                return Err(OAuth2Error::invalid_client());
            }
        }
        Ok(())
    }

    /// Validate client credentials
    ///
    /// # Errors
    /// Returns an error if client is not found, credentials are invalid, or client is expired
    pub async fn validate_client(
        &self,
        client_id: &str,
        client_secret: &str,
    ) -> Result<OAuth2Client, OAuth2Error> {
        debug!("Validating OAuth client: {}", client_id);

        let client = self.get_client(client_id).await.map_err(|e| {
            Self::log_lookup_failure(client_id, &e);
            Self::lookup_refusal(&e)
        })?;

        debug!("OAuth client {} found, validating secret", client_id);

        // Verify client secret using constant-time comparison via Argon2
        Self::verify_client_secret(client_id, client_secret, &client.client_secret_hash)?;

        // Check if client is expired
        Self::check_client_expiry(client_id, client.expires_at)?;

        info!("OAuth client {} validated successfully", client_id);
        Ok(client)
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
    fn is_valid_redirect_uri(uri: &str) -> bool {
        // OAuth 2.0 Security Best Practices (RFC 6749 Section 3.1.2.2)
        // - MUST be absolute URI
        // - MUST NOT include fragment component
        // - SHOULD use https:// except for localhost/loopback

        if !Self::validate_uri_format(uri) {
            return false;
        }

        // Allow out-of-band URN for native apps (RFC 8252)
        if uri == "urn:ietf:wg:oauth:2.0:oob" {
            return true;
        }

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

        true
    }

    /// Validate HTTP(S) URI scheme and host
    fn validate_http_uri(uri: &str) -> bool {
        let Ok(parsed_uri) = url::Url::parse(uri) else {
            warn!("Rejected malformed redirect_uri: {}", redact_url(uri));
            return false;
        };

        let scheme = parsed_uri.scheme();
        let is_localhost = parsed_uri.host_str() == Some("localhost")
            || parsed_uri.host_str() == Some("127.0.0.1");

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

    /// Check if grant type is supported
    fn is_supported_grant_type(grant_type: &str) -> bool {
        matches!(
            grant_type,
            "authorization_code" | "client_credentials" | "refresh_token"
        )
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
