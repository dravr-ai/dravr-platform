// ABOUTME: Demonstrates A2A 1.0 task lifecycle management with status polling
// ABOUTME: Shows SendMessage (returnImmediately), GetTask monitoring, and ListTasks pagination
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Task Lifecycle Management Example (A2A 1.0)
//!
//! This example demonstrates the A2A 1.0 task management surface:
//! 1. Submitting work via `SendMessage` with `returnImmediately: true`
//! 2. Monitoring task status with `GetTask` polling
//! 3. Handling task state transitions (submitted → working → terminal)
//! 4. Retrieving task artifacts
//! 5. Listing tasks with `ListTasks` (cursor pagination)
//!
//! ## A2A 1.0 Task Lifecycle
//! ```text
//! ┌───────────┐
//! │ SUBMITTED │  Task received, awaiting execution
//! └─────┬─────┘
//!       │
//!       v
//! ┌───────────┐
//! │  WORKING  │  Task is actively being processed
//! └─────┬─────┘
//!       │
//!       ├───────────┬───────────┐
//!       v           v           v
//! ┌───────────┐ ┌────────┐ ┌──────────┐
//! │ COMPLETED │ │ FAILED │ │ CANCELED │  Terminal states
//! └───────────┘ └────────┘ └──────────┘
//! ```

use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::time::Duration;
use tracing::{info, warn};
use uuid::Uuid;

/// A2A 1.0 task states (ProtoJSON wire strings)
#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Clone, Copy)]
enum TaskState {
    #[serde(rename = "TASK_STATE_SUBMITTED")]
    Submitted,
    #[serde(rename = "TASK_STATE_WORKING")]
    Working,
    #[serde(rename = "TASK_STATE_COMPLETED")]
    Completed,
    #[serde(rename = "TASK_STATE_FAILED")]
    Failed,
    #[serde(rename = "TASK_STATE_CANCELED")]
    Canceled,
    #[serde(rename = "TASK_STATE_INPUT_REQUIRED")]
    InputRequired,
    #[serde(rename = "TASK_STATE_REJECTED")]
    Rejected,
    #[serde(rename = "TASK_STATE_AUTH_REQUIRED")]
    AuthRequired,
}

/// A2A 1.0 task status object
#[derive(Debug, Deserialize, Serialize)]
struct TaskStatus {
    state: TaskState,
    #[serde(skip_serializing_if = "Option::is_none")]
    timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<Value>,
}

/// A2A 1.0 wire task
#[derive(Debug, Deserialize, Serialize)]
struct Task {
    id: String,
    status: TaskStatus,
    #[serde(rename = "contextId", skip_serializing_if = "Option::is_none")]
    context_id: Option<String>,
    #[serde(default)]
    artifacts: Vec<Value>,
    #[serde(default)]
    history: Vec<Value>,
}

/// Task Manager Client
struct TaskManager {
    http_client: Client,
    server_url: String,
    access_token: Option<String>,
    client_id: String,
}

impl TaskManager {
    /// Create a new task manager
    fn new(server_url: String, client_id: String) -> Self {
        let http_client = Client::builder()
            .timeout(Duration::from_secs(30))
            .user_agent("TaskManagerExample/1.0")
            .build()
            .expect("Failed to create HTTP client");

        Self {
            http_client,
            server_url,
            access_token: None,
            client_id,
        }
    }

    /// Authenticate with A2A client credentials
    async fn authenticate(&mut self, client_secret: &str) -> Result<()> {
        info!("🔐 Authenticating with A2A protocol");

        let auth_payload = json!({
            "client_id": self.client_id,
            "client_secret": client_secret,
            "grant_type": "client_credentials",
            "scopes": ["read", "write", "tasks"]
        });

        let response = self
            .http_client
            .post(format!("{}/a2a/auth", self.server_url))
            .header("Content-Type", "application/json")
            .json(&auth_payload)
            .send()
            .await
            .context("Failed to authenticate")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            anyhow::bail!("Authentication failed: {error_text}");
        }

        let auth_response: Value = response.json().await?;
        self.access_token = Some(
            auth_response
                .get("session_token")
                .or_else(|| auth_response.get("access_token"))
                .and_then(|t| t.as_str())
                .context("No access token in response")?
                .to_string(),
        );

        info!("✅ Authentication successful");
        Ok(())
    }

    /// POST one A2A 1.0 JSON-RPC request (PascalCase method) and return the result.
    async fn call(&self, method: &str, params: Value) -> Result<Value> {
        let request = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": Uuid::new_v4().to_string()
        });

        let response = self
            .http_client
            .post(format!("{}/a2a/jsonrpc", self.server_url))
            .header("Content-Type", "application/json")
            // Spec §3.6: every request negotiates the protocol version.
            .header("A2A-Version", "1.0")
            .header(
                "Authorization",
                format!("Bearer {}", self.access_token.as_ref().context("Not authenticated")?),
            )
            .json(&request)
            .send()
            .await
            .with_context(|| format!("Failed to call {method}"))?;

        let json_response: Value = response.json().await?;

        if let Some(error) = json_response.get("error") {
            anyhow::bail!("{method} failed: {error}");
        }

        json_response
            .get("result")
            .cloned()
            .context("No result in response")
    }

    /// Submit work without blocking: `SendMessage` with `returnImmediately`
    /// returns the `submitted` task snapshot and executes in the background.
    async fn submit_task(&self, tool_name: &str, parameters: Value) -> Result<Task> {
        info!("📝 Submitting task for tool: {}", tool_name);

        let result = self
            .call(
                "SendMessage",
                json!({
                    "message": {
                        "messageId": Uuid::new_v4().to_string(),
                        "role": "ROLE_USER",
                        "parts": [{
                            "data": { "tool_name": tool_name, "parameters": parameters }
                        }]
                    },
                    "configuration": { "returnImmediately": true }
                }),
            )
            .await?;

        // SendMessage returns the SendMessageResponse oneof {"task": ...}.
        let task: Task = serde_json::from_value(
            result.get("task").context("No task in response")?.clone(),
        )?;

        info!("✅ Task submitted: {}", task.id);
        Ok(task)
    }

    /// Get task status via `GetTask`
    async fn get_task(&self, task_id: &str) -> Result<Task> {
        let result = self.call("GetTask", json!({ "id": task_id })).await?;
        Ok(serde_json::from_value(result)?)
    }

    /// List tasks via `ListTasks` (first page)
    async fn list_tasks(&self) -> Result<Vec<Task>> {
        info!("📋 Listing tasks");

        let result = self
            .call("ListTasks", json!({ "pageSize": 50, "includeArtifacts": false }))
            .await?;

        let tasks: Vec<Task> = serde_json::from_value(
            result.get("tasks").context("No tasks in result")?.clone(),
        )?;

        let next = result
            .get("nextPageToken")
            .and_then(|t| t.as_str())
            .unwrap_or_default();
        if !next.is_empty() {
            info!("   (more pages available, nextPageToken={next})");
        }

        info!("✅ Found {} tasks", tasks.len());
        Ok(tasks)
    }

    /// Monitor task until it reaches a terminal state
    async fn monitor_task(&self, task_id: &str, max_attempts: usize) -> Result<Task> {
        info!("👀 Monitoring task: {}", task_id);

        for attempt in 1..=max_attempts {
            let task = self.get_task(task_id).await?;

            match task.status.state {
                TaskState::Submitted => {
                    info!("   [{}] Task is submitted...", attempt);
                }
                TaskState::Working => {
                    info!("   [{}] Task is working...", attempt);
                }
                TaskState::InputRequired | TaskState::AuthRequired => {
                    warn!("   [{}] ⏸ Task is interrupted: {:?}", attempt, task.status.state);
                    return Ok(task);
                }
                TaskState::Completed => {
                    info!("   [{}] ✅ Task completed!", attempt);
                    return Ok(task);
                }
                TaskState::Failed => {
                    warn!("   [{}] ❌ Task failed: {:?}", attempt, task.status.message);
                    return Ok(task);
                }
                TaskState::Canceled | TaskState::Rejected => {
                    warn!("   [{}] 🚫 Task ended: {:?}", attempt, task.status.state);
                    return Ok(task);
                }
            }

            if attempt < max_attempts {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }

        anyhow::bail!("Task monitoring timeout after {max_attempts} attempts")
    }

    /// Demonstrate full task lifecycle
    async fn demonstrate_task_lifecycle(&self) -> Result<()> {
        info!("\n🔄 Demonstrating A2A 1.0 Task Lifecycle\n");

        // Step 1: Submit a tool invocation as a non-blocking task
        let task = self
            .submit_task("get_activities", json!({ "limit": 5 }))
            .await?;
        info!("\n📊 Task Details:");
        info!("   ID: {}", task.id);
        info!("   Context: {:?}", task.context_id);
        info!("   State: {:?}", task.status.state);

        // Step 2: Monitor task until terminal
        info!("\n👀 Monitoring task status...");
        let final_task = self.monitor_task(&task.id, 5).await?;

        info!("\n📋 Final Task Status:");
        info!("   ID: {}", final_task.id);
        info!("   State: {:?}", final_task.status.state);
        info!("   Timestamp: {:?}", final_task.status.timestamp);

        if let Some(artifact) = final_task.artifacts.first() {
            info!("   Artifact: {}", serde_json::to_string_pretty(artifact)?);
        }

        // Step 3: List tasks
        let tasks = self.list_tasks().await?;
        info!("\n📚 Tasks ({}):", tasks.len());
        for (idx, t) in tasks.iter().take(5).enumerate() {
            info!("   {}. {} - {:?}", idx + 1, t.id, t.status.state);
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter("task_manager_example=info")
        .init();

    info!("🚀 A2A 1.0 Task Lifecycle Management Example");
    info!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");

    // Load configuration
    let server_url = std::env::var("PIERRE_SERVER_URL")
        .unwrap_or_else(|_| "http://localhost:8081".to_string());

    let client_id = std::env::var("PIERRE_A2A_CLIENT_ID")
        .unwrap_or_else(|_| "task_manager_client".to_string());

    let client_secret = std::env::var("PIERRE_A2A_CLIENT_SECRET")
        .unwrap_or_else(|_| "demo_secret_123".to_string());

    // Create task manager
    let mut manager = TaskManager::new(server_url, client_id);

    // Authenticate
    if let Err(e) = manager.authenticate(&client_secret).await {
        tracing::error!("❌ Authentication failed: {}", e);
        tracing::error!("   Make sure to register an A2A client first:");
        tracing::error!("   See examples/agents/fitness_analyzer/README.md for setup instructions");
        return Err(e);
    }

    // Run demonstration
    match manager.demonstrate_task_lifecycle().await {
        Ok(()) => {
            info!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            info!("✅ Task Lifecycle Demo Completed Successfully");
            Ok(())
        }
        Err(e) => {
            tracing::error!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
            tracing::error!("❌ Task Lifecycle Demo Failed: {}", e);
            Err(e)
        }
    }
}
