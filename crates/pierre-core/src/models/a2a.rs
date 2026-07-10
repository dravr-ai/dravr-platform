// ABOUTME: A2A protocol data types shared between pierre-core and the main server crate
// ABOUTME: Contains client, session, task, usage, and status models for agent-to-agent communication
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A2A Data Models
//!
//! Pure data types for the A2A (Agent-to-Agent) protocol layer. These types
//! represent clients, sessions, tasks, and usage records exchanged between
//! agents and stored in the database.

use crate::constants::rate_limits::DEFAULT_BURST_LIMIT;
use crate::constants::time::HOUR_SECONDS;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt::{self, Display, Formatter};

/// A2A Client registration information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2AClient {
    /// Unique client identifier
    pub id: String,
    /// User ID for session tracking and consistency
    pub user_id: uuid::Uuid,
    /// Human-readable client name
    pub name: String,
    /// Description of the client application
    pub description: String,
    /// Public key for signature verification
    pub public_key: String,
    /// List of capabilities this client can access
    pub capabilities: Vec<String>,
    /// Allowed OAuth redirect URIs
    pub redirect_uris: Vec<String>,
    /// Whether this client is active
    pub is_active: bool,
    /// When this client was created
    pub created_at: DateTime<Utc>,
    // Additional fields for database compatibility
    /// List of permissions granted to this client
    #[serde(default = "default_permissions")]
    pub permissions: Vec<String>,
    /// Maximum requests allowed per window
    #[serde(default = "default_rate_limit_requests")]
    pub rate_limit_requests: u32,
    /// Rate limit window duration in seconds
    #[serde(default = "default_rate_limit_window")]
    pub rate_limit_window_seconds: u32,
    /// When this client was last updated
    #[serde(default = "chrono::Utc::now")]
    pub updated_at: DateTime<Utc>,
}

fn default_permissions() -> Vec<String> {
    vec!["read_activities".into()]
}

const fn default_rate_limit_requests() -> u32 {
    DEFAULT_BURST_LIMIT * 10
}

#[allow(clippy::cast_possible_truncation)] // Safe: HOUR_SECONDS is 3600, well within u32 range
const fn default_rate_limit_window() -> u32 {
    HOUR_SECONDS as u32
}

/// A2A Active session information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ASession {
    /// Unique session identifier
    pub id: String,
    /// Client ID that owns this session
    pub client_id: String,
    /// User ID if the session is user-scoped
    pub user_id: Option<uuid::Uuid>,
    /// `OAuth2` scopes granted to this session
    pub granted_scopes: Vec<String>,
    /// When the session was created
    pub created_at: DateTime<Utc>,
    /// When the session expires
    pub expires_at: DateTime<Utc>,
    /// Timestamp of the last API activity
    pub last_activity: DateTime<Utc>,
    /// Total number of requests made in this session
    pub requests_count: u64,
}

/// A2A Task database record.
///
/// Stores the persistent state behind the A2A 1.0 wire `Task` object. The
/// wire-facing `camelCase` shape (with `TaskStatus{state,message,timestamp}`,
/// `history`, `artifacts`) is assembled in `pierre-a2a` from this record;
/// `status_message`, `history`, and `artifacts` hold pre-serialized wire JSON
/// so this crate stays independent of the protocol types.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ATask {
    /// Unique task identifier
    pub id: String,
    /// Current lifecycle state of the task
    pub status: TaskStatus,
    /// Server-side context grouping identifier (A2A `contextId`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    /// Wire `Message` JSON attached to the current status (A2A `TaskStatus.message`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_message: Option<serde_json::Value>,
    /// Wire `Message[]` JSON of the task's message history
    #[serde(skip_serializing_if = "Option::is_none")]
    pub history: Option<serde_json::Value>,
    /// Wire `Artifact[]` JSON of the task's generated artifacts
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<serde_json::Value>,
    /// Client ID that created this task
    pub client_id: String,
    /// Type of task being performed
    pub task_type: String,
    /// Input data for the task
    pub input_data: serde_json::Value,
    /// Result data from the task (if completed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// When the task was created
    pub created_at: DateTime<Utc>,
    /// When the task was last updated (A2A status timestamp)
    pub updated_at: DateTime<Utc>,
}

/// Task lifecycle states per the A2A protocol 1.0 `TaskState` enum.
///
/// Serde names are the `ProtoJSON` wire strings (`TASK_STATE_*`); [`Display`]
/// yields the compact strings persisted in the database `status` column.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TaskStatus {
    /// Task received and acknowledged, not yet processing
    #[serde(rename = "TASK_STATE_SUBMITTED")]
    Submitted,
    /// Task is actively being processed
    #[serde(rename = "TASK_STATE_WORKING")]
    Working,
    /// Task finished successfully (terminal)
    #[serde(rename = "TASK_STATE_COMPLETED")]
    Completed,
    /// Task failed with an error (terminal)
    #[serde(rename = "TASK_STATE_FAILED")]
    Failed,
    /// Task was canceled by the client or system (terminal)
    #[serde(rename = "TASK_STATE_CANCELED")]
    Canceled,
    /// Task is paused waiting for client input (interrupted)
    #[serde(rename = "TASK_STATE_INPUT_REQUIRED")]
    InputRequired,
    /// Task was rejected by the agent (terminal)
    #[serde(rename = "TASK_STATE_REJECTED")]
    Rejected,
    /// Task is paused waiting for additional authentication (interrupted)
    #[serde(rename = "TASK_STATE_AUTH_REQUIRED")]
    AuthRequired,
}

impl TaskStatus {
    /// Whether this state is terminal — no further transitions are allowed
    /// and any subscription stream must close.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Completed | Self::Failed | Self::Canceled | Self::Rejected
        )
    }

    /// Whether this state is interrupted — paused awaiting client action.
    #[must_use]
    pub const fn is_interrupted(self) -> bool {
        matches!(self, Self::InputRequired | Self::AuthRequired)
    }
}

impl Display for TaskStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Submitted => write!(f, "submitted"),
            Self::Working => write!(f, "working"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Canceled => write!(f, "canceled"),
            Self::InputRequired => write!(f, "input-required"),
            Self::Rejected => write!(f, "rejected"),
            Self::AuthRequired => write!(f, "auth-required"),
        }
    }
}

/// A2A push notification configuration record (A2A 1.0
/// `TaskPushNotificationConfig`).
///
/// One row per webhook registration on a task; the server POSTs task state
/// changes to `url` using the stored authentication material.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2APushNotificationConfig {
    /// Unique configuration identifier (A2A `id`)
    pub config_id: String,
    /// Task this configuration is attached to
    pub task_id: String,
    /// Webhook URL to POST task updates to
    pub url: String,
    /// Opaque client token echoed back in notifications
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Authentication scheme for the webhook request (e.g. "Bearer")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_scheme: Option<String>,
    /// Credentials paired with `auth_scheme`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_credentials: Option<String>,
    /// When this configuration was created
    pub created_at: DateTime<Utc>,
    /// When this configuration was last updated
    pub updated_at: DateTime<Utc>,
}

/// Records of A2A protocol usage for analytics and billing
#[derive(Debug, Serialize, Deserialize)]
pub struct A2AUsage {
    /// Database record ID (None for new records)
    pub id: Option<i64>,
    /// A2A client identifier
    pub client_id: String,
    /// Optional session token for this request
    pub session_token: Option<String>,
    /// When the request was made
    pub timestamp: DateTime<Utc>,
    /// Name of the tool/endpoint called
    pub tool_name: String,
    /// Response time in milliseconds
    pub response_time_ms: Option<u32>,
    /// HTTP status code returned
    pub status_code: u16,
    /// Error message if request failed
    pub error_message: Option<String>,
    /// Request payload size in bytes
    pub request_size_bytes: Option<u32>,
    /// Response payload size in bytes
    pub response_size_bytes: Option<u32>,
    /// Client IP address
    pub ip_address: Option<String>,
    /// Client user agent string
    pub user_agent: Option<String>,
    /// A2A protocol version used
    pub protocol_version: String,
    /// List of capabilities advertised by client
    pub client_capabilities: Vec<String>,
    /// OAuth scopes granted for this request
    pub granted_scopes: Vec<String>,
}

/// Aggregated statistics for A2A usage over a time period
#[derive(Debug, Serialize, Deserialize)]
pub struct A2AUsageStats {
    /// A2A client identifier
    pub client_id: String,
    /// Start of the statistics period
    pub period_start: DateTime<Utc>,
    /// End of the statistics period
    pub period_end: DateTime<Utc>,
    /// Total number of requests in period
    pub total_requests: u32,
    /// Number of successful requests (2xx status)
    pub successful_requests: u32,
    /// Number of failed requests (4xx/5xx status)
    pub failed_requests: u32,
    /// Average response time across all requests (ms)
    pub avg_response_time_ms: Option<u32>,
    /// Total bytes sent in requests
    pub total_request_bytes: Option<u64>,
    /// Total bytes sent in responses
    pub total_response_bytes: Option<u64>,
}
