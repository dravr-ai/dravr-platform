// ABOUTME: Repository trait definitions for the admin tokens and admin overrides domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
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

use serde_json::Value;

/// Admin token management repository
#[async_trait]
pub trait AdminRepository: Send + Sync {
    /// Create a new admin token
    async fn create_token(
        &self,
        request: &CreateAdminTokenRequest,
        admin_jwt_secret: &str,
        jwks_manager: &dyn JwtSigner,
    ) -> AppResult<GeneratedAdminToken>;
    /// Get admin token by ID
    async fn get_token_by_id(&self, token_id: &str) -> AppResult<Option<AdminToken>>;
    /// Get admin token by prefix for fast lookup
    async fn get_token_by_prefix(&self, token_prefix: &str) -> AppResult<Option<AdminToken>>;
    /// List all admin tokens (super admin only)
    async fn list_tokens(&self, include_inactive: bool) -> AppResult<Vec<AdminToken>>;
    /// Deactivate admin token
    async fn deactivate_token(&self, token_id: &str) -> AppResult<()>;
    /// Update admin token last used timestamp
    async fn update_token_last_used(
        &self,
        token_id: &str,
        ip_address: Option<&str>,
    ) -> AppResult<()>;
    /// Record admin token usage for audit trail
    async fn record_token_usage(&self, usage: &AdminTokenUsage) -> AppResult<()>;
    /// Get admin token usage history
    async fn get_token_usage_history(
        &self,
        token_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<Vec<AdminTokenUsage>>;
    /// Record API key provisioning by admin
    async fn record_provisioned_key(
        &self,
        admin_token_id: &str,
        api_key_id: &str,
        user_email: &str,
        tier: &str,
        rate_limit_requests: u32,
        rate_limit_period: &str,
    ) -> AppResult<()>;
    /// Get admin provisioned keys history
    async fn get_provisioned_keys(
        &self,
        admin_token_id: Option<&str>,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<Vec<Value>>;
}
