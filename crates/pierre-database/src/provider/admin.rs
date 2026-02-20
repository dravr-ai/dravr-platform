// ABOUTME: Admin database operations trait covering tokens, RSA keys, and user MCP tokens
// ABOUTME: Enables AdminRepository, ImpersonationRepository, and UserMcpTokenRepository blanket impls
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::admin::jwt::JwtSigner;
use pierre_core::admin::models::{
    AdminToken, AdminTokenUsage, CreateAdminTokenRequest, GeneratedAdminToken,
};
use pierre_core::errors::AppResult;
use pierre_core::models::{
    CreateUserMcpTokenRequest, UserMcpToken, UserMcpTokenCreated, UserMcpTokenInfo,
};
use pierre_core::permissions::impersonation::ImpersonationSession;
use uuid::Uuid;

/// Admin token, RSA key, impersonation, and user MCP token database operations
#[async_trait]
pub trait AdminDbOps: Send + Sync + Clone {
    // --- Admin Token Management ---

    /// Create a new admin token
    async fn create_admin_token(
        &self,
        request: &CreateAdminTokenRequest,
        admin_jwt_secret: &str,
        jwks_manager: &dyn JwtSigner,
    ) -> AppResult<GeneratedAdminToken>;

    /// Get admin token by ID
    async fn get_admin_token_by_id(&self, token_id: &str) -> AppResult<Option<AdminToken>>;

    /// Get admin token by prefix for fast lookup
    async fn get_admin_token_by_prefix(&self, token_prefix: &str) -> AppResult<Option<AdminToken>>;

    /// List all admin tokens (super admin only)
    async fn list_admin_tokens(&self, include_inactive: bool) -> AppResult<Vec<AdminToken>>;

    /// Deactivate admin token
    async fn deactivate_admin_token(&self, token_id: &str) -> AppResult<()>;

    /// Update admin token last used timestamp
    async fn update_admin_token_last_used(
        &self,
        token_id: &str,
        ip_address: Option<&str>,
    ) -> AppResult<()>;

    /// Record admin token usage for audit trail
    async fn record_admin_token_usage(&self, usage: &AdminTokenUsage) -> AppResult<()>;

    /// Get admin token usage history
    async fn get_admin_token_usage_history(
        &self,
        token_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<Vec<AdminTokenUsage>>;

    /// Record API key provisioning by admin
    async fn record_admin_provisioned_key(
        &self,
        admin_token_id: &str,
        api_key_id: &str,
        user_email: &str,
        tier: &str,
        rate_limit_requests: u32,
        rate_limit_period: &str,
    ) -> AppResult<()>;

    /// Get admin provisioned keys history
    async fn get_admin_provisioned_keys(
        &self,
        admin_token_id: Option<&str>,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<Vec<serde_json::Value>>;

    // --- User MCP Tokens (AI Client Authentication) ---

    /// Create a new user MCP token for AI client authentication
    async fn create_user_mcp_token(
        &self,
        user_id: Uuid,
        request: &CreateUserMcpTokenRequest,
    ) -> AppResult<UserMcpTokenCreated>;

    /// Validate a user MCP token and return the associated user ID
    async fn validate_user_mcp_token(&self, token_value: &str) -> AppResult<Uuid>;

    /// List all MCP tokens for a user
    async fn list_user_mcp_tokens(&self, user_id: Uuid) -> AppResult<Vec<UserMcpTokenInfo>>;

    /// Revoke a user MCP token
    async fn revoke_user_mcp_token(&self, token_id: &str, user_id: Uuid) -> AppResult<()>;

    /// Get a user MCP token by ID
    async fn get_user_mcp_token(
        &self,
        token_id: &str,
        user_id: Uuid,
    ) -> AppResult<Option<UserMcpToken>>;

    /// Cleanup expired user MCP tokens (mark as revoked)
    async fn cleanup_expired_user_mcp_tokens(&self) -> AppResult<u64>;

    // --- Impersonation Session Management ---

    /// Create a new impersonation session for audit trail
    async fn create_impersonation_session(&self, session: &ImpersonationSession) -> AppResult<()>;

    /// Get impersonation session by ID
    async fn get_impersonation_session(
        &self,
        session_id: &str,
    ) -> AppResult<Option<ImpersonationSession>>;

    /// Get active impersonation session where user is impersonator or target
    async fn get_active_impersonation_session(
        &self,
        user_id: Uuid,
    ) -> AppResult<Option<ImpersonationSession>>;

    /// End an impersonation session
    async fn end_impersonation_session(&self, session_id: &str) -> AppResult<()>;

    /// End all active impersonation sessions for an impersonator
    async fn end_all_impersonation_sessions(&self, impersonator_id: Uuid) -> AppResult<u64>;

    /// List impersonation sessions with optional filters
    async fn list_impersonation_sessions(
        &self,
        impersonator_id: Option<Uuid>,
        target_user_id: Option<Uuid>,
        active_only: bool,
        limit: u32,
    ) -> AppResult<Vec<ImpersonationSession>>;
}
