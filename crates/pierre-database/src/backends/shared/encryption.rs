// ABOUTME: Encryption/decryption utilities for OAuth tokens and sensitive data.
// ABOUTME: Uses AES-256-GCM with AAD binding for secure data at rest across database backends.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Encryption/decryption utilities for OAuth tokens and sensitive data
//!
//! This module harmonizes encryption across PostgreSQL and SQLite, ensuring
//! consistent security for sensitive data at rest using AES-256-GCM with AAD binding.

use pierre_core::errors::AppResult;
use uuid::Uuid;

/// Create AAD (Additional Authenticated Data) context for token encryption
///
/// Format: `"{tenant_id}|{user_id}|{provider}|{table}"`
///
/// This prevents cross-tenant token reuse attacks by binding the encrypted
/// token to its specific context. If an attacker copies an encrypted token
/// to a different tenant/user/provider context, decryption will fail due to
/// AAD mismatch.
///
/// # Arguments
/// * `tenant_id` - Tenant ID (or "default" for single-tenant)
/// * `user_id` - User UUID
/// * `provider` - OAuth provider (e.g., "strava", "fitbit", "google")
/// * `table` - Database table name (e.g., `"user_oauth_tokens"`)
///
/// # Returns
/// AAD context string in format: `"{tenant_id}|{user_id}|{provider}|{table}"`
///
/// # Examples
/// ```
/// # use pierre_database::backends::shared::encryption::create_token_aad_context;
/// # use uuid::Uuid;
/// let user_id = Uuid::new_v4();
/// let aad = create_token_aad_context("tenant-123", user_id, "strava", "user_oauth_tokens");
/// assert!(aad.contains("tenant-123"));
/// assert!(aad.contains("strava"));
/// ```
#[must_use]
pub fn create_token_aad_context(
    tenant_id: &str,
    user_id: Uuid,
    provider: &str,
    table: &str,
) -> String {
    format!("{tenant_id}|{user_id}|{provider}|{table}")
}

/// Encrypt OAuth token with AAD binding
///
/// Uses AES-256-GCM encryption with Additional Authenticated Data to prevent
/// cross-tenant token reuse. The AAD context binds the encrypted token to
/// its specific tenant/user/provider combination.
///
/// # Arguments
/// * `db` - Database implementing `HasEncryption` trait
/// * `token` - Plain-text OAuth token to encrypt
/// * `tenant_id` - Tenant ID
/// * `user_id` - User UUID
/// * `provider` - OAuth provider name
///
/// # Returns
/// * `Ok(String)` - Base64-encoded encrypted token with nonce
///
/// # Errors
/// * Returns error if encryption fails
///
/// # Security
/// - Uses AES-256-GCM (AEAD cipher)
/// - Unique nonce per encryption
/// - AAD prevents token tampering and context switching
/// - Compliant with GDPR, HIPAA, SOC 2 encryption-at-rest requirements
///
/// # Examples
/// ```text
/// let encrypted = shared::encryption::encrypt_oauth_token(
///     db,
///     "access_token_here",
///     "tenant-123",
///     user_id,
///     "strava"
/// )?;
/// ```
pub fn encrypt_oauth_token<D>(
    db: &D,
    token: &str,
    tenant_id: &str,
    user_id: Uuid,
    provider: &str,
) -> AppResult<String>
where
    D: HasEncryption,
{
    let aad_context = create_token_aad_context(tenant_id, user_id, provider, "user_oauth_tokens");
    db.encrypt_data_with_aad(token, &aad_context)
}

/// Decrypt OAuth token with AAD binding
///
/// Reverses `encrypt_oauth_token`. The same AAD context used for encryption
/// MUST be provided or decryption will fail (authentication error).
///
/// # Arguments
/// * `db` - Database implementing `HasEncryption` trait
/// * `encrypted_token` - Base64-encoded encrypted token (from database)
/// * `tenant_id` - Tenant ID (must match encryption context)
/// * `user_id` - User UUID (must match encryption context)
/// * `provider` - OAuth provider name (must match encryption context)
///
/// # Returns
/// * `Ok(String)` - Decrypted plain-text token
///
/// # Errors
/// * Returns error if:
///   - Decryption fails (wrong key)
///   - AAD mismatch (token moved to different context)
///   - Data corrupted/tampered
///
/// # Security
/// AAD verification ensures the token hasn't been:
/// - Copied to a different tenant
/// - Reassigned to a different user
/// - Switched to a different provider
///
/// # Examples
/// ```text
/// let plain_token = shared::encryption::decrypt_oauth_token(
///     db,
///     &encrypted_from_db,
///     "tenant-123",
///     user_id,
///     "strava"
/// )?;
/// ```
pub fn decrypt_oauth_token<D>(
    db: &D,
    encrypted_token: &str,
    tenant_id: &str,
    user_id: Uuid,
    provider: &str,
) -> AppResult<String>
where
    D: HasEncryption,
{
    let aad_context = create_token_aad_context(tenant_id, user_id, provider, "user_oauth_tokens");
    db.decrypt_data_with_aad(encrypted_token, &aad_context)
}

/// Create AAD context binding an RSA private key to the key id that owns it
///
/// Format: `"{kid}|rsa_keypairs"`
///
/// The JWT signing key has no tenant or user to bind to, so the key id is the
/// context: ciphertext copied onto a different `rsa_keypairs` row fails
/// authentication and cannot be pressed into service under another `kid`.
///
/// # Examples
/// ```
/// # use pierre_database::backends::shared::encryption::create_rsa_key_aad_context;
/// let aad = create_rsa_key_aad_context("jwt_signing_2026");
/// assert_eq!(aad, "jwt_signing_2026|rsa_keypairs");
/// ```
#[must_use]
pub fn create_rsa_key_aad_context(kid: &str) -> String {
    format!("{kid}|rsa_keypairs")
}

/// Armour header that opens every PEM-encoded private key export.
const PEM_ARMOUR_PREFIX: &str = "-----BEGIN";

/// Whether a stored `rsa_keypairs.private_key_pem` value is un-encrypted PEM
///
/// Ciphertext produced by [`HasEncryption::encrypt_data_with_aad`] is
/// `"v{N}:{base64}"`, which cannot begin with PEM armour, so rows written
/// before the column carried ciphertext are recognised exactly.
///
/// # Examples
/// ```
/// # use pierre_database::backends::shared::encryption::is_plaintext_private_key_pem;
/// assert!(is_plaintext_private_key_pem("-----BEGIN PRIVATE KEY-----\nMIIE")); // secret-scan-ok: the detector's own fixture — PEM armour with no key material
/// assert!(!is_plaintext_private_key_pem("v1:c29tZSBjaXBoZXJ0ZXh0"));
/// ```
#[must_use]
pub fn is_plaintext_private_key_pem(stored: &str) -> bool {
    stored.trim_start().starts_with(PEM_ARMOUR_PREFIX)
}

/// Encrypt an RSA private key PEM for storage in `rsa_keypairs`
///
/// # Errors
/// Returns an error if encryption fails
pub fn encrypt_rsa_private_key<D>(db: &D, kid: &str, private_key_pem: &str) -> AppResult<String>
where
    D: HasEncryption,
{
    db.encrypt_data_with_aad(private_key_pem, &create_rsa_key_aad_context(kid))
}

/// Decrypt an RSA private key PEM read from `rsa_keypairs`
///
/// Callers separate plaintext rows out with [`is_plaintext_private_key_pem`]
/// first; this handles the ciphertext case.
///
/// # Errors
/// Returns an error if the stored ciphertext is corrupt or its AAD context
/// does not match the key id it was read under
pub fn decrypt_rsa_private_key<D>(db: &D, kid: &str, stored: &str) -> AppResult<String>
where
    D: HasEncryption,
{
    db.decrypt_data_with_aad(stored, &create_rsa_key_aad_context(kid))
}

/// Trait for databases that support encryption
///
/// Both `PostgreSQL` and `SQLite` must implement this trait to use shared
/// encryption helpers. This ensures consistent encryption behavior across
/// database backends.
///
/// # Implementation Requirements
/// - Must use AES-256-GCM (AEAD cipher)
/// - Must generate unique nonce per encryption
/// - Must bind AAD to ciphertext (prevents context switching)
/// - Must encode output as base64 for database storage
///
/// # Examples
/// ```text
/// impl HasEncryption for Database {
///     fn encrypt_data_with_aad(&self, data: &str, aad: &str) -> Result<String> {
///         // AES-256-GCM implementation with ring crate
///         // See src/database/mod.rs:690 for reference
///     }
///
///     fn decrypt_data_with_aad(&self, encrypted: &str, aad: &str) -> Result<String> {
///         // Reverse of encrypt_data_with_aad
///         // See src/database/mod.rs:729 for reference
///     }
/// }
/// ```
pub trait HasEncryption {
    /// Encrypt data using AES-256-GCM with Additional Authenticated Data
    ///
    /// # Arguments
    /// * `data` - Plain-text data to encrypt
    /// * `aad` - Additional Authenticated Data (context binding)
    ///
    /// # Returns
    /// Base64-encoded string containing: nonce (12 bytes) + ciphertext + auth tag
    ///
    /// # Errors
    /// Returns error if encryption fails
    fn encrypt_data_with_aad(&self, data: &str, aad: &str) -> AppResult<String>;

    /// Decrypt data using AES-256-GCM with Additional Authenticated Data
    ///
    /// # Arguments
    /// * `encrypted` - Base64-encoded encrypted data (from `encrypt_data_with_aad`)
    /// * `aad` - Additional Authenticated Data (MUST match encryption AAD)
    ///
    /// # Returns
    /// Decrypted plain-text data
    ///
    /// # Errors
    /// Returns error if AAD doesn't match or data is tampered/corrupted
    fn decrypt_data_with_aad(&self, encrypted: &str, aad: &str) -> AppResult<String>;

    /// Compute a keyed HMAC-SHA256 hash for secure token storage
    ///
    /// Used for refresh tokens where we need deterministic lookups but don't
    /// need to recover the original value. The HMAC key is the database encryption
    /// key, so even if the DB is compromised, tokens cannot be verified without it.
    ///
    /// # Arguments
    /// * `token` - The plaintext token to hash
    ///
    /// # Returns
    /// Base64-encoded HMAC-SHA256 digest
    ///
    /// # Errors
    /// Returns error if HMAC computation fails
    fn hash_token_for_storage(&self, token: &str) -> AppResult<String>;
}

/// Hash a refresh token for secure database storage using HMAC-SHA256
///
/// Refresh tokens are hashed (not encrypted) because they only need to be
/// verified, never recovered. This provides defense-in-depth: if the database
/// is compromised, the attacker cannot replay refresh tokens because they
/// only have the HMAC digest, not the original token value.
///
/// The HMAC is keyed with the database encryption key, preventing offline
/// brute-force attacks even with database access.
///
/// # Arguments
/// * `db` - Database implementing `HasEncryption` trait
/// * `token` - The plaintext refresh token to hash
///
/// # Returns
/// * `Ok(String)` - Base64-encoded HMAC-SHA256 digest for storage/lookup
///
/// # Errors
/// Returns error if HMAC computation fails
pub fn hash_refresh_token<D>(db: &D, token: &str) -> AppResult<String>
where
    D: HasEncryption,
{
    db.hash_token_for_storage(token)
}

/// DEK version assigned to ciphertext written before DEK versioning existed.
///
/// Such ciphertext carries no version prefix; on read it is treated as version 1
/// so existing rows remain decryptable without a data migration.
pub const LEGACY_DEK_VERSION: u32 = 1;

/// Tag a base64 ciphertext payload with the DEK version that produced it.
///
/// The on-disk form is `"v{version}:{payload}"`. This is unambiguous against
/// legacy (un-prefixed) ciphertext because standard base64 never contains `:`.
#[must_use]
pub fn tag_dek_version(version: u32, payload: &str) -> String {
    format!("v{version}:{payload}")
}

/// Split stored ciphertext into its DEK version and base64 payload.
///
/// Recognizes the `"v{N}:{payload}"` form. Any string without a valid `v{N}:`
/// prefix is treated as [`LEGACY_DEK_VERSION`] ciphertext (the whole string is
/// the payload) — base64 cannot contain `:`, so this never misclassifies a real
/// legacy payload.
#[must_use]
pub fn split_dek_version(stored: &str) -> (u32, &str) {
    if let Some(rest) = stored.strip_prefix('v') {
        if let Some((digits, payload)) = rest.split_once(':') {
            if !digits.is_empty() {
                if let Ok(version) = digits.parse::<u32>() {
                    return (version, payload);
                }
            }
        }
    }
    (LEGACY_DEK_VERSION, stored)
}
