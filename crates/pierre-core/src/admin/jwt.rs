// ABOUTME: JWT signing trait and admin token management for authentication
// ABOUTME: Provides JwtSigner abstraction, AdminJwtManager for token generation/hashing, and TokenGenerationConfig
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

// --- Admin JWT utilities (behind `admin-jwt` feature) ---

#[cfg(feature = "admin-jwt")]
use chrono::{DateTime, Duration, Utc};
#[cfg(feature = "admin-jwt")]
use rand::{distributions::Alphanumeric, Rng};
#[cfg(feature = "admin-jwt")]
use serde::{Deserialize, Serialize};
#[cfg(feature = "admin-jwt")]
use sha2::{Digest, Sha256};

#[cfg(feature = "admin-jwt")]
use crate::admin::models::{AdminPermission, AdminPermissions};
#[cfg(feature = "admin-jwt")]
use crate::constants::service_names;
#[cfg(feature = "admin-jwt")]
use crate::errors::AppError;

/// JWT token manager for admin authentication
///
/// Provides cryptographic utilities for admin token generation, hashing,
/// and secret management. The struct is stateless; all operations use
/// either static methods or the `JwtSigner` trait for signing.
#[cfg(feature = "admin-jwt")]
#[derive(Clone)]
pub struct AdminJwtManager {}

#[cfg(feature = "admin-jwt")]
impl Default for AdminJwtManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "admin-jwt")]
impl AdminJwtManager {
    /// Create new JWT manager for RS256 token operations
    #[must_use]
    pub const fn new() -> Self {
        Self {}
    }

    /// Generate a cryptographically secure JWT secret
    #[must_use]
    pub fn generate_jwt_secret() -> String {
        // Generate 64 character (512-bit) random secret
        rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(64)
            .map(char::from)
            .collect()
    }

    /// Hash a JWT secret for storage
    #[must_use]
    pub fn hash_secret(secret: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(secret.as_bytes());
        format!("{:x}", hasher.finalize())
    }

    /// Generate JWT token using RS256 (asymmetric signing)
    ///
    /// # Errors
    /// Returns an error if JWT encoding fails
    pub fn generate_token(
        &self,
        token_id: &str,
        service_name: &str,
        permissions: &AdminPermissions,
        is_super_admin: bool,
        expires_at: Option<DateTime<Utc>>,
        jwks_manager: &dyn JwtSigner,
    ) -> AppResult<String> {
        let now = Utc::now();
        let exp = expires_at.unwrap_or_else(|| now + Duration::days(365));

        let claims = AdminTokenClaims {
            // Standard JWT claims
            iss: service_names::PIERRE_MCP_SERVER.into(),
            sub: token_id.to_owned(),
            aud: service_names::ADMIN_API.into(),
            exp: u64::try_from(exp.timestamp().max(0)).unwrap_or(0),
            iat: u64::try_from(now.timestamp().max(0)).unwrap_or(0),
            nbf: u64::try_from(now.timestamp().max(0)).unwrap_or(0),
            jti: token_id.to_owned(),

            // Custom claims
            service_name: service_name.to_owned(),
            permissions: permissions.to_vec(),
            is_super_admin,
            token_type: "admin".into(),
        };

        // Serialize claims to JSON and sign with RS256 via JwtSigner
        let claims_value = serde_json::to_value(&claims).map_err(|e| {
            AppError::internal(format!("Failed to serialize admin JWT claims: {e}"))
        })?;
        jwks_manager
            .sign_token(&claims_value)
            .map_err(|e| AppError::internal(format!("Failed to generate RS256 admin JWT: {e}")))
    }

    /// Generate token prefix for identification
    #[must_use]
    pub fn generate_token_prefix(token: &str) -> String {
        let token_prefix = &token[..8];
        format!("admin_jwt_{token_prefix}")
    }

    /// Hash token for storage (bcrypt-compatible)
    ///
    /// # Errors
    /// Returns an error if bcrypt hashing fails
    pub fn hash_token_for_storage(token: &str) -> AppResult<String> {
        bcrypt::hash(token, bcrypt::DEFAULT_COST)
            .map_err(|e| AppError::internal(format!("Failed to hash token: {e}")))
    }

    /// Verify token hash
    ///
    /// # Errors
    /// Returns an error if bcrypt verification fails
    pub fn verify_token_hash(token: &str, hash: &str) -> AppResult<bool> {
        bcrypt::verify(token, hash)
            .map_err(|e| AppError::internal(format!("Failed to verify token hash: {e}")))
    }
}

/// JWT claims for admin tokens
#[cfg(feature = "admin-jwt")]
#[derive(Debug, Clone, Serialize, Deserialize)]
struct AdminTokenClaims {
    // Standard JWT claims
    iss: String, // Issuer: "pierre-mcp-server"
    sub: String, // Subject: token ID
    aud: String, // Audience: "admin-api"
    exp: u64,    // Expiration time
    iat: u64,    // Issued at
    nbf: u64,    // Not before
    jti: String, // JWT ID: token ID

    // Custom claims
    service_name: String,
    permissions: Vec<AdminPermission>,
    is_super_admin: bool,
    token_type: String, // Always "admin"
}

/// Token generation configuration
///
/// Encapsulates all parameters needed to generate an admin JWT token,
/// including service identity, permissions, and expiration settings.
#[cfg(feature = "admin-jwt")]
#[derive(Debug, Clone)]
pub struct TokenGenerationConfig {
    /// Service name for the token
    pub service_name: String,
    /// Optional human-readable description
    pub service_description: Option<String>,
    /// Permissions granted to this token
    pub permissions: Option<AdminPermissions>,
    /// Token expiration in days (None for no expiration)
    pub expires_in_days: Option<u64>,
    /// Whether this is a super admin token with full privileges
    pub is_super_admin: bool,
}

#[cfg(feature = "admin-jwt")]
impl TokenGenerationConfig {
    /// Create config for regular admin token
    #[must_use]
    pub fn regular_admin(service_name: String) -> Self {
        Self {
            service_name,
            service_description: None,
            permissions: Some(AdminPermissions::default_admin()),
            expires_in_days: Some(365), // 1 year
            is_super_admin: false,
        }
    }

    /// Create config for super admin token
    #[must_use]
    pub fn super_admin(service_name: String) -> Self {
        Self {
            service_name,
            service_description: Some("Super Admin Token".into()),
            permissions: Some(AdminPermissions::super_admin()),
            expires_in_days: None, // Never expires
            is_super_admin: true,
        }
    }

    /// Get effective permissions
    #[must_use]
    pub fn get_permissions(&self) -> AdminPermissions {
        self.permissions.as_ref().map_or_else(
            || {
                if self.is_super_admin {
                    AdminPermissions::super_admin()
                } else {
                    AdminPermissions::default_admin()
                }
            },
            Clone::clone,
        )
    }

    /// Get expiration date
    #[must_use]
    pub fn get_expiration(&self) -> Option<DateTime<Utc>> {
        self.expires_in_days
            .map(|days| Utc::now() + Duration::days(i64::try_from(days).unwrap_or(365)))
    }
}
