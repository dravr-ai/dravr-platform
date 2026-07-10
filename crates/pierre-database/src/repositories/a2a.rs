// ABOUTME: Repository trait definitions for the agent-to-agent (A2A) protocol persistence domain
// ABOUTME: Split out of repositories.rs as part of Finding B (per-domain repository modules)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::a2a::{
    A2AClient, A2APushNotificationConfig, A2ASession, A2ATask, A2AUsage, A2AUsageStats, TaskStatus,
};

use serde_json::Value;
use uuid::Uuid;

/// A2A (Agent-to-Agent) client and session management repository
#[async_trait]
pub trait A2ARepository: Send + Sync {
    /// Create a new A2A client
    async fn create_client(
        &self,
        client: &A2AClient,
        client_secret: &str,
        api_key_id: &str,
    ) -> AppResult<String>;
    /// Get A2A client by ID
    async fn get_client(&self, client_id: &str) -> AppResult<Option<A2AClient>>;
    /// Get A2A client by API key ID
    async fn get_client_by_api_key_id(&self, api_key_id: &str) -> AppResult<Option<A2AClient>>;
    /// Get A2A client by name
    async fn get_client_by_name(&self, name: &str) -> AppResult<Option<A2AClient>>;
    /// List all A2A clients for a user
    async fn list_clients(&self, user_id: &Uuid) -> AppResult<Vec<A2AClient>>;
    /// Deactivate an A2A client
    async fn deactivate_client(&self, client_id: &str) -> AppResult<()>;
    /// Get client credentials for authentication
    async fn get_client_credentials(&self, client_id: &str) -> AppResult<Option<(String, String)>>;
    /// Invalidate all active sessions for a client
    async fn invalidate_client_sessions(&self, client_id: &str) -> AppResult<()>;
    /// Deactivate all API keys associated with a client
    async fn deactivate_client_api_keys(&self, client_id: &str) -> AppResult<()>;
    /// Create a new A2A session
    async fn create_session(
        &self,
        client_id: &str,
        user_id: Option<&Uuid>,
        granted_scopes: &[String],
        expires_in_hours: i64,
    ) -> AppResult<String>;
    /// Get A2A session by token
    async fn get_session(&self, session_token: &str) -> AppResult<Option<A2ASession>>;
    /// Update A2A session activity timestamp
    async fn update_session_activity(&self, session_token: &str) -> AppResult<()>;
    /// Get active sessions for a specific client
    async fn get_active_sessions(&self, client_id: &str) -> AppResult<Vec<A2ASession>>;
    /// Create a new A2A task
    async fn create_task(
        &self,
        client_id: &str,
        session_id: Option<&str>,
        task_type: &str,
        input_data: &Value,
        context_id: Option<&str>,
    ) -> AppResult<String>;
    /// Get A2A task by ID
    async fn get_task(&self, task_id: &str) -> AppResult<Option<A2ATask>>;
    /// List A2A tasks for a client with optional filtering, ordered by
    /// status timestamp (`updated_at`) descending per A2A 1.0 `ListTasks`
    async fn list_tasks(
        &self,
        client_id: Option<&str>,
        status_filter: Option<&TaskStatus>,
        context_id: Option<&str>,
        updated_after: Option<DateTime<Utc>>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> AppResult<Vec<A2ATask>>;
    /// Update A2A task status, result, and the wire `TaskStatus.message` JSON
    async fn update_task_status(
        &self,
        task_id: &str,
        status: &TaskStatus,
        result: Option<&Value>,
        status_message: Option<&Value>,
    ) -> AppResult<()>;
    /// Replace the task's wire `history` (`Message[]`) and `artifacts`
    /// (`Artifact[]`) JSON documents
    async fn update_task_wire_state(
        &self,
        task_id: &str,
        history: Option<&Value>,
        artifacts: Option<&Value>,
    ) -> AppResult<()>;
    /// Store a push notification configuration for a task (A2A 1.0
    /// `CreateTaskPushNotificationConfig`)
    async fn create_push_config(&self, config: &A2APushNotificationConfig) -> AppResult<()>;
    /// Get one push notification configuration of a task
    async fn get_push_config(
        &self,
        task_id: &str,
        config_id: &str,
    ) -> AppResult<Option<A2APushNotificationConfig>>;
    /// List all push notification configurations of a task
    async fn list_push_configs(&self, task_id: &str) -> AppResult<Vec<A2APushNotificationConfig>>;
    /// Delete a push notification configuration; returns whether a row existed
    async fn delete_push_config(&self, task_id: &str, config_id: &str) -> AppResult<bool>;
    /// Record A2A usage for analytics
    async fn record_usage(&self, usage: &A2AUsage) -> AppResult<()>;
    /// Get current A2A usage count for a client
    async fn get_client_current_usage(&self, client_id: &str) -> AppResult<u32>;
    /// Get A2A usage statistics for a client
    async fn get_usage_stats(
        &self,
        client_id: &str,
        start_date: DateTime<Utc>,
        end_date: DateTime<Utc>,
    ) -> AppResult<A2AUsageStats>;
    /// Get A2A client usage history
    async fn get_client_usage_history(
        &self,
        client_id: &str,
        days: u32,
    ) -> AppResult<Vec<(DateTime<Utc>, u32, u32)>>;
}
