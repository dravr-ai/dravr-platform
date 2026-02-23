// ABOUTME: API key repository dispatch for the database factory
// ABOUTME: Delegates ApiKeyRepository calls to SQLite or PostgreSQL backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::api_keys::ApiKey;
use crate::database_plugins::ApiKeyRepository;
use crate::errors::AppResult;
use async_trait::async_trait;
use uuid::Uuid;

#[async_trait]
impl ApiKeyRepository for Database {
    async fn create(&self, api_key: &ApiKey) -> AppResult<()> {
        match self {
            Self::SQLite(db) => ApiKeyRepository::create(db, api_key).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => ApiKeyRepository::create(db, api_key).await,
        }
    }
    async fn get_by_prefix(&self, prefix: &str, hash: &str) -> AppResult<Option<ApiKey>> {
        match self {
            Self::SQLite(db) => db.get_by_prefix(prefix, hash).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_by_prefix(prefix, hash).await,
        }
    }
    async fn get_for_user(&self, user_id: uuid::Uuid) -> AppResult<Vec<ApiKey>> {
        match self {
            Self::SQLite(db) => ApiKeyRepository::get_for_user(db, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => ApiKeyRepository::get_for_user(db, user_id).await,
        }
    }
    async fn update_last_used(&self, api_key_id: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.update_last_used(api_key_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_last_used(api_key_id).await,
        }
    }
    async fn deactivate(&self, api_key_id: &str, user_id: uuid::Uuid) -> AppResult<()> {
        match self {
            Self::SQLite(db) => ApiKeyRepository::deactivate(db, api_key_id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => ApiKeyRepository::deactivate(db, api_key_id, user_id).await,
        }
    }
    async fn get_by_id(
        &self,
        api_key_id: &str,
        user_id: Option<Uuid>,
    ) -> AppResult<Option<ApiKey>> {
        match self {
            Self::SQLite(db) => ApiKeyRepository::get_by_id(db, api_key_id, user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => ApiKeyRepository::get_by_id(db, api_key_id, user_id).await,
        }
    }
    async fn get_filtered(
        &self,
        user_email: Option<&str>,
        active_only: bool,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> AppResult<Vec<ApiKey>> {
        match self {
            Self::SQLite(db) => {
                ApiKeyRepository::get_filtered(db, user_email, active_only, limit, offset).await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.get_filtered(user_email, active_only, limit, offset)
                    .await
            }
        }
    }
    async fn cleanup_expired(&self) -> AppResult<u64> {
        match self {
            Self::SQLite(db) => db.cleanup_expired().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.cleanup_expired().await,
        }
    }
    async fn get_expired(&self) -> AppResult<Vec<ApiKey>> {
        match self {
            Self::SQLite(db) => db.get_expired().await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_expired().await,
        }
    }
}
