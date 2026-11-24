// ABOUTME: PostgreSQL encryption support using AES-256-GCM with AEAD
// ABOUTME: Provides at-rest encryption for OAuth tokens and sensitive data with cross-tenant protection
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::PostgresDatabase;
use crate::backends::shared::encryption::{
    hmac_token_for_storage, open_with_aad, seal_with_aad, HasEncryption,
};
use pierre_core::errors::AppResult;

// Implement encryption support for PostgreSQL over the shared AES-256-GCM
// envelope, the same one SQLite writes
impl HasEncryption for PostgresDatabase {
    /// Encrypt data using AES-256-GCM with Additional Authenticated Data
    ///
    /// # Security
    /// - Uses AES-256-GCM (AEAD cipher) via ring crate
    /// - Generates unique 96-bit nonce per encryption
    /// - Binds AAD to prevent cross-tenant token reuse
    /// - Output: base64(nonce || ciphertext || `auth_tag`), tagged with the DEK version
    fn encrypt_data_with_aad(&self, data: &str, aad_context: &str) -> AppResult<String> {
        seal_with_aad(&self.dek_keys(), data, aad_context)
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
        open_with_aad(&self.dek_keys(), encrypted_data, aad_context)
    }

    /// Compute HMAC-SHA256 of a token for secure storage
    ///
    /// Used for refresh tokens where we need deterministic lookups but don't
    /// need to recover the original value.
    fn hash_token_for_storage(&self, token: &str) -> AppResult<String> {
        // Pinned to the blind-index key (DEK v1) so lookups survive DEK rotation.
        Ok(hmac_token_for_storage(&self.blind_index_key, token))
    }
}
