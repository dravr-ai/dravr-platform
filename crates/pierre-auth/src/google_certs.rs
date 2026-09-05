// ABOUTME: Google public-certificate cache shared by every Google-signed ID-token verifier
// ABOUTME: Fetches the kid -> X.509 map, honours Cache-Control max-age, serves PEM public keys
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The public keys behind one Google certificate endpoint.
//!
//! Google publishes the certificates that sign its ID tokens as a JSON map
//! of key id to X.509 certificate PEM, at one URL per signing identity —
//! the Firebase `securetoken@system` set and the `accounts.google.com` set
//! share the shape and the `Cache-Control: max-age` contract. A
//! [`GoogleCertCache`] wraps one such URL: it fetches the map on demand,
//! converts each certificate to the RSA public key `jsonwebtoken` needs, and
//! keeps the keys until the server-declared TTL runs out (never below
//! [`MIN_CACHE_TTL_SECS`]).

use std::collections::HashMap;
use std::sync::Arc;

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use x509_parser::prelude::*;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::http_client::{api_client, SharedHttpClient};

/// Minimum cache TTL in seconds (5 minutes)
pub const MIN_CACHE_TTL_SECS: i64 = 300;

/// Default cache TTL in seconds if Cache-Control header is missing (1 hour)
pub const DEFAULT_CACHE_TTL_SECS: i64 = 3600;

/// Cached Google public keys
struct CachedKeys {
    /// Key ID to PEM-encoded public key mapping
    keys: HashMap<String, String>,
    /// When the cache expires
    expires_at: DateTime<Utc>,
}

/// Cache of the public keys behind one Google certificate URL.
///
/// Thread-safe via `Arc<RwLock<_>>` for concurrent access: token
/// validation reads the cache from every request task while a refresh
/// writes it.
pub struct GoogleCertCache {
    /// Certificate endpoint this cache mirrors
    url: String,
    /// HTTP client for fetching public keys
    http_client: &'static SharedHttpClient,
    /// Cached public keys (Arc for sharing across threads)
    cached_keys: Arc<RwLock<Option<CachedKeys>>>,
}

impl GoogleCertCache {
    /// Create an empty cache for the certificate map served at `url`.
    #[must_use]
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            http_client: api_client(),
            cached_keys: Arc::new(RwLock::new(None)),
        }
    }

    /// Get the PEM public key for a given key ID
    ///
    /// Serves from cache while the cached set is fresh; otherwise fetches
    /// the certificate map from Google's endpoint first.
    ///
    /// # Errors
    ///
    /// Returns an error if the endpoint cannot be fetched or parsed, if no
    /// certificate in the response yields a public key, or if the refreshed
    /// set has no key under `kid`.
    pub async fn public_key(&self, kid: &str) -> AppResult<String> {
        // Check if we have a valid cached key
        if let Some(key) = self.try_get_cached_key(kid).await {
            return Ok(key);
        }

        // Fetch fresh keys
        self.refresh_keys().await?;

        // Try to get the key from the refreshed cache
        self.get_cached_key_or_error(kid).await
    }

    /// Try to get a key from the cache if valid
    async fn try_get_cached_key(&self, kid: &str) -> Option<String> {
        let result = {
            let cache = self.cached_keys.read().await;
            cache.as_ref().and_then(|cached| {
                if cached.expires_at > Utc::now() {
                    cached.keys.get(kid).cloned()
                } else {
                    None
                }
            })
        };
        if result.is_some() {
            debug!(kid = %kid, url = %self.url, "Using cached Google public key");
        }
        result
    }

    /// Get a key from cache or return error
    async fn get_cached_key_or_error(&self, kid: &str) -> AppResult<String> {
        let result = {
            let cache = self.cached_keys.read().await;
            cache
                .as_ref()
                .ok_or_else(|| AppError::internal("Failed to fetch Google public keys"))
                .and_then(|cached| {
                    cached.keys.get(kid).cloned().ok_or_else(|| {
                        debug!(kid = %kid, url = %self.url, "Google public key not found for kid");
                        AppError::auth_invalid("Unknown token signing key")
                    })
                })
        };
        result
    }

    /// Refresh the public key cache from Google's endpoint
    async fn refresh_keys(&self) -> AppResult<()> {
        info!(url = %self.url, "Fetching Google public keys");

        let (certs, cache_ttl) = self.fetch_google_certificates().await?;
        let keys = convert_certs_to_keys(certs)?;
        self.update_cache(keys, cache_ttl).await;

        Ok(())
    }

    /// Fetch X.509 certificates from Google's endpoint
    async fn fetch_google_certificates(&self) -> AppResult<(HashMap<String, String>, i64)> {
        let response = self.http_client.get(&self.url).send().await.map_err(|e| {
            warn!(error = %e, url = %self.url, "Failed to fetch Google public keys");
            AppError::internal(format!("Failed to fetch Google public keys: {e}"))
        })?;

        // Parse cache TTL from Cache-Control header
        let cache_ttl = response
            .headers()
            .get("cache-control")
            .and_then(|v| v.to_str().ok())
            .and_then(parse_max_age)
            .unwrap_or(DEFAULT_CACHE_TTL_SECS)
            .max(MIN_CACHE_TTL_SECS);

        // Parse the response body as a map of kid -> X.509 certificate
        let certs: HashMap<String, String> = response.json().await.map_err(|e| {
            warn!(error = %e, url = %self.url, "Failed to parse Google public keys response");
            AppError::internal(format!("Failed to parse Google public keys: {e}"))
        })?;

        Ok((certs, cache_ttl))
    }

    /// Update the key cache with new keys
    async fn update_cache(&self, keys: HashMap<String, String>, cache_ttl: i64) {
        let expires_at = Utc::now() + Duration::seconds(cache_ttl);

        info!(
            num_keys = keys.len(),
            cache_ttl_secs = cache_ttl,
            expires_at = %expires_at,
            url = %self.url,
            "Google public keys cached"
        );

        let mut cache = self.cached_keys.write().await;
        *cache = Some(CachedKeys { keys, expires_at });
    }
}

/// Convert X.509 certificates to PEM-encoded public keys
///
/// A certificate that fails to parse is logged and skipped; the result is
/// an error only when none of them yields a key.
///
/// # Errors
///
/// Returns an error when no certificate in `certs` yields a public key.
pub fn convert_certs_to_keys(
    certs: impl IntoIterator<Item = (String, String)>,
) -> AppResult<HashMap<String, String>> {
    let mut keys = HashMap::new();
    for (kid, cert_pem) in certs {
        match extract_public_key_from_cert(&cert_pem) {
            Ok(public_key_pem) => {
                keys.insert(kid, public_key_pem);
            }
            Err(e) => {
                warn!(kid = %kid, error = %e, "Failed to extract public key from certificate");
            }
        }
    }

    if keys.is_empty() {
        return Err(AppError::internal("No valid Google public keys found"));
    }

    Ok(keys)
}

/// Parse max-age value from Cache-Control header
///
/// Example: "public, max-age=3600, must-revalidate" -> 3600
#[must_use]
pub fn parse_max_age(cache_control: &str) -> Option<i64> {
    cache_control
        .split(',')
        .map(str::trim)
        .find(|s| s.starts_with("max-age="))
        .and_then(|s| s.strip_prefix("max-age="))
        .and_then(|s| s.parse().ok())
}

/// Extract the public key from an X.509 certificate in PEM format
///
/// Google returns X.509 certificates, but JWT validation needs the RSA
/// public key they carry.
///
/// # Errors
///
/// Returns an error when the PEM or the DER certificate inside it does not parse.
pub fn extract_public_key_from_cert(cert_pem: &str) -> AppResult<String> {
    // Parse the PEM-encoded certificate
    let (_, pem) = parse_x509_pem(cert_pem.as_bytes())
        .map_err(|e| AppError::internal(format!("Failed to parse X.509 PEM: {e}")))?;

    // Parse the X.509 certificate
    let (_, cert) = X509Certificate::from_der(&pem.contents)
        .map_err(|e| AppError::internal(format!("Failed to parse X.509 certificate: {e}")))?;

    // Get the subject public key info (SPKI)
    let spki = cert.public_key();

    // Convert SPKI to PEM format
    // The SPKI is already in DER format, we just need to PEM-encode it
    let spki_der = spki.raw;
    let pem_encoded = format!(
        "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----",
        STANDARD
            .encode(spki_der)
            .chars()
            .collect::<Vec<_>>()
            .chunks(64)
            .map(|c| c.iter().collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    );

    Ok(pem_encoded)
}
