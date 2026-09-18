// ABOUTME: Admin token authentication module — AdminAuthService + Axum middleware over the pierre-core JWT manager
// ABOUTME: Re-exports AdminJwtManager and TokenGenerationConfig from pierre_core::admin so one implementation mints and validates
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Admin Token System
//!
//! This module provides secure admin authentication for API key provisioning.
//! Admin services can authenticate using JWT tokens to provision, revoke, and
//! manage API keys for users.

/// Admin authentication service + Axum middleware.
pub mod service;

/// Admin JWT minting, validation and hashing — one implementation, shared with the
/// repository layer that stores the minted token.
pub use pierre_core::admin::{AdminJwtManager, TokenGenerationConfig};
pub use service::AdminAuthService;

/// Admin authentication middleware (`pub mod middleware`-shaped).
pub mod middleware {
    pub use super::service::middleware::admin_auth_middleware;
}

// JWKS (JSON Web Key Set) management for asymmetric JWT (via pierre-auth crate)

/// JSON Web Key representation
pub use pierre_auth::admin::jwks::JsonWebKey;
/// JSON Web Key Set container
pub use pierre_auth::admin::jwks::JsonWebKeySet;
/// JWKS manager for key rotation
pub use pierre_auth::admin::jwks::JwksManager;
/// RSA key pair for JWT signing
pub use pierre_auth::admin::jwks::RsaKeyPair;

// Admin system data models and permissions live in pierre_core::admin::models;
// callers import from there directly.

// Firebase Authentication types live in pierre_auth::firebase; callers import directly.
