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

/// A2A Task structure for long-running operations
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2ATask {
    /// Unique task identifier
    pub id: String,
    /// Current status of the task
    pub status: TaskStatus,
    /// When the task was created
    pub created_at: DateTime<Utc>,
    /// When the task completed (if finished)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    /// Task result data (if completed successfully)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Error message (if failed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Client ID that created this task
    pub client_id: String,
    /// Type of task being performed
    pub task_type: String,
    /// Input data for the task
    pub input_data: serde_json::Value,
    /// Output data from the task (if completed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_data: Option<serde_json::Value>,
    /// Detailed error message (if failed)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// When the task was last updated
    pub updated_at: DateTime<Utc>,
}

/// Task status enumeration
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    /// Task is queued but not yet started
    Pending,
    /// Task is currently executing
    Running,
    /// Task finished successfully
    Completed,
    /// Task failed with an error
    Failed,
    /// Task was cancelled by user or system
    Cancelled,
}

impl Display for TaskStatus {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::Running => write!(f, "running"),
            Self::Completed => write!(f, "completed"),
            Self::Failed => write!(f, "failed"),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
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
