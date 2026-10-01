// ABOUTME: JWKS (JSON Web Key Set) initialization helpers for pierre-cli
// ABOUTME: Loads the persisted RSA keypairs for JWT signing, storing the first one insert-if-absent
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_auth::admin::jwks::{load_or_store_first_keypair, JwksManager};
use pierre_core::errors::AppError;
use pierre_database::RepositoryRegistry;
use tracing::info;

/// RSA key size the server signs with in production.
const RSA_KEY_SIZE_BITS: usize = 4096;

/// Initialize the JWKS manager from the keypairs the server signs with.
///
/// Shares the server's boot path, so a failed read is returned and the first
/// keypair is stored under the same fixed key id insert-if-absent: the CLI
/// and a booting server converge on the same row (carnet#696).
pub async fn initialize_jwks_manager(repos: &RepositoryRegistry) -> Result<JwksManager, AppError> {
    info!("Initializing JWKS manager for RS256 admin tokens...");
    load_or_store_first_keypair(repos.security.as_ref(), RSA_KEY_SIZE_BITS).await
}
