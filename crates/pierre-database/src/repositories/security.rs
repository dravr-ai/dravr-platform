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
    /// Save RSA keypair to database for persistence across restarts
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
    async fn get_or_create_system_secret(&self, secret_type: &str) -> AppResult<String>;
    /// Get existing system secret
    async fn get_system_secret(&self, secret_type: &str) -> AppResult<String>;
    /// Update system secret (for rotation)
    async fn update_system_secret(&self, secret_type: &str, new_value: &str) -> AppResult<()>;
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

/// Save the keypair under its key id, replacing the PEMs and the active
/// flag of a row that already carries that id.
pub(crate) const SAVE_RSA_KEYPAIR_SQL: &str = r"
            INSERT INTO rsa_keypairs (kid, private_key_pem, public_key_pem, created_at, is_active, key_size_bits)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT(kid) DO UPDATE SET
                private_key_pem = EXCLUDED.private_key_pem,
                public_key_pem = EXCLUDED.public_key_pem,
                is_active = EXCLUDED.is_active
            ";

/// Every stored keypair, newest first.
pub(crate) const LOAD_RSA_KEYPAIRS_SQL: &str = "SELECT kid, private_key_pem, public_key_pem, created_at, is_active FROM rsa_keypairs ORDER BY created_at DESC";

/// Rewrite one row's private half; the read path uses it to upgrade a
/// plaintext row to ciphertext.
pub(crate) const REWRITE_RSA_PRIVATE_KEY_SQL: &str =
    "UPDATE rsa_keypairs SET private_key_pem = $1 WHERE kid = $2";

/// Create a system secret; the caller has just checked none exists.
pub(crate) const INSERT_SYSTEM_SECRET_SQL: &str = "INSERT INTO system_secrets (secret_type, secret_value, created_at, updated_at) VALUES ($1, $2, $3, $4)";

/// The value stored under a secret name.
pub(crate) const GET_SYSTEM_SECRET_SQL: &str =
    "SELECT secret_value FROM system_secrets WHERE secret_type = $1";

/// Store or rotate a system secret: insert when the name is new, otherwise
/// replace the value and stamp `updated_at`.
pub(crate) const UPSERT_SYSTEM_SECRET_SQL: &str = "INSERT INTO system_secrets (secret_type, secret_value, created_at, updated_at) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT(secret_type) DO UPDATE SET secret_value = EXCLUDED.secret_value, updated_at = EXCLUDED.updated_at";

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
                if let Ok(secret) = self.get_system_secret(secret_type).await {
                    return Ok(secret);
                }

                // Only the admin JWT secret is minted here; the data-encryption
                // keys are wrapped and stored by key management under their own
                // names through update_system_secret.
                let secret_value = match secret_type {
                    "admin_jwt_secret" => AdminJwtManager::generate_jwt_secret(),
                    _ => {
                        return Err(AppError::invalid_input(format!(
                            "Unknown secret type: {secret_type}"
                        )))
                    }
                };

                let now = Utc::now();
                sqlx::query(INSERT_SYSTEM_SECRET_SQL)
                    .bind(secret_type)
                    .bind(&secret_value)
                    .bind(now)
                    .bind(now)
                    .execute(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

                Ok(secret_value)
            }

            async fn get_system_secret(&self, secret_type: &str) -> AppResult<String> {
                let row = sqlx::query(GET_SYSTEM_SECRET_SQL)
                    .bind(secret_type)
                    .fetch_one(self.pool())
                    .await
                    .map_err(|e| AppError::database(format!("Database query failed: {e}")))?;

                row.try_get("secret_value")
                    .map_err(|e| AppError::database(format!("Failed to get secret_value: {e}")))
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
