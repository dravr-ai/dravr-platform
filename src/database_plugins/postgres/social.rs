// ABOUTME: PostgreSQL social insight repository implementation
// ABOUTME: Manages social insight data storage and retrieval for community features
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::super::InsightRepository;
use super::PostgresDatabase;
use crate::errors::{AppError, AppResult};
use async_trait::async_trait;
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

#[async_trait]
impl InsightRepository for PostgresDatabase {
    async fn store(&self, user_id: Uuid, insight_data: Value) -> AppResult<String> {
        let insight_id = Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        let insight_json = serde_json::to_string(&insight_data)
            .map_err(|e| AppError::database(format!("Failed to serialize insight: {e}")))?;

        sqlx::query(
            r"
            INSERT INTO insights (id, user_id, insight_type, insight_data, created_at)
            VALUES ($1, $2, $3, $4, $5)
            ",
        )
        .bind(&insight_id)
        .bind(user_id)
        .bind("general") // Default insight type since it's not provided separately
        .bind(&insight_json)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::database(format!("Failed to store insight: {e}")))?;

        Ok(insight_id)
    }

    async fn get_for_user(
        &self,
        user_id: Uuid,
        insight_type: Option<&str>,
        limit: Option<u32>,
    ) -> AppResult<Vec<Value>> {
        let limit = limit.unwrap_or(50);

        let rows = if let Some(insight_type) = insight_type {
            sqlx::query(
                r"
                SELECT content
                FROM insights
                WHERE user_id = $1 AND insight_type = $2
                ORDER BY created_at DESC
                LIMIT $3
                ",
            )
            .bind(user_id)
            .bind(insight_type)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user insights by type: {e}")))?
        } else {
            sqlx::query(
                r"
                SELECT content
                FROM insights
                WHERE user_id = $1
                ORDER BY created_at DESC
                LIMIT $2
                ",
            )
            .bind(user_id)
            .bind(i64::from(limit))
            .fetch_all(&self.pool)
            .await
            .map_err(|e| AppError::database(format!("Failed to get user insights: {e}")))?
        };

        Ok(rows.into_iter().map(|row| row.get("content")).collect())
    }
}
