// ABOUTME: Repository trait, statements and shared body for the RSA signing keypair and the system secrets
// ABOUTME: One SQL text per operation; the two backends share every bind, so the shell passes only its type
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The security store, written once.
//!
//! Two tables: `rsa_keypairs`, holding the keypair that signs user sessions
//! with its private half enveloped under AES-256-GCM (bound by AAD to the
//! key id) and its public half in clear for JWKS; and `system_secrets`, the
//! name → value store the admin JWT secret and the wrapped data-encryption
//! keys live in. Rows written before the envelope existed are re-encrypted
//! on read rather than failing, so an upgrade cannot lock every session out.
//!
//! Nothing here differs per backend once the binds are chosen: `$n`
//! placeholders serve both drivers; timestamps bind as [`DateTime<Utc>`]
//! (`TIMESTAMPTZ` on Postgres, RFC 3339 text on `SQLite`, which orders
//! correctly under `ORDER BY created_at DESC` because every value shares one
//! offset and width); `is_active` binds and reads as `bool` (`BOOLEAN` on
//! Postgres, `INTEGER` 0/1 on `SQLite`); `key_size_bits` binds as the `i32`
//! the trait carries. The encryption calls go through
//! [`HasEncryption`](crate::backends::shared::encryption::HasEncryption),
//! which both backends implement.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;

/// Security and secret repository
#[async_trait]
pub trait SecurityRepository: Send + Sync {
    /// Save an RSA keypair under its key id, insert-if-absent
    ///
    /// A row that already carries `kid` is never replaced: key material that
    /// signed live sessions must not change under them (carnet#696). A caller
    /// that raced another instance re-reads with [`Self::load_rsa_keypairs`]
    /// and adopts whatever is stored.
    async fn save_rsa_keypair(
        &self,
        kid: &str,
        private_key_pem: &str,
        public_key_pem: &str,
        created_at: DateTime<Utc>,
        is_active: bool,
        key_size_bits: i32,
    ) -> AppResult<()>;
    /// Load all RSA keypairs from database
    async fn load_rsa_keypairs(
        &self,
    ) -> AppResult<Vec<(String, String, String, DateTime<Utc>, bool)>>;
    /// Get or create system secret (generates if not exists)
    ///
    /// Only a definite not-found mints a value, and the mint is
    /// insert-if-absent followed by a re-read, so racing callers converge on
    /// one stored value. Any other read error is returned.
    async fn get_or_create_system_secret(&self, secret_type: &str) -> AppResult<String>;
    /// Get existing system secret
    ///
    /// # Errors
    ///
    /// Returns [`ErrorCode::ResourceNotFound`](pierre_core::errors::ErrorCode::ResourceNotFound)
    /// when no row carries `secret_type`, and a database error for every
    /// other failure, so a caller can tell an absent secret from a failed read.
    async fn get_system_secret(&self, secret_type: &str) -> AppResult<String>;
    /// Store `value` under `secret_type` only when no row carries that name,
    /// then return the value the store holds — the caller's when it was
    /// absent, the existing one otherwise. Never replaces a stored secret.
    async fn insert_system_secret_if_absent(
        &self,
        secret_type: &str,
        value: &str,
    ) -> AppResult<String>;
    /// Update system secret (for rotation)
    async fn update_system_secret(&self, secret_type: &str, new_value: &str) -> AppResult<()>;
    /// Count the rows that hold ciphertext sealed under a Database Encryption
    /// Key, across every table that stores one.
    ///
    /// Key management asks before it mints a key: a fresh key on a database
    /// that already holds ciphertext cannot open any of it (carnet#703).
    async fn count_encrypted_rows(&self) -> AppResult<i64>;
    /// Encrypt data with AAD (Additional Authenticated Data)
    ///
    /// # Errors
    /// Returns an error if encryption fails (invalid key, nonce generation failure)
    fn encrypt_data_with_aad(&self, data: &str, aad: &str) -> AppResult<String>;
    /// Decrypt data with AAD
    ///
    /// # Errors
    /// Returns an error if decryption fails (invalid data, AAD mismatch, tampered data)
    fn decrypt_data_with_aad(&self, encrypted: &str, aad: &str) -> AppResult<String>;
}

/// Save the keypair under its key id; a row that already carries that id is
/// left untouched (carnet#696).
pub(crate) const SAVE_RSA_KEYPAIR_SQL: &str = r"
            INSERT INTO rsa_keypairs (kid, private_key_pem, public_key_pem, created_at, is_active, key_size_bits)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT(kid) DO NOTHING
            ";

/// Every stored keypair, newest first; the key id breaks a timestamp tie so
/// every instance loads the same order and settles on the same active key.
pub(crate) const LOAD_RSA_KEYPAIRS_SQL: &str = "SELECT kid, private_key_pem, public_key_pem, created_at, is_active FROM rsa_keypairs ORDER BY created_at DESC, kid DESC";

/// Rewrite one row's private half; the read path uses it to upgrade a
/// plaintext row to ciphertext.
pub(crate) const REWRITE_RSA_PRIVATE_KEY_SQL: &str =
    "UPDATE rsa_keypairs SET private_key_pem = $1 WHERE kid = $2";

/// Create a system secret unless a row already carries the name; an existing
/// secret is never replaced, so racing creators converge on the first write.
pub(crate) const INSERT_SYSTEM_SECRET_IF_ABSENT_SQL: &str =
    "INSERT INTO system_secrets (secret_type, secret_value, created_at, updated_at) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT(secret_type) DO NOTHING";

/// The value stored under a secret name.
pub(crate) const GET_SYSTEM_SECRET_SQL: &str =
    "SELECT secret_value FROM system_secrets WHERE secret_type = $1";

/// Store or rotate a system secret: insert when the name is new, otherwise
/// replace the value and stamp `updated_at`.
pub(crate) const UPSERT_SYSTEM_SECRET_SQL: &str = "INSERT INTO system_secrets (secret_type, secret_value, created_at, updated_at) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT(secret_type) DO UPDATE SET secret_value = EXCLUDED.secret_value, updated_at = EXCLUDED.updated_at";

/// Rows holding DEK ciphertext, summed over every table that stores one.
///
/// A deployment-wide count with no tenant or user key on purpose: the DEK
/// and the JWT signing keypair are global, so whether any tenant's row holds
/// ciphertext is the question. Only a number leaves the query.
pub(crate) const COUNT_ENCRYPTED_ROWS_SQL: &str = r"
            SELECT (SELECT COUNT(*) FROM user_oauth_tokens)
                 + (SELECT COUNT(*) FROM rsa_keypairs)
                 + (SELECT COUNT(*) FROM tenant_oauth_credentials)
                 + (SELECT COUNT(*) FROM strava_oauth_app_pool)
                 + (SELECT COUNT(*) FROM user_llm_credentials) AS encrypted_rows
            ";

/// Emit the whole [`SecurityRepository`] implementation for one backend
/// type, plus the private read-time upgrade of a plaintext private key.
///
/// The body is written once here; each backend's shell invokes it with its
/// own type, and sqlx resolves the driver from `self.pool()` per expansion.
macro_rules! impl_security_repository {
    ($ty:ty) => {
        impl $ty {
            /// Rewrite a plaintext `rsa_keypairs` row as AES-256-GCM ciphertext
            ///
            /// Rows written before the column carried ciphertext are upgraded the
            /// first time they are read. A failure here leaves the row readable, so it
            /// is logged rather than propagated: signing must keep working.
            async fn upgrade_rsa_private_key_storage(&self, kid: &str, private_key_pem: &str) {
                match encrypt_rsa_private_key(self, kid, private_key_pem) {
                    Ok(encrypted) => {
                        if let Err(e) = sqlx::query(REWRITE_RSA_PRIVATE_KEY_SQL)
                            .bind(&encrypted)
                            .bind(kid)
                            .execute(self.pool())
                            .await
                        {
                            warn!(
                                "Failed to store RSA signing key as ciphertext for kid {kid}: {e}"
                            );
                        }
                    }
                    Err(e) => warn!("Failed to encrypt RSA signing key for kid {kid}: {e}"),
                }
            }
        }

        #[async_trait::async_trait]
        impl SecurityRepository for $ty {
            async fn save_rsa_keypair(
                &self,
                kid: &str,
                private_key_pem: &str,
                public_key_pem: &str,
                created_at: DateTime<Utc>,
                is_active: bool,
                key_size_bits: i32,
            ) -> AppResult<()> {
                let encrypted_private_key = encrypt_rsa_private_key(self, kid, private_key_pem)?;

                sqlx::query(SAVE_RSA_KEYPAIR_SQL)
                    .bind(kid)
                    .bind(&encrypted_private_key)
                    .bind(public_key_pem)
                    .bind(created_at)
                    .bind(is_active)
                    .bind(key_size_bits)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

                Ok(())
            }

            async fn load_rsa_keypairs(
                &self,
            ) -> AppResult<Vec<(String, String, String, DateTime<Utc>, bool)>> {
                let rows = sqlx::query(LOAD_RSA_KEYPAIRS_SQL)
                    .fetch_all(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Database query failed: {e}")))?;

                let mut keypairs = Vec::with_capacity(rows.len());
                for row in rows {
                    let kid: String = row
                        .try_get("kid")
                        .map_err(|e| AppError::database(format!("Failed to get kid: {e}")))?;
                    let stored_private_key: String =
                        row.try_get("private_key_pem").map_err(|e| {
                            AppError::database(format!("Failed to get private_key_pem: {e}"))
                        })?;
                    let private_key_pem = if is_plaintext_private_key_pem(&stored_private_key) {
                        self.upgrade_rsa_private_key_storage(&kid, &stored_private_key)
                            .await;
                        stored_private_key
                    } else {
                        decrypt_rsa_private_key(self, &kid, &stored_private_key)?
                    };
                    let public_key_pem: String = row.try_get("public_key_pem").map_err(|e| {
                        AppError::database(format!("Failed to get public_key_pem: {e}"))
                    })?;
                    let created_at: DateTime<Utc> = row.try_get("created_at").map_err(|e| {
                        AppError::database(format!("Failed to get created_at: {e}"))
                    })?;
                    let is_active: bool = row
                        .try_get("is_active")
                        .map_err(|e| AppError::database(format!("Failed to get is_active: {e}")))?;

                    keypairs.push((kid, private_key_pem, public_key_pem, created_at, is_active));
                }

                Ok(keypairs)
            }

            async fn get_or_create_system_secret(&self, secret_type: &str) -> AppResult<String> {
                match self.get_system_secret(secret_type).await {
                    Ok(secret) => return Ok(secret),
                    Err(e) if e.code == ErrorCode::ResourceNotFound => {}
                    Err(e) => return Err(e),
                }

                // Only the admin JWT secret is minted here; the data-encryption
                // keys are wrapped and stored by key management under their own
                // names through insert_system_secret_if_absent.
                let secret_value = match secret_type {
                    "admin_jwt_secret" => AdminJwtManager::generate_jwt_secret(),
                    _ => {
                        return Err(AppError::invalid_input(format!(
                            "Unknown secret type: {secret_type}"
                        )))
                    }
                };

                self.insert_system_secret_if_absent(secret_type, &secret_value)
                    .await
            }

            async fn get_system_secret(&self, secret_type: &str) -> AppResult<String> {
                let row = sqlx::query(GET_SYSTEM_SECRET_SQL)
                    .bind(secret_type)
                    .fetch_optional(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Database query failed: {e}")))?
                    .ok_or_else(|| AppError::not_found(format!("System secret '{secret_type}'")))?;

                row.try_get("secret_value")
                    .map_err(|e| AppError::database(format!("Failed to get secret_value: {e}")))
            }

            async fn insert_system_secret_if_absent(
                &self,
                secret_type: &str,
                value: &str,
            ) -> AppResult<String> {
                let now = Utc::now();
                sqlx::query(INSERT_SYSTEM_SECRET_IF_ABSENT_SQL)
                    .bind(secret_type)
                    .bind(value)
                    .bind(now)
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

                self.get_system_secret(secret_type).await
            }

            async fn update_system_secret(
                &self,
                secret_type: &str,
                new_value: &str,
            ) -> AppResult<()> {
                let now = Utc::now();
                sqlx::query(UPSERT_SYSTEM_SECRET_SQL)
                    .bind(secret_type)
                    .bind(new_value)
                    .bind(now)
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

                Ok(())
            }

            async fn count_encrypted_rows(&self) -> AppResult<i64> {
                let row = sqlx::query(COUNT_ENCRYPTED_ROWS_SQL)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| {
                        AppError::database(format!("Failed to count encrypted rows: {e}"))
                    })?;
                row.try_get("encrypted_rows")
                    .map_err(|e| AppError::database(format!("Failed to get encrypted_rows: {e}")))
            }

            fn encrypt_data_with_aad(&self, data: &str, aad: &str) -> AppResult<String> {
                HasEncryption::encrypt_data_with_aad(self, data, aad)
            }

            fn decrypt_data_with_aad(&self, encrypted: &str, aad: &str) -> AppResult<String> {
                HasEncryption::decrypt_data_with_aad(self, encrypted, aad)
            }
        }
    };
}
pub(crate) use impl_security_repository;
