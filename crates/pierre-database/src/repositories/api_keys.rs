// ABOUTME: Repository trait definitions for the API keys and user MCP tokens domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;

use pierre_core::models::{
    ApiKey, CreateUserMcpTokenRequest, UserMcpToken, UserMcpTokenCreated, UserMcpTokenInfo,
};
use uuid::Uuid;

/// API key management repository
#[async_trait]
pub trait ApiKeyRepository: Send + Sync {
    /// Create a new API key
    async fn create(&self, api_key: &ApiKey) -> AppResult<()>;
    /// Get API key by its prefix and hash
    async fn get_by_prefix(&self, prefix: &str, hash: &str) -> AppResult<Option<ApiKey>>;
    /// Get all API keys for a user
    async fn get_for_user(&self, user_id: Uuid) -> AppResult<Vec<ApiKey>>;
    /// Update API key last used timestamp
    async fn update_last_used(&self, api_key_id: &str) -> AppResult<()>;
    /// Deactivate an API key
    async fn deactivate(&self, api_key_id: &str, user_id: Uuid) -> AppResult<()>;
    /// Get API key by ID, optionally scoped to a specific user for ownership enforcement
    async fn get_by_id(&self, api_key_id: &str, user_id: Option<Uuid>)
        -> AppResult<Option<ApiKey>>;
    /// Get API keys with optional filters
    async fn get_filtered(
        &self,
        user_email: Option<&str>,
        active_only: bool,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> AppResult<Vec<ApiKey>>;
    /// Clean up expired API keys
    async fn cleanup_expired(&self) -> AppResult<u64>;
    /// Get expired API keys
    async fn get_expired(&self) -> AppResult<Vec<ApiKey>>;
}

/// User MCP token management repository
#[async_trait]
pub trait UserMcpTokenRepository: Send + Sync {
    /// Create a new user MCP token for AI client authentication
    async fn create_token(
        &self,
        user_id: Uuid,
        request: &CreateUserMcpTokenRequest,
    ) -> AppResult<UserMcpTokenCreated>;
    /// Validate a user MCP token and return the associated user ID
    async fn validate_token(&self, token_value: &str) -> AppResult<Uuid>;
    /// List all MCP tokens for a user
    async fn list_tokens(&self, user_id: Uuid) -> AppResult<Vec<UserMcpTokenInfo>>;
    /// Revoke a user MCP token
    async fn revoke_token(&self, token_id: &str, user_id: Uuid) -> AppResult<()>;
    /// Get a user MCP token by ID
    async fn get_token(&self, token_id: &str, user_id: Uuid) -> AppResult<Option<UserMcpToken>>;
    /// Cleanup expired user MCP tokens (mark as revoked)
    async fn cleanup_expired_tokens(&self) -> AppResult<u64>;
}
