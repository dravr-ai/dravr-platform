// ABOUTME: JWT signing trait abstraction for admin token creation
// ABOUTME: Decouples repository traits from concrete JWKS implementation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use crate::errors::AppResult;

/// Trait for signing JWT tokens using asymmetric keys.
///
/// Abstracts the concrete JWKS key management from the repository layer,
/// allowing repository trait definitions to accept any JWT signer without
/// depending on the full `JwksManager` implementation.
pub trait JwtSigner: Send + Sync {
    /// Sign a JSON claims payload into a JWT string.
    ///
    /// The caller serializes their claims struct to `serde_json::Value`
    /// before passing it here. The implementation handles key selection
    /// and RS256 encoding.
    ///
    /// # Errors
    /// Returns an error if the signing key is unavailable or encoding fails.
    fn sign_token(&self, claims: &serde_json::Value) -> AppResult<String>;
}

impl<T: JwtSigner> JwtSigner for Arc<T> {
    fn sign_token(&self, claims: &serde_json::Value) -> AppResult<String> {
        (**self).sign_token(claims)
    }
}
