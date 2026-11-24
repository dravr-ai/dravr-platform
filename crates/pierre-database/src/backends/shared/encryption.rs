// ABOUTME: Encryption/decryption utilities for OAuth tokens and sensitive data.
// ABOUTME: Uses AES-256-GCM with AAD binding for secure data at rest across database backends.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Encryption/decryption utilities for OAuth tokens and sensitive data
//!
//! This module harmonizes encryption across PostgreSQL and SQLite, ensuring
//! consistent security for sensitive data at rest using AES-256-GCM with AAD binding.

use std::collections::HashMap;

use base64::engine::general_purpose;
use base64::Engine;
use pierre_core::errors::{AppError, AppResult};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use uuid::Uuid;

use super::key_material::{decrypt_stored_secret, StoredSecretKind};

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
/// * `provider` - OAuth provider (e.g., "strava", "whoop", "google")
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
/// * `kind` - Which token column the ciphertext was read from; a failure is
///   reported under it as a key-material failure
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
///     StoredSecretKind::OAuthAccessToken,
///     &encrypted_from_db,
///     "tenant-123",
///     user_id,
///     "strava"
/// )?;
/// ```
pub fn decrypt_oauth_token<D>(
    db: &D,
    kind: StoredSecretKind,
    encrypted_token: &str,
    tenant_id: &str,
    user_id: Uuid,
    provider: &str,
) -> AppResult<String>
where
    D: HasEncryption,
{
    let aad_context = create_token_aad_context(tenant_id, user_id, provider, "user_oauth_tokens");
    decrypt_stored_secret(db, kind, encrypted_token, &aad_context)
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
    decrypt_stored_secret(
        db,
        StoredSecretKind::RsaPrivateKey,
        stored,
        &create_rsa_key_aad_context(kid),
    )
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
///     fn encrypt_data_with_aad(&self, data: &str, aad: &str) -> AppResult<String> {
///         seal_with_aad(&self.dek_keys(), data, aad)
///     }
///
///     fn decrypt_data_with_aad(&self, encrypted: &str, aad: &str) -> AppResult<String> {
///         open_with_aad(&self.dek_keys(), encrypted, aad)
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

/// The data-encryption keys a backend holds, borrowed for one envelope operation.
pub(crate) struct DekKeys<'keys> {
    /// Version of `active_key`; new ciphertext is tagged with it.
    pub(crate) active_version: u32,
    /// DEK that encrypts new data.
    pub(crate) active_key: &'keys [u8],
    /// Retained DEKs of earlier versions, used to decrypt only.
    pub(crate) prior_versions: &'keys HashMap<u32, Vec<u8>>,
}

impl<'keys> DekKeys<'keys> {
    /// The DEK bytes for a ciphertext version: the active DEK for the active
    /// version, a retained prior DEK for an older one.
    ///
    /// # Errors
    ///
    /// Returns an error if no DEK is held for `version`.
    fn key_for_version(&self, version: u32) -> AppResult<&'keys [u8]> {
        if version == self.active_version {
            Ok(self.active_key)
        } else if let Some(key) = self.prior_versions.get(&version) {
            Ok(key)
        } else {
            Err(AppError::internal(format!(
                "No DEK available for ciphertext version {version} (active version is {})",
                self.active_version
            )))
        }
    }
}

/// Encrypt `data` with AES-256-GCM under the active DEK, bound to `aad_context`.
///
/// The one envelope both backends write: a fresh 96-bit nonce, the ciphertext
/// and its authentication tag, base64-encoded and tagged with the DEK version
/// (`"v{N}:{base64(nonce || ciphertext || tag)}"`). The AAD binds the
/// ciphertext to its context (tenant|user|provider|table), so it cannot be
/// moved between contexts or users.
///
/// # Errors
///
/// Returns an error if the nonce cannot be generated, the key is not a valid
/// AES-256 key, or sealing fails.
pub(crate) fn seal_with_aad(
    keys: &DekKeys<'_>,
    data: &str,
    aad_context: &str,
) -> AppResult<String> {
    let rng = SystemRandom::new();

    // Generate unique nonce
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rng.fill(&mut nonce_bytes)
        .map_err(|e| AppError::internal(format!("Failed to generate nonce: {e}")))?;
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    // Create encryption key
    let unbound_key = UnboundKey::new(&AES_256_GCM, keys.active_key)
        .map_err(|e| AppError::internal(format!("Failed to create encryption key: {e}")))?;
    let key = LessSafeKey::new(unbound_key);

    // Encrypt data with AAD binding
    let mut data_bytes = data.as_bytes().to_vec();
    let aad = Aad::from(aad_context.as_bytes());
    key.seal_in_place_append_tag(nonce, aad, &mut data_bytes)
        .map_err(|e| AppError::internal(format!("Failed to encrypt data: {e}")))?;

    // Combine nonce and encrypted data, base64 encode, then tag with the active DEK version
    let mut combined = nonce_bytes.to_vec();
    combined.extend(data_bytes);

    Ok(tag_dek_version(
        keys.active_version,
        &general_purpose::STANDARD.encode(combined),
    ))
}

/// Decrypt an envelope written by [`seal_with_aad`].
///
/// The DEK is chosen by the version the ciphertext is tagged with, and the
/// same AAD context used for encryption must be given or authentication fails.
///
/// # Errors
///
/// Returns an error if no DEK is held for the ciphertext's version, the data
/// is malformed, or authentication fails (AAD mismatch, wrong key, tampering).
pub(crate) fn open_with_aad(
    keys: &DekKeys<'_>,
    encrypted_data: &str,
    aad_context: &str,
) -> AppResult<String> {
    // Split the DEK version tag, then decode the base64 payload
    let (dek_version, payload) = split_dek_version(encrypted_data);
    let dek = keys.key_for_version(dek_version)?;
    let combined = general_purpose::STANDARD
        .decode(payload)
        .map_err(|e| AppError::internal(format!("Failed to decode base64: {e}")))?;

    if combined.len() < NONCE_LEN {
        return Err(AppError::internal("Invalid encrypted data: too short"));
    }

    // Extract nonce and encrypted data
    let (nonce_bytes, encrypted_bytes) = combined.split_at(NONCE_LEN);
    let nonce = Nonce::assume_unique_for_key(
        nonce_bytes
            .try_into()
            .map_err(|e| AppError::internal(format!("Invalid nonce size: {e}")))?,
    );

    // Create decryption key for the resolved DEK version
    let unbound_key = UnboundKey::new(&AES_256_GCM, dek)
        .map_err(|e| AppError::internal(format!("Failed to create decryption key: {e}")))?;
    let key = LessSafeKey::new(unbound_key);

    // Decrypt data with AAD verification
    let mut decrypted_data = encrypted_bytes.to_vec();
    let aad = Aad::from(aad_context.as_bytes());
    let decrypted = key
        .open_in_place(nonce, aad, &mut decrypted_data)
        .map_err(|e| {
            AppError::internal(format!(
                "Decryption failed (possible AAD mismatch or tampered data): {e:?}"
            ))
        })?;

    String::from_utf8(decrypted.to_vec())
        .map_err(|e| AppError::internal(format!("Failed to convert decrypted data to string: {e}")))
}

/// Compute the base64 HMAC-SHA256 of a token for storage.
///
/// Used for refresh tokens, which need a deterministic lookup but never their
/// original value back. Keyed with the blind-index key (DEK v1), not the
/// active DEK, so lookups survive DEK rotation.
#[must_use]
pub(crate) fn hmac_token_for_storage(blind_index_key: &[u8], token: &str) -> String {
    let key = hmac::Key::new(hmac::HMAC_SHA256, blind_index_key);
    let tag = hmac::sign(&key, token.as_bytes());
    general_purpose::STANDARD.encode(tag.as_ref())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use pierre_core::errors::{AppResult, ErrorCode};

    use super::{hmac_token_for_storage, open_with_aad, seal_with_aad, DekKeys};

    const KEY_V1: [u8; 32] = [7u8; 32];
    const KEY_V2: [u8; 32] = [9u8; 32];

    fn keys<'a>(
        active_version: u32,
        active_key: &'a [u8],
        prior_versions: &'a HashMap<u32, Vec<u8>>,
    ) -> DekKeys<'a> {
        DekKeys {
            active_version,
            active_key,
            prior_versions,
        }
    }

    #[test]
    fn a_sealed_value_opens_under_the_same_context() -> AppResult<()> {
        let prior = HashMap::new();
        let keys = keys(1, &KEY_V1, &prior);
        let sealed = seal_with_aad(&keys, "refresh-token", "t|u|strava|user_oauth_tokens")?;
        assert!(sealed.starts_with("v1:"), "{sealed}");
        assert!(!sealed.contains("refresh-token"));
        assert_eq!(
            open_with_aad(&keys, &sealed, "t|u|strava|user_oauth_tokens")?,
            "refresh-token"
        );
        Ok(())
    }

    #[test]
    fn sealing_twice_never_repeats_a_ciphertext() -> AppResult<()> {
        let prior = HashMap::new();
        let keys = keys(1, &KEY_V1, &prior);
        assert_ne!(
            seal_with_aad(&keys, "same", "ctx")?,
            seal_with_aad(&keys, "same", "ctx")?
        );
        Ok(())
    }

    #[test]
    fn another_context_or_key_fails_as_an_internal_error() -> AppResult<()> {
        let prior = HashMap::new();
        let sealed = seal_with_aad(&keys(1, &KEY_V1, &prior), "secret", "tenant-a|u")?;

        let moved = open_with_aad(&keys(1, &KEY_V1, &prior), &sealed, "tenant-b|u");
        assert_eq!(moved.map_err(|e| e.code), Err(ErrorCode::InternalError));

        let wrong_key = open_with_aad(&keys(1, &KEY_V2, &prior), &sealed, "tenant-a|u");
        assert_eq!(wrong_key.map_err(|e| e.code), Err(ErrorCode::InternalError));
        Ok(())
    }

    #[test]
    fn a_prior_version_still_opens_after_rotation() -> AppResult<()> {
        let none = HashMap::new();
        let sealed = seal_with_aad(&keys(1, &KEY_V1, &none), "old", "ctx")?;

        let prior = HashMap::from([(1, KEY_V1.to_vec())]);
        let rotated = keys(2, &KEY_V2, &prior);
        assert_eq!(open_with_aad(&rotated, &sealed, "ctx")?, "old");
        assert!(seal_with_aad(&rotated, "new", "ctx")?.starts_with("v2:"));
        Ok(())
    }

    #[test]
    fn an_untagged_payload_opens_as_version_one() -> AppResult<()> {
        let prior = HashMap::new();
        let keys = keys(1, &KEY_V1, &prior);
        let sealed = seal_with_aad(&keys, "legacy", "ctx")?;
        let untagged = sealed
            .strip_prefix("v1:")
            .expect("bug: a value sealed under DEK v1 carries the v1 tag");
        assert_eq!(open_with_aad(&keys, untagged, "ctx")?, "legacy");
        Ok(())
    }

    #[test]
    fn malformed_ciphertext_is_refused_with_its_reason() {
        let prior = HashMap::new();
        let keys = keys(1, &KEY_V1, &prior);

        let unknown_version = open_with_aad(&keys, "v7:AAAA", "ctx");
        assert_eq!(
            unknown_version.map_err(|e| (e.code, e.message)),
            Err((
                ErrorCode::InternalError,
                "No DEK available for ciphertext version 7 (active version is 1)".to_owned()
            ))
        );

        let too_short = open_with_aad(&keys, "v1:AAAA", "ctx");
        assert_eq!(
            too_short.map_err(|e| (e.code, e.message)),
            Err((
                ErrorCode::InternalError,
                "Invalid encrypted data: too short".to_owned()
            ))
        );

        let not_base64 = open_with_aad(&keys, "v1:not base64!", "ctx");
        assert_eq!(
            not_base64.map_err(|e| e.code),
            Err(ErrorCode::InternalError)
        );
    }

    #[test]
    fn a_token_hash_is_deterministic_and_keyed() {
        let hash = hmac_token_for_storage(&KEY_V1, "token");
        assert_eq!(hash, hmac_token_for_storage(&KEY_V1, "token"));
        assert_ne!(hash, hmac_token_for_storage(&KEY_V2, "token"));
        assert_ne!(hash, hmac_token_for_storage(&KEY_V1, "other"));
        assert!(!hash.contains("token"));
    }
}
