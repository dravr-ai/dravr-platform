// ABOUTME: RSA signing-keypair persistence — private key enveloped, public key plain
// ABOUTME: The key that signs user sessions, so it is stored the way other secrets are
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Persistence for the RSA keypair that signs JWTs.
//!
//! The private half is the most sensitive row in the schema — it signs user
//! sessions, not just admin tokens — so it goes through the same AES-256-GCM
//! envelope with AAD binding that OAuth tokens, provider client secrets and
//! tenant LLM keys use. The public half stays plaintext because JWKS publishes
//! it. Rows written before the envelope existed are re-encrypted on read
//! rather than failing, so an upgrade cannot lock every session out.

use chrono::{DateTime, Utc};
use tracing::warn;

use super::Database;
use crate::backends::shared::encryption::{
    decrypt_rsa_private_key, encrypt_rsa_private_key, is_plaintext_private_key_pem,
};
use pierre_core::errors::{AppError, AppResult};

impl Database {
    /// Save RSA keypair to database for persistence across restarts
    ///
    /// The private key is encrypted at rest with AES-256-GCM, bound by AAD to
    /// the key id it is stored under. The public key stays plaintext — it is
    /// published through JWKS.
    ///
    /// # Errors
    ///
    /// Returns an error if encryption or the database operation fails
    pub async fn save_rsa_keypair(
        &self,
        kid: &str,
        private_key_pem: &str,
        public_key_pem: &str,
        created_at: DateTime<Utc>,
        is_active: bool,
        key_size_bits: usize,
    ) -> AppResult<()> {
        let encrypted_private_key = encrypt_rsa_private_key(self, kid, private_key_pem)?;

        sqlx::query(
            r"
            INSERT INTO rsa_keypairs (kid, private_key_pem, public_key_pem, created_at, is_active, key_size_bits)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT(kid) DO UPDATE SET
                private_key_pem = EXCLUDED.private_key_pem,
                public_key_pem = EXCLUDED.public_key_pem,
                is_active = EXCLUDED.is_active
            ",
        )
        .bind(kid)
        .bind(&encrypted_private_key)
        .bind(public_key_pem)
        .bind(created_at)
        .bind(is_active)
        .bind(i64::try_from(key_size_bits).map_err(|e| AppError::invalid_input(format!("RSA key size exceeds maximum supported value: {e}")))?)
        .execute(&self.pool)

            .await

            .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }

    /// Rewrite a plaintext `rsa_keypairs` row as AES-256-GCM ciphertext
    ///
    /// Rows written before the column carried ciphertext are upgraded the
    /// first time they are read. A failure here leaves the row readable, so it
    /// is logged rather than propagated: signing must keep working.
    async fn upgrade_rsa_private_key_storage(&self, kid: &str, private_key_pem: &str) {
        match encrypt_rsa_private_key(self, kid, private_key_pem) {
            Ok(encrypted) => {
                if let Err(e) =
                    sqlx::query("UPDATE rsa_keypairs SET private_key_pem = $1 WHERE kid = $2")
                        .bind(&encrypted)
                        .bind(kid)
                        .execute(&self.pool)
                        .await
                {
                    warn!("Failed to store RSA signing key as ciphertext for kid {kid}: {e}");
                }
            }
            Err(e) => warn!("Failed to encrypt RSA signing key for kid {kid}: {e}"),
        }
    }

    /// Load all RSA keypairs from database
    ///
    /// Private keys are decrypted with the AAD context of the key id they are
    /// stored under.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails or a stored private key
    /// cannot be decrypted
    pub async fn load_rsa_keypairs(
        &self,
    ) -> AppResult<Vec<(String, String, String, DateTime<Utc>, bool)>> {
        use sqlx::Row;

        let rows = sqlx::query(
            "SELECT kid, private_key_pem, public_key_pem, created_at, is_active FROM rsa_keypairs ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)

            .await

            .map_err(|e| AppError::database(format!("Database query failed: {e}")))?;

        let mut keypairs = Vec::new();
        for row in rows {
            let kid: String = row
                .try_get("kid")
                .map_err(|e| AppError::database(format!("Failed to get kid: {e}")))?;
            let stored_private_key: String = row
                .try_get("private_key_pem")
                .map_err(|e| AppError::database(format!("Failed to get private_key_pem: {e}")))?;
            let private_key_pem = if is_plaintext_private_key_pem(&stored_private_key) {
                self.upgrade_rsa_private_key_storage(&kid, &stored_private_key)
                    .await;
                stored_private_key
            } else {
                decrypt_rsa_private_key(self, &kid, &stored_private_key)?
            };
            let public_key_pem: String = row
                .try_get("public_key_pem")
                .map_err(|e| AppError::database(format!("Failed to get public_key_pem: {e}")))?;
            let created_at: DateTime<Utc> = row
                .try_get("created_at")
                .map_err(|e| AppError::database(format!("Failed to get created_at: {e}")))?;
            let is_active: bool = row
                .try_get("is_active")
                .map_err(|e| AppError::database(format!("Failed to get is_active: {e}")))?;

            keypairs.push((kid, private_key_pem, public_key_pem, created_at, is_active));
        }

        Ok(keypairs)
    }

    /// Update active status of RSA keypair
    ///
    /// # Errors
    ///
    /// Returns an error if database operation fails
    pub async fn update_rsa_keypair_active_status(
        &self,
        kid: &str,
        is_active: bool,
    ) -> AppResult<()> {
        sqlx::query("UPDATE rsa_keypairs SET is_active = $1 WHERE kid = $2")
            .bind(is_active)
            .bind(kid)
            .execute(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Database operation failed: {e}")))?;

        Ok(())
    }
}
