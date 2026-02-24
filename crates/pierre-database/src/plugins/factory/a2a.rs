// ABOUTME: A2A repository dispatch for the database factory
// ABOUTME: Delegates A2ARepository calls to SQLite or PostgreSQL backends
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::Database;
use crate::database::{A2AUsage, A2AUsageStats};
use crate::plugins::A2ARepository;
use async_trait::async_trait;
use pierre_core::errors::AppResult;
use pierre_core::models::a2a::{A2AClient, A2ASession, A2ATask, TaskStatus};

#[async_trait]
impl A2ARepository for Database {
    async fn create_client(
        &self,
        client: &A2AClient,
        client_secret: &str,
        api_key_id: &str,
    ) -> AppResult<String> {
        match self {
            Self::SQLite(db) => db.create_client(client, client_secret, api_key_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.create_client(client, client_secret, api_key_id).await,
        }
    }
    async fn get_client(&self, client_id: &str) -> AppResult<Option<A2AClient>> {
        match self {
            Self::SQLite(db) => A2ARepository::get_client(db, client_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => A2ARepository::get_client(db, client_id).await,
        }
    }
    async fn get_client_by_api_key_id(&self, api_key_id: &str) -> AppResult<Option<A2AClient>> {
        match self {
            Self::SQLite(db) => db.get_client_by_api_key_id(api_key_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_client_by_api_key_id(api_key_id).await,
        }
    }
    async fn get_client_by_name(&self, name: &str) -> AppResult<Option<A2AClient>> {
        match self {
            Self::SQLite(db) => db.get_client_by_name(name).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_client_by_name(name).await,
        }
    }
    async fn list_clients(&self, user_id: &uuid::Uuid) -> AppResult<Vec<A2AClient>> {
        match self {
            Self::SQLite(db) => db.list_clients(user_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_clients(user_id).await,
        }
    }
    async fn deactivate_client(&self, client_id: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.deactivate_client(client_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.deactivate_client(client_id).await,
        }
    }
    async fn get_client_credentials(&self, client_id: &str) -> AppResult<Option<(String, String)>> {
        match self {
            Self::SQLite(db) => db.get_client_credentials(client_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_client_credentials(client_id).await,
        }
    }
    async fn invalidate_client_sessions(&self, client_id: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.invalidate_client_sessions(client_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.invalidate_client_sessions(client_id).await,
        }
    }
    async fn deactivate_client_api_keys(&self, client_id: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.deactivate_client_api_keys(client_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.deactivate_client_api_keys(client_id).await,
        }
    }
    async fn create_session(
        &self,
        client_id: &str,
        user_id: Option<&uuid::Uuid>,
        granted_scopes: &[String],
        expires_in_hours: i64,
    ) -> AppResult<String> {
        match self {
            Self::SQLite(db) => {
                A2ARepository::create_session(
                    db,
                    client_id,
                    user_id,
                    granted_scopes,
                    expires_in_hours,
                )
                .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                A2ARepository::create_session(
                    db,
                    client_id,
                    user_id,
                    granted_scopes,
                    expires_in_hours,
                )
                .await
            }
        }
    }
    async fn get_session(&self, session_token: &str) -> AppResult<Option<A2ASession>> {
        match self {
            Self::SQLite(db) => A2ARepository::get_session(db, session_token).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => A2ARepository::get_session(db, session_token).await,
        }
    }
    async fn update_session_activity(&self, session_token: &str) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.update_session_activity(session_token).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_session_activity(session_token).await,
        }
    }
    async fn get_active_sessions(&self, client_id: &str) -> AppResult<Vec<A2ASession>> {
        match self {
            Self::SQLite(db) => A2ARepository::get_active_sessions(db, client_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => A2ARepository::get_active_sessions(db, client_id).await,
        }
    }
    async fn create_task(
        &self,
        client_id: &str,
        session_id: Option<&str>,
        task_type: &str,
        input_data: &serde_json::Value,
    ) -> AppResult<String> {
        match self {
            Self::SQLite(db) => {
                db.create_task(client_id, session_id, task_type, input_data)
                    .await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                db.create_task(client_id, session_id, task_type, input_data)
                    .await
            }
        }
    }
    async fn get_task(&self, task_id: &str) -> AppResult<Option<A2ATask>> {
        match self {
            Self::SQLite(db) => db.get_task(task_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_task(task_id).await,
        }
    }
    async fn list_tasks(
        &self,
        client_id: Option<&str>,
        status_filter: Option<&TaskStatus>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<A2ATask>> {
        match self {
            Self::SQLite(db) => db.list_tasks(client_id, status_filter, limit, offset).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.list_tasks(client_id, status_filter, limit, offset).await,
        }
    }
    async fn update_task_status(
        &self,
        task_id: &str,
        status: &TaskStatus,
        result: Option<&serde_json::Value>,
        error: Option<&str>,
    ) -> AppResult<()> {
        match self {
            Self::SQLite(db) => db.update_task_status(task_id, status, result, error).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.update_task_status(task_id, status, result, error).await,
        }
    }
    async fn record_usage(&self, usage: &A2AUsage) -> AppResult<()> {
        match self {
            Self::SQLite(db) => A2ARepository::record_usage(db, usage).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => A2ARepository::record_usage(db, usage).await,
        }
    }
    async fn get_client_current_usage(&self, client_id: &str) -> AppResult<u32> {
        match self {
            Self::SQLite(db) => db.get_client_current_usage(client_id).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_client_current_usage(client_id).await,
        }
    }
    async fn get_usage_stats(
        &self,
        client_id: &str,
        start_date: chrono::DateTime<chrono::Utc>,
        end_date: chrono::DateTime<chrono::Utc>,
    ) -> AppResult<A2AUsageStats> {
        match self {
            Self::SQLite(db) => {
                A2ARepository::get_usage_stats(db, client_id, start_date, end_date).await
            }
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => {
                A2ARepository::get_usage_stats(db, client_id, start_date, end_date).await
            }
        }
    }
    async fn get_client_usage_history(
        &self,
        client_id: &str,
        days: u32,
    ) -> AppResult<Vec<(chrono::DateTime<chrono::Utc>, u32, u32)>> {
        match self {
            Self::SQLite(db) => db.get_client_usage_history(client_id, days).await,
            #[cfg(feature = "postgresql")]
            Self::PostgreSQL(db) => db.get_client_usage_history(client_id, days).await,
        }
    }
}
