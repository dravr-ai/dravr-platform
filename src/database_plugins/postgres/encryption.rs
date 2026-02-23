// ABOUTME: PostgreSQL encryption support using AES-256-GCM with AEAD
// ABOUTME: Provides at-rest encryption for OAuth tokens and sensitive data with cross-tenant protection
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::PostgresDatabase;
use crate::database_plugins::shared::encryption::HasEncryption;
use crate::errors::{AppError, AppResult};

// Implement encryption support for PostgreSQL (harmonize with SQLite security)
impl HasEncryption for PostgresDatabase {
    /// Encrypt data using AES-256-GCM with Additional Authenticated Data
    ///
    /// This brings `PostgreSQL` to security parity with `SQLite`, which already
    /// encrypts OAuth tokens at rest.
    ///
    /// # Security
    /// - Uses AES-256-GCM (AEAD cipher) via ring crate
    /// - Generates unique 96-bit nonce per encryption
    /// - Binds AAD to prevent cross-tenant token reuse
    /// - Output: base64(nonce || ciphertext || `auth_tag`)
    fn encrypt_data_with_aad(&self, data: &str, aad_context: &str) -> AppResult<String> {
        use base64::{engine::general_purpose, Engine as _};
        use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};
        use ring::rand::{SecureRandom, SystemRandom};

        let rng = SystemRandom::new();

        // Generate unique nonce (96 bits for GCM)
        let mut nonce_bytes = [0u8; 12];
        rng.fill(&mut nonce_bytes)
            .map_err(|e| AppError::database(format!("Failed to generate nonce: {e:?}")))?;
        let nonce = Nonce::assume_unique_for_key(nonce_bytes);

        // Create encryption key
        let unbound_key = UnboundKey::new(&AES_256_GCM, &self.encryption_key)
            .map_err(|e| AppError::database(format!("Failed to create encryption key: {e:?}")))?;
        let key = LessSafeKey::new(unbound_key);

        // Encrypt data with AAD binding
        let mut data_bytes = data.as_bytes().to_vec();
        let aad = Aad::from(aad_context.as_bytes());
        key.seal_in_place_append_tag(nonce, aad, &mut data_bytes)
            .map_err(|e| AppError::database(format!("Encryption failed: {e:?}")))?;

        // Combine nonce and encrypted data, then base64 encode
        let mut combined = nonce_bytes.to_vec();
        combined.extend(data_bytes);

        Ok(general_purpose::STANDARD.encode(combined))
    }

    /// Decrypt data using AES-256-GCM with Additional Authenticated Data
    ///
    /// Reverses `encrypt_data_with_aad`. AAD context must match or decryption fails.
    ///
    /// # Security
    /// - Verifies AAD matches (prevents token context switching)
    /// - Authenticates ciphertext hasn't been tampered
    /// - Fails safely on any mismatch/corruption
    fn decrypt_data_with_aad(&self, encrypted_data: &str, aad_context: &str) -> AppResult<String> {
        use base64::{engine::general_purpose, Engine as _};
        use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM};

        // Decode from base64
        let combined = general_purpose::STANDARD
            .decode(encrypted_data)
            .map_err(|e| AppError::database(format!("Failed to decode base64 data: {e}")))?;

        if combined.len() < 12 {
            return Err(AppError::database(
                "Invalid encrypted data: too short".to_owned(),
            ));
        }

        // Extract nonce and encrypted data
        let (nonce_bytes, encrypted_bytes) = combined.split_at(12);
        let nonce = Nonce::assume_unique_for_key(
            nonce_bytes
                .try_into()
                .map_err(|e| AppError::database(format!("Invalid nonce size: {e:?}")))?,
        );

        // Create decryption key
        let unbound_key = UnboundKey::new(&AES_256_GCM, &self.encryption_key)
            .map_err(|e| AppError::database(format!("Failed to create decryption key: {e:?}")))?;
        let key = LessSafeKey::new(unbound_key);

        // Decrypt data with AAD verification
        let mut decrypted_data = encrypted_bytes.to_vec();
        let aad = Aad::from(aad_context.as_bytes());
        let decrypted = key
            .open_in_place(nonce, aad, &mut decrypted_data)
            .map_err(|e| {
                AppError::database(format!(
                    "Decryption failed (possible AAD mismatch or tampered data): {e:?}"
                ))
            })?;

        String::from_utf8(decrypted.to_vec()).map_err(|e| {
            AppError::database(format!("Failed to convert decrypted data to string: {e}"))
        })
    }

    /// Compute HMAC-SHA256 of a token for secure storage
    ///
    /// Used for refresh tokens where we need deterministic lookups but don't
    /// need to recover the original value.
    fn hash_token_for_storage(&self, token: &str) -> AppResult<String> {
        use base64::{engine::general_purpose, Engine as _};
        use ring::hmac;

        let key = hmac::Key::new(hmac::HMAC_SHA256, &self.encryption_key);
        let tag = hmac::sign(&key, token.as_bytes());
        Ok(general_purpose::STANDARD.encode(tag.as_ref()))
    }
}
