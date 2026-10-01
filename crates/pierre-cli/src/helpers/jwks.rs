// ABOUTME: JWKS (JSON Web Key Set) initialization helpers for pierre-cli
// ABOUTME: Loads the persisted RSA keypairs for JWT signing, storing the first one insert-if-absent
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_auth::admin::jwks::JwksManager;
use pierre_core::errors::AppError;
use pierre_database::RepositoryRegistry;
use tracing::{error, info};

/// Generate an RSA keypair and store it insert-if-absent; a row already
/// stored under the same key id is left untouched.
async fn persist_first_keypair(repos: &RepositoryRegistry) -> Result<(), AppError> {
    info!("No persisted RSA keys found, generating the first keypair");
    let kid = format!("key_{}", chrono::Utc::now().format("%Y%m%d_%H%M%S"));
    let mut candidate = JwksManager::new();
    candidate.generate_rsa_key_pair(&kid)?;

    let key_pair = candidate.get_active_key()?;
    let private_pem = key_pair.export_private_key_pem()?;
    let public_pem = key_pair.export_public_key_pem()?;
    repos
        .security
        .save_rsa_keypair(
            &kid,
            &private_pem,
            &public_pem,
            key_pair.created_at,
            true,
            4096,
        )
        .await?;
    info!("Generated RSA keypair {kid} and stored it unless one was already present");
    Ok(())
}

/// Every persisted keypair, storing the first one when the table is empty.
///
/// Only an empty table counts as "no keys"; a failed read is returned.
async fn load_or_store_first_keypair(
    repos: &RepositoryRegistry,
) -> Result<Vec<(String, String, String, chrono::DateTime<chrono::Utc>, bool)>, AppError> {
    let keypairs = repos.security.load_rsa_keypairs().await.inspect_err(|e| {
        error!(error_code = ?e.code, error = %e, "Failed to read the persisted RSA keypairs");
    })?;
    if !keypairs.is_empty() {
        return Ok(keypairs);
    }
    persist_first_keypair(repos).await?;
    repos.security.load_rsa_keypairs().await
}

/// Initialize the JWKS manager from the keypairs the server signs with.
///
/// A failed read is returned, since a key the store does not hold would sign
/// tokens no server accepts. The first keypair is stored insert-if-absent and
/// the table read again, so the CLI and a booting server converge on the same
/// rows (carnet#696).
pub async fn initialize_jwks_manager(repos: &RepositoryRegistry) -> Result<JwksManager, AppError> {
    info!("Initializing JWKS manager for RS256 admin tokens...");
    let keypairs = load_or_store_first_keypair(repos).await?;
    info!(
        "Loading {} persisted RSA keypairs from database",
        keypairs.len()
    );
    let mut jwks_manager = JwksManager::new();
    jwks_manager.load_keys_from_database(keypairs)?;
    Ok(jwks_manager)
}
