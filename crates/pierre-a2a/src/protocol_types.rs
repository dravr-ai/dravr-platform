// ABOUTME: A2A 1.0 wire data model (Task/Message/Part/Artifact/events) and error types
// ABOUTME: ProtoJSON shapes per the normative a2a.proto — camelCase fields, TASK_STATE_*/ROLE_* enums
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::jsonrpc::JsonRpcError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

pub use pierre_core::models::a2a::TaskStatus as TaskState;

/// A2A protocol version implemented by this server.
///
/// Per spec §3.6 the negotiated form is `Major.Minor` — patch numbers MUST
/// NOT appear in agent cards, requests, or version negotiation.
pub const A2A_VERSION: &str = "1.0";

/// HTTP header carrying the client's requested protocol version (spec §3.6).
pub const A2A_VERSION_HEADER: &str = "A2A-Version";

/// `google.rpc.ErrorInfo` domain for A2A protocol errors (spec §5.4).
pub const A2A_ERROR_DOMAIN: &str = "a2a-protocol.org";

/// Build the `google.rpc.ErrorInfo` detail array required in the `data`
/// member of every A2A-specific JSON-RPC error (spec §5.4).
#[must_use]
pub fn error_info(reason: &str) -> Value {
    json!([{
        "@type": "type.googleapis.com/google.rpc.ErrorInfo",
        "reason": reason,
        "domain": A2A_ERROR_DOMAIN,
    }])
}

/// A2A-specific JSON-RPC errors (spec §5.4, codes `-32001..-32009`).
///
/// Distinct from [`A2AError`], which covers Pierre's A2A management plane
/// (client registration, sessions, rate limits) rather than the protocol
/// wire surface.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum A2ASpecError {
    /// The referenced task id does not exist
    #[error("Task not found")]
    TaskNotFound,
    /// The task is in a terminal state and cannot be canceled
    #[error("Task cannot be canceled")]
    TaskNotCancelable,
    /// The server does not support push notifications
    #[error("Push notifications are not supported")]
    PushNotificationNotSupported,
    /// The requested operation is not supported for this task/state
    #[error("This operation is not supported")]
    UnsupportedOperation,
    /// A provided content (media) type is not supported
    #[error("Content type is not supported")]
    ContentTypeNotSupported,
    /// The agent produced an invalid response for the request
    #[error("Invalid agent response")]
    InvalidAgentResponse,
    /// No extended agent card is configured on this server
    #[error("Extended agent card is not configured")]
    ExtendedAgentCardNotConfigured,
    /// A required protocol extension is not supported by the server
    #[error("A required extension is not supported")]
    ExtensionSupportRequired,
    /// The requested protocol version is not supported
    #[error("Requested protocol version is not supported")]
    VersionNotSupported,
}

impl A2ASpecError {
    /// JSON-RPC error code (spec §5.4)
    #[must_use]
    pub const fn code(self) -> i32 {
        match self {
            Self::TaskNotFound => -32001,
            Self::TaskNotCancelable => -32002,
            Self::PushNotificationNotSupported => -32003,
            Self::UnsupportedOperation => -32004,
            Self::ContentTypeNotSupported => -32005,
            Self::InvalidAgentResponse => -32006,
            Self::ExtendedAgentCardNotConfigured => -32007,
            Self::ExtensionSupportRequired => -32008,
            Self::VersionNotSupported => -32009,
        }
    }

    /// `google.rpc.ErrorInfo` reason: the error name in `UPPER_SNAKE_CASE`
    /// without the `Error` suffix (spec §5.4)
    #[must_use]
    pub const fn reason(self) -> &'static str {
        match self {
            Self::TaskNotFound => "TASK_NOT_FOUND",
            Self::TaskNotCancelable => "TASK_NOT_CANCELABLE",
            Self::PushNotificationNotSupported => "PUSH_NOTIFICATION_NOT_SUPPORTED",
            Self::UnsupportedOperation => "UNSUPPORTED_OPERATION",
            Self::ContentTypeNotSupported => "CONTENT_TYPE_NOT_SUPPORTED",
            Self::InvalidAgentResponse => "INVALID_AGENT_RESPONSE",
            Self::ExtendedAgentCardNotConfigured => "EXTENDED_AGENT_CARD_NOT_CONFIGURED",
            Self::ExtensionSupportRequired => "EXTENSION_SUPPORT_REQUIRED",
            Self::VersionNotSupported => "VERSION_NOT_SUPPORTED",
        }
    }

    /// HTTP status for the HTTP+JSON binding (v1.0.1 error-mapping table)
    #[must_use]
    pub const fn http_status(self) -> u16 {
        match self {
            Self::TaskNotFound => 404,
            Self::InvalidAgentResponse => 500,
            _ => 400,
        }
    }
}

impl From<A2ASpecError> for JsonRpcError {
    fn from(error: A2ASpecError) -> Self {
        Self {
            code: error.code(),
            message: error.to_string(),
            data: Some(error_info(error.reason())),
        }
    }
}

/// Pierre A2A management-plane error types (client registration, sessions,
/// credentials, rate limits).
///
/// These map onto JSON-RPC implementation-defined server errors (`-32000`)
/// or the standard invalid-params/internal codes — never onto the A2A
/// protocol codes `-32001..-32099`, which are reserved for [`A2ASpecError`].
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize, thiserror::Error)]
pub enum A2AError {
    /// Invalid request parameters or format
    #[error("Invalid request: {0}")]
    InvalidRequest(String),
    /// Authentication failed
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
    /// Client not registered
    #[error("Client not registered: {0}")]
    ClientNotRegistered(String),
    /// Database operation failed
    #[error("Database error: {0}")]
    DatabaseError(String),
    /// Internal server error
    #[error("Internal error: {0}")]
    InternalError(String),
    /// Client has been deactivated
    #[error("Client deactivated: {0}")]
    ClientDeactivated(String),
    /// Rate limit exceeded
    #[error("Rate limit exceeded: {0}")]
    RateLimitExceeded(String),
    /// Session expired or invalid
    #[error("Session expired: {0}")]
    SessionExpired(String),
    /// Invalid session token
    #[error("Invalid session token: {0}")]
    InvalidSessionToken(String),
    /// Insufficient permissions
    #[error("Insufficient permissions: {0}")]
    InsufficientPermissions(String),
    /// Resource not found
    #[error("Resource not found: {0}")]
    ResourceNotFound(String),
    /// Service temporarily unavailable
    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),
}

impl From<A2AError> for JsonRpcError {
    fn from(error: A2AError) -> Self {
        let (code, message) = match error {
            A2AError::InvalidRequest(msg) => (-32602, format!("Invalid params: {msg}")),
            A2AError::InternalError(msg) => (-32603, format!("Internal error: {msg}")),
            A2AError::AuthenticationFailed(msg) => {
                (-32000, format!("Authentication failed: {msg}"))
            }
            A2AError::ClientNotRegistered(msg) => (-32000, format!("Client not registered: {msg}")),
            A2AError::DatabaseError(msg) => (-32000, format!("Database error: {msg}")),
            A2AError::ClientDeactivated(msg) => (-32000, format!("Client deactivated: {msg}")),
            A2AError::RateLimitExceeded(msg) => (-32000, format!("Rate limit exceeded: {msg}")),
            A2AError::SessionExpired(msg) => (-32000, format!("Session expired: {msg}")),
            A2AError::InvalidSessionToken(msg) => (-32000, format!("Invalid session token: {msg}")),
            A2AError::InsufficientPermissions(msg) => {
                (-32000, format!("Insufficient permissions: {msg}"))
            }
            A2AError::ResourceNotFound(msg) => (-32000, format!("Resource not found: {msg}")),
            A2AError::ServiceUnavailable(msg) => (-32000, format!("Service unavailable: {msg}")),
        };

        Self {
            code,
            message,
            data: None,
        }
    }
}

/// Message role (A2A 1.0 `Role`, `ProtoJSON` enum strings).
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Role {
    /// Message authored by the client/user side
    #[serde(rename = "ROLE_USER")]
    User,
    /// Message authored by the agent
    #[serde(rename = "ROLE_AGENT")]
    Agent,
}

/// Content of a message/artifact part (A2A 1.0 `Part.content` oneof).
///
/// Externally tagged and flattened into [`Part`], producing the `ProtoJSON`
/// oneof form: exactly one of `text` / `raw` / `url` / `data` is present as
/// a JSON member — the `kind` discriminators of pre-1.0 are gone.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PartContent {
    /// Plain text content
    #[serde(rename = "text")]
    Text(String),
    /// Raw bytes, base64-encoded in JSON
    #[serde(rename = "raw")]
    Raw(String),
    /// Content referenced by URL
    #[serde(rename = "url")]
    Url(String),
    /// Structured JSON data
    #[serde(rename = "data")]
    Data(Value),
}

/// A single part of a message or artifact (A2A 1.0 `Part`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Part {
    /// The part content (exactly one of `text`/`raw`/`url`/`data`)
    #[serde(flatten)]
    pub content: PartContent,
    /// Optional file name associated with the content
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// Media (MIME) type of the content — renamed from pre-1.0 `mimeType`
    #[serde(rename = "mediaType", skip_serializing_if = "Option::is_none")]
    pub media_type: Option<String>,
    /// Part-level metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

impl Part {
    /// Create a plain-text part
    #[must_use]
    pub const fn text(text: String) -> Self {
        Self {
            content: PartContent::Text(text),
            filename: None,
            media_type: None,
            metadata: None,
        }
    }

    /// Create a structured-data part
    #[must_use]
    pub const fn data(data: Value) -> Self {
        Self {
            content: PartContent::Data(data),
            filename: None,
            media_type: None,
            metadata: None,
        }
    }
}

/// A message exchanged between client and agent (A2A 1.0 `Message`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    /// Creator-generated unique message identifier (required)
    #[serde(rename = "messageId")]
    pub message_id: String,
    /// Author role (required)
    pub role: Role,
    /// Message content — at least one part (required)
    pub parts: Vec<Part>,
    /// Context this message belongs to
    #[serde(rename = "contextId", skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    /// Task this message belongs to
    #[serde(rename = "taskId", skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Related task ids the agent may use as context
    #[serde(
        rename = "referenceTaskIds",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub reference_task_ids: Vec<String>,
    /// URIs of protocol extensions relevant to this message
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    /// Message-level metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

impl Message {
    /// Create an agent-authored message with a fresh id
    #[must_use]
    pub fn agent(message_id: String, parts: Vec<Part>) -> Self {
        Self {
            message_id,
            role: Role::Agent,
            parts,
            context_id: None,
            task_id: None,
            reference_task_ids: Vec::new(),
            extensions: Vec::new(),
            metadata: None,
        }
    }
}

/// Current status of a task (A2A 1.0 `TaskStatus`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireTaskStatus {
    /// Lifecycle state (required)
    pub state: TaskState,
    /// Message associated with this status (e.g. the agent's input request)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Message>,
    /// ISO-8601 UTC timestamp of the status change
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
}

/// An output generated by a task (A2A 1.0 `Artifact`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Artifact {
    /// Unique artifact identifier within the task (required)
    #[serde(rename = "artifactId")]
    pub artifact_id: String,
    /// Artifact content — at least one part (required)
    pub parts: Vec<Part>,
    /// Human-readable name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Human-readable description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// URIs of protocol extensions relevant to this artifact
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<String>,
    /// Artifact-level metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

/// A stateful unit of work (A2A 1.0 `Task`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    /// Server-generated task identifier (required)
    pub id: String,
    /// Current status (required)
    pub status: WireTaskStatus,
    /// Context grouping identifier
    #[serde(rename = "contextId", skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    /// Artifacts generated by the task
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<Artifact>,
    /// Message history of the task
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Message>,
    /// Task-level metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

/// Event notifying a task status change (A2A 1.0 `TaskStatusUpdateEvent`).
///
/// The pre-1.0 `final` flag is gone — a subscription stream signals task
/// completion by closing after the terminal-state event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskStatusUpdateEvent {
    /// Task that changed (required)
    #[serde(rename = "taskId")]
    pub task_id: String,
    /// Context of the task (required)
    #[serde(rename = "contextId")]
    pub context_id: String,
    /// The new status (required)
    pub status: WireTaskStatus,
    /// Event-level metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

/// Event carrying a new or updated artifact (A2A 1.0 `TaskArtifactUpdateEvent`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskArtifactUpdateEvent {
    /// Task that produced the artifact (required)
    #[serde(rename = "taskId")]
    pub task_id: String,
    /// Context of the task (required)
    #[serde(rename = "contextId")]
    pub context_id: String,
    /// The artifact (required)
    pub artifact: Artifact,
    /// Whether the parts append to a previous chunk of the same artifact
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub append: bool,
    /// Whether this is the final chunk of the artifact
    #[serde(
        rename = "lastChunk",
        default,
        skip_serializing_if = "std::ops::Not::not"
    )]
    pub last_chunk: bool,
    /// Event-level metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

/// One frame of a streaming response (A2A 1.0 `StreamResponse` oneof).
///
/// `ProtoJSON` oneof: exactly one member is present. On the JSON-RPC binding
/// each SSE `data:` line wraps this in a full JSON-RPC envelope; on the
/// HTTP+JSON binding the bare object is the `data:` payload.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum StreamResponse {
    /// Initial or final task snapshot
    #[serde(rename = "task")]
    Task(Task),
    /// A direct agent message (message-only exchanges)
    #[serde(rename = "message")]
    Message(Message),
    /// Task status change
    #[serde(rename = "statusUpdate")]
    StatusUpdate(TaskStatusUpdateEvent),
    /// Artifact produced/updated
    #[serde(rename = "artifactUpdate")]
    ArtifactUpdate(TaskArtifactUpdateEvent),
}

/// Configuration accompanying `SendMessage` (A2A 1.0 `SendMessageConfiguration`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SendMessageConfiguration {
    /// Media types the client accepts in the response
    #[serde(
        rename = "acceptedOutputModes",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub accepted_output_modes: Vec<String>,
    /// Return the task immediately after submission instead of blocking
    /// until a terminal/interrupted state (replaces pre-1.0 `blocking`,
    /// with inverted polarity — the default `false` blocks)
    #[serde(rename = "returnImmediately", default)]
    pub return_immediately: bool,
    /// Number of history messages to include in returned tasks
    #[serde(rename = "historyLength", skip_serializing_if = "Option::is_none")]
    pub history_length: Option<u32>,
    /// Push notification configuration to register at send time
    #[serde(
        rename = "pushNotificationConfig",
        skip_serializing_if = "Option::is_none"
    )]
    pub push_notification_config: Option<PushNotificationConfigInput>,
}

/// Parameters of `SendMessage` / `SendStreamingMessage` (A2A 1.0
/// `SendMessageRequest`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessageRequest {
    /// The message to deliver (required)
    pub message: Message,
    /// Send configuration
    #[serde(skip_serializing_if = "Option::is_none")]
    pub configuration: Option<SendMessageConfiguration>,
    /// Request-level metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Map<String, Value>>,
}

/// Parameters of `GetTask` (A2A 1.0 `GetTaskRequest`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetTaskRequest {
    /// Task identifier (required)
    pub id: String,
    /// Number of history messages to include (absent = full history)
    #[serde(rename = "historyLength", skip_serializing_if = "Option::is_none")]
    pub history_length: Option<u32>,
}

/// Parameters of `ListTasks` (A2A 1.0 `ListTasksRequest`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ListTasksRequest {
    /// Page size (1..=100, default 50)
    #[serde(rename = "pageSize", skip_serializing_if = "Option::is_none")]
    pub page_size: Option<u32>,
    /// Opaque continuation token from a previous response
    #[serde(rename = "pageToken", skip_serializing_if = "Option::is_none")]
    pub page_token: Option<String>,
    /// Filter: only tasks in this context
    #[serde(rename = "contextId", skip_serializing_if = "Option::is_none")]
    pub context_id: Option<String>,
    /// Filter: only tasks in this state (`TASK_STATE_*` wire string)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<TaskState>,
    /// Filter: only tasks whose status timestamp is after this ISO-8601 instant
    #[serde(
        rename = "statusTimestampAfter",
        skip_serializing_if = "Option::is_none"
    )]
    pub status_timestamp_after: Option<String>,
    /// Include artifacts in the returned tasks (default false = omitted)
    #[serde(rename = "includeArtifacts", default)]
    pub include_artifacts: bool,
    /// Number of history messages to include per task
    #[serde(rename = "historyLength", skip_serializing_if = "Option::is_none")]
    pub history_length: Option<u32>,
}

/// Response of `ListTasks` (A2A 1.0 `ListTasksResponse`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListTasksResponse {
    /// The page of tasks, status-timestamp descending
    pub tasks: Vec<Task>,
    /// Continuation token — always present; empty string on the last page
    #[serde(rename = "nextPageToken")]
    pub next_page_token: String,
}

/// Authentication material for a push notification webhook (A2A 1.0
/// `AuthenticationInfo` inside `TaskPushNotificationConfig`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushNotificationAuthenticationInfo {
    /// Authentication scheme (e.g. "Bearer") (required)
    pub scheme: String,
    /// Credentials paired with the scheme
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<String>,
}

/// Client-supplied push notification configuration (create form — the id
/// is server-assigned when absent).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PushNotificationConfigInput {
    /// Optional client-suggested configuration id
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Webhook URL to POST task updates to (required)
    pub url: String,
    /// Opaque client token echoed back in notifications
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Webhook authentication material
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authentication: Option<PushNotificationAuthenticationInfo>,
}

/// A registered push notification configuration (A2A 1.0
/// `TaskPushNotificationConfig`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskPushNotificationConfig {
    /// Configuration identifier (required)
    pub id: String,
    /// Task the configuration is attached to (required)
    #[serde(rename = "taskId")]
    pub task_id: String,
    /// Webhook URL (required)
    pub url: String,
    /// Opaque client token echoed back in notifications
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// Webhook authentication material
    #[serde(skip_serializing_if = "Option::is_none")]
    pub authentication: Option<PushNotificationAuthenticationInfo>,
}
