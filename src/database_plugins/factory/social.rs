// ABOUTME: Social insight repository dispatch for the database factory
// ABOUTME: Delegates InsightRepository calls to SQLite or PostgreSQL backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::database_plugins::InsightRepository;
use crate::errors::AppResult;
use async_trait::async_trait;

#[async_trait]
impl InsightRepository for Database {
    async fn store(
        &self,
        user_id: uuid::Uuid,
        insight_data: serde_json::Value,
    ) -> AppResult<String> {
        match self {
            Self::SQLite(db) => InsightRepository::store(db, user_id, insight_data).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => InsightRepository::store(db, user_id, insight_data).await,
        }
    }
    async fn get_for_user(
        &self,
        user_id: uuid::Uuid,
        insight_type: Option<&str>,
        limit: Option<u32>,
    ) -> AppResult<Vec<serde_json::Value>> {
        match self {
            Self::SQLite(db) => {
                InsightRepository::get_for_user(db, user_id, insight_type, limit).await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                InsightRepository::get_for_user(db, user_id, insight_type, limit).await
            }
        }
    }
}
