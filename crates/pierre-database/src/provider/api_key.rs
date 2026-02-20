// ABOUTME: API key database operations trait covering key CRUD, usage tracking, and JWT usage
// ABOUTME: Enables ApiKeyRepository blanket impl with focused trait bound
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::ApiKey;
use uuid::Uuid;

/// API key CRUD and lifecycle database operations
#[async_trait]
pub trait ApiKeyDbOps: Send + Sync + Clone {
    /// Create a new API key
    async fn create_api_key(&self, api_key: &ApiKey) -> AppResult<()>;

    /// Get API key by its prefix and hash
    async fn get_api_key_by_prefix(&self, prefix: &str, hash: &str) -> AppResult<Option<ApiKey>>;

    /// Get all API keys for a user
    async fn get_user_api_keys(&self, user_id: Uuid) -> AppResult<Vec<ApiKey>>;

    /// Update API key last used timestamp
    async fn update_api_key_last_used(&self, api_key_id: &str) -> AppResult<()>;

    /// Deactivate an API key
    async fn deactivate_api_key(&self, api_key_id: &str, user_id: Uuid) -> AppResult<()>;

    /// Get API key by ID, optionally scoped to a specific user for ownership enforcement
    async fn get_api_key_by_id(
        &self,
        api_key_id: &str,
        user_id: Option<Uuid>,
    ) -> AppResult<Option<ApiKey>>;

    /// Get API keys with optional filters
    async fn get_api_keys_filtered(
        &self,
        user_email: Option<&str>,
        active_only: bool,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> AppResult<Vec<ApiKey>>;

    /// Clean up expired API keys
    async fn cleanup_expired_api_keys(&self) -> AppResult<u64>;

    /// Get expired API keys
    async fn get_expired_api_keys(&self) -> AppResult<Vec<ApiKey>>;
}
