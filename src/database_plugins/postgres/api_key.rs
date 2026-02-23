// ABOUTME: PostgreSQL API key repository implementation
// ABOUTME: Handles API key creation, validation, usage tracking, and lifecycle management
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::super::ApiKeyRepository;
use super::PostgresDatabase;
use crate::api_keys::{ApiKey, ApiKeyTier};
use crate::constants::tiers;
use crate::errors::{AppError, AppResult};
use async_trait::async_trait;
use sqlx::Row;
use std::fmt::Write;
use uuid::Uuid;

#[async_trait]
impl ApiKeyRepository for PostgresDatabase {
    async fn create(&self, api_key: &ApiKey) -> AppResult<()> {
        sqlx::query(
            r"
            INSERT INTO api_keys (id, user_id, name, key_prefix, key_hash, description, tier, is_active, rate_limit_requests, rate_limit_window_seconds, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            ",
        )
        .bind(&api_key.id)
        .bind(api_key.user_id)
        .bind(&api_key.name)
        .bind(&api_key.key_prefix)
        .bind(&api_key.key_hash)
        .bind(&api_key.description)
        .bind(format!("{:?}", api_key.tier).to_lowercase())
        .bind(api_key.is_active)
        .bind(i32::try_from(api_key.rate_limit_requests).unwrap_or(i32::MAX))
        .bind(i32::try_from(api_key.rate_limit_window_seconds).unwrap_or(i32::MAX))
        .bind(api_key.expires_at)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to create API key: {e}")))?;

        Ok(())
    }

    async fn get_by_prefix(&self, prefix: &str, hash: &str) -> AppResult<Option<ApiKey>> {
        let row = sqlx::query(
            r"
            SELECT id, user_id, name, key_prefix, key_hash, description, tier, is_active, rate_limit_requests,
                   rate_limit_window_seconds, created_at, expires_at, last_used_at, updated_at
            FROM api_keys
            WHERE id LIKE $1 AND key_hash = $2 AND is_active = true
            ",
        )
        .bind(format!("{prefix}%"))
        .bind(hash)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get API key by prefix: {e}")))?;

        row.map_or_else(
            || Ok(None),
            |row| {
                Ok(Some(ApiKey {
                    id: row.get("id"),
                    user_id: row.get("user_id"),
                    name: row.get("name"),
                    key_prefix: row.get("key_prefix"),
                    key_hash: row.get("key_hash"),
                    description: row.get("description"),
                    tier: match row.get::<String, _>("tier").to_lowercase().as_str() {
                        tiers::TRIAL | tiers::STARTER => ApiKeyTier::Starter,
                        tiers::PROFESSIONAL => ApiKeyTier::Professional,
                        tiers::ENTERPRISE => ApiKeyTier::Enterprise,
                        _ => ApiKeyTier::Trial,
                    },
                    is_active: row.get("is_active"),
                    rate_limit_requests: u32::try_from(
                        row.get::<i32, _>("rate_limit_requests").max(0),
                    )
                    .unwrap_or(0),
                    rate_limit_window_seconds: u32::try_from(
                        row.get::<i32, _>("rate_limit_window_seconds").max(0),
                    )
                    .unwrap_or(0),
                    created_at: row.get("created_at"),
                    expires_at: row.get("expires_at"),
                    last_used_at: row.get("last_used_at"),
                }))
            },
        )
    }

    // Remaining database methods follow the same PostgreSQL implementation pattern

    async fn get_for_user(&self, user_id: Uuid) -> AppResult<Vec<ApiKey>> {
        let rows = sqlx::query(
            r"
            SELECT id, user_id, name, key_prefix, key_hash, description, tier, is_active, rate_limit_requests,
                   rate_limit_window_seconds, created_at, expires_at, last_used_at, updated_at
            FROM api_keys
            WHERE user_id = $1
            ORDER BY created_at DESC
            ",
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get user API keys: {e}")))?;

        Ok(rows
            .into_iter()
            .map(|row| ApiKey {
                id: row.get("id"),
                user_id: row.get("user_id"),
                name: row.get("name"),
                key_prefix: row.get("key_prefix"),
                key_hash: row.get("key_hash"),
                description: row.get("description"),
                tier: match row.get::<String, _>("tier").to_lowercase().as_str() {
                    tiers::TRIAL | tiers::STARTER => ApiKeyTier::Starter,
                    tiers::PROFESSIONAL => ApiKeyTier::Professional,
                    tiers::ENTERPRISE => ApiKeyTier::Enterprise,
                    _ => ApiKeyTier::Trial,
                },
                is_active: row.get("is_active"),
                rate_limit_requests: u32::try_from(row.get::<i32, _>("rate_limit_requests").max(0))
                    .unwrap_or(0),
                rate_limit_window_seconds: u32::try_from(
                    row.get::<i32, _>("rate_limit_window_seconds").max(0),
                )
                .unwrap_or(0),
                created_at: row.get("created_at"),
                expires_at: row.get("expires_at"),
                last_used_at: row.get("last_used_at"),
            })
            .collect())
    }

    async fn update_last_used(&self, api_key_id: &str) -> AppResult<()> {
        sqlx::query(
            r"
            UPDATE api_keys
            SET last_used_at = CURRENT_TIMESTAMP
            WHERE id = $1
            ",
        )
        .bind(api_key_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to update API key last used: {e}")))?;

        Ok(())
    }

    async fn deactivate(&self, api_key_id: &str, user_id: Uuid) -> AppResult<()> {
        sqlx::query(
            r"
            UPDATE api_keys
            SET is_active = false
            WHERE id = $1 AND user_id = $2
            ",
        )
        .bind(api_key_id)
        .bind(user_id)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to deactivate API key: {e}")))?;

        Ok(())
    }

    async fn get_by_id(
        &self,
        api_key_id: &str,
        user_id: Option<Uuid>,
    ) -> AppResult<Option<ApiKey>> {
        let row = if let Some(uid) = user_id {
            sqlx::query(
                r"
                SELECT id, user_id, name, description, key_prefix, key_hash, tier,
                       rate_limit_requests, rate_limit_window_seconds, is_active,
                       created_at, last_used_at, expires_at, updated_at
                FROM api_keys
                WHERE id = $1 AND user_id = $2
                ",
            )
            .bind(api_key_id)
            .bind(uid)
            .fetch_optional(&self.pool)
            .await
        } else {
            // Admin callers that legitimately need cross-user access pass None
            sqlx::query(
                r"
                SELECT id, user_id, name, description, key_prefix, key_hash, tier,
                       rate_limit_requests, rate_limit_window_seconds, is_active,
                       created_at, last_used_at, expires_at, updated_at
                FROM api_keys
                WHERE id = $1
                ",
            )
            .bind(api_key_id)
            .fetch_optional(&self.pool)
            .await
        }
        .map_err(|e| AppError::database(format!("Failed to get API key by ID: {e}")))?;

        row.map_or_else(
            || Ok(None),
            |row| {
                use sqlx::Row;
                let tier_str: String = row.get("tier");
                let tier = match tier_str.as_str() {
                    tiers::STARTER => ApiKeyTier::Starter,
                    tiers::PROFESSIONAL => ApiKeyTier::Professional,
                    tiers::ENTERPRISE => ApiKeyTier::Enterprise,
                    _ => ApiKeyTier::Trial, // Default to trial for unknown values (including "trial")
                };

                Ok(Some(ApiKey {
                    id: row.get("id"),
                    user_id: row.get("user_id"),
                    name: row.get("name"),
                    key_prefix: row.get("key_prefix"),
                    description: row.get("description"),
                    key_hash: row.get("key_hash"),
                    tier,
                    rate_limit_requests: u32::try_from(
                        row.get::<i32, _>("rate_limit_requests").max(0),
                    )
                    .unwrap_or(0),
                    rate_limit_window_seconds: u32::try_from(
                        row.get::<i32, _>("rate_limit_window_seconds").max(0),
                    )
                    .unwrap_or(0),
                    is_active: row.get("is_active"),
                    created_at: row.get("created_at"),
                    last_used_at: row.get("last_used_at"),
                    expires_at: row.get("expires_at"),
                }))
            },
        )
    }

    async fn get_filtered(
        &self,
        user_email: Option<&str>,
        active_only: bool,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> AppResult<Vec<ApiKey>> {
        let mut query: String = "SELECT ak.id, ak.user_id, ak.name, ak.description, ak.key_prefix, ak.key_hash, ak.tier, ak.rate_limit_requests, ak.rate_limit_window_seconds, ak.is_active, ak.created_at, ak.last_used_at, ak.expires_at, ak.updated_at FROM api_keys ak".into();

        let mut conditions = Vec::new();
        let mut param_count = 0;

        if user_email.is_some() {
            query.push_str(" JOIN users u ON ak.user_id = u.id");
            param_count += 1;
            conditions.push(format!("u.email = ${param_count}"));
        }

        if active_only {
            conditions.push("ak.is_active = true".into());
        }

        if !conditions.is_empty() {
            query.push_str(" WHERE ");
            query.push_str(&conditions.join(" AND "));
        }

        query.push_str(" ORDER BY ak.created_at DESC");

        if let Some(_limit) = limit {
            param_count += 1;
            write!(&mut query, " LIMIT ${param_count}")
                .map_err(|e| AppError::database(format!("Failed to write LIMIT clause: {e}")))?;
            if let Some(_offset) = offset {
                param_count += 1;
                write!(&mut query, " OFFSET ${param_count}").map_err(|e| {
                    AppError::database(format!("Failed to write OFFSET clause: {e}"))
                })?;
            }
        }

        let mut sqlx_query = sqlx::query(&query);

        if let Some(email) = user_email {
            sqlx_query = sqlx_query.bind(email);
        }

        if let Some(limit) = limit {
            sqlx_query = sqlx_query.bind(limit);
            if let Some(offset) = offset {
                sqlx_query = sqlx_query.bind(offset);
            }
        }

        let rows = sqlx_query
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to list API keys: {e}")))?;

        let mut api_keys = Vec::with_capacity(rows.len());
        for row in rows {
            let tier_str: String = row.get("tier");
            let tier = match tier_str.as_str() {
                tiers::STARTER => ApiKeyTier::Starter,
                tiers::PROFESSIONAL => ApiKeyTier::Professional,
                tiers::ENTERPRISE => ApiKeyTier::Enterprise,
                _ => ApiKeyTier::Trial, // Default to trial for unknown values (including "trial")
            };

            api_keys.push(ApiKey {
                id: row.get("id"),
                user_id: row.get("user_id"),
                name: row.get("name"),
                key_prefix: row.get("key_prefix"),
                description: row.get("description"),
                key_hash: row.get("key_hash"),
                tier,
                rate_limit_requests: u32::try_from(row.get::<i32, _>("rate_limit_requests").max(0))
                    .unwrap_or(0),
                rate_limit_window_seconds: u32::try_from(
                    row.get::<i32, _>("rate_limit_window_seconds").max(0),
                )
                .unwrap_or(0),
                is_active: row.get("is_active"),
                created_at: row.get("created_at"),
                last_used_at: row.get("last_used_at"),
                expires_at: row.get("expires_at"),
            });
        }

        Ok(api_keys)
    }

    async fn cleanup_expired(&self) -> AppResult<u64> {
        let result = sqlx::query(
            r"
            UPDATE api_keys
            SET is_active = false
            WHERE expires_at IS NOT NULL AND expires_at < CURRENT_TIMESTAMP AND is_active = true
            ",
        )
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to cleanup expired API keys: {e}")))?;

        Ok(result.rows_affected())
    }

    async fn get_expired(&self) -> AppResult<Vec<ApiKey>> {
        let rows = sqlx::query(
            r"
            SELECT id, user_id, name, key_prefix, key_hash, description, tier, is_active, rate_limit_requests,
                   rate_limit_window_seconds, created_at, expires_at, last_used_at, updated_at
            FROM api_keys
            WHERE expires_at IS NOT NULL AND expires_at < CURRENT_TIMESTAMP
            ORDER BY expires_at ASC
            ",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to get expired API keys: {e}")))?;

        Ok(rows
            .into_iter()
            .map(|row| ApiKey {
                id: row.get("id"),
                user_id: row.get("user_id"),
                name: row.get("name"),
                key_prefix: row.get("key_prefix"),
                key_hash: row.get("key_hash"),
                description: row.get("description"),
                tier: match row.get::<String, _>("tier").to_lowercase().as_str() {
                    tiers::TRIAL | tiers::STARTER => ApiKeyTier::Starter,
                    tiers::PROFESSIONAL => ApiKeyTier::Professional,
                    tiers::ENTERPRISE => ApiKeyTier::Enterprise,
                    _ => ApiKeyTier::Trial,
                },
                is_active: row.get("is_active"),
                rate_limit_requests: u32::try_from(row.get::<i32, _>("rate_limit_requests").max(0))
                    .unwrap_or(0),
                rate_limit_window_seconds: u32::try_from(
                    row.get::<i32, _>("rate_limit_window_seconds").max(0),
                )
                .unwrap_or(0),
                created_at: row.get("created_at"),
                expires_at: row.get("expires_at"),
                last_used_at: row.get("last_used_at"),
            })
            .collect())
    }
}
