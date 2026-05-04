// ABOUTME: Central SSE manager that coordinates both OAuth notifications and MCP protocol streams
// ABOUTME: Provides unified connection management with clean separation of stream types
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::{
    a2a_task_stream::A2ATaskStream, notifications::NotificationStream, protocol::McpProtocolStream,
};
use crate::constants::network_config::SSE_BROADCAST_CHANNEL_SIZE;
use crate::errors::AppError;
use crate::mcp::protocol::McpRequest;
use crate::mcp::resources::ServerContext;
use crate::mcp::tenant_isolation::validate_jwt_token_for_mcp;
use crate::models::OAuthNotification;
use chrono::{Duration, Utc};
use dashmap::DashMap;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::broadcast;
use tracing::{info, warn};
use uuid::Uuid;

use crate::middleware::redact_session_id;

/// Connection types for different SSE streams
#[derive(Debug, Clone)]
pub enum ConnectionType {
    /// OAuth notification stream for a specific user
    Notification {
        /// User ID for the notification stream
        user_id: Uuid,
    },
    /// MCP protocol stream for a client session
    Protocol {
        /// Session ID for the protocol stream
        session_id: String,
    },
    /// A2A task stream for tracking task progress
    A2ATask {
        /// Task ID being streamed
        task_id: String,
        /// Client ID that owns the task
        client_id: String,
    },
}

/// SSE connection metadata
#[derive(Debug, Clone)]
pub struct ConnectionMetadata {
    /// Type of SSE connection (notification or protocol)
    pub connection_type: ConnectionType,
    /// When the connection was established
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Timestamp of last activity on this connection
    pub last_activity: chrono::DateTime<chrono::Utc>,
}

/// Unified SSE manager handling notification, protocol, and A2A task streams.
///
/// Uses `DashMap` for shard-level concurrent access — operations on different
/// keys never contend, eliminating the global `RwLock` bottleneck that
/// serialised all stream lookups and metadata updates.
#[derive(Clone)]
pub struct SseManager {
    notification_streams: Arc<DashMap<Uuid, NotificationStream>>,
    protocol_streams: Arc<DashMap<String, McpProtocolStream>>,
    a2a_task_streams: Arc<DashMap<String, A2ATaskStream>>,
    connection_metadata: Arc<DashMap<String, ConnectionMetadata>>,
    /// Maps `user_id` to their active `session_ids` for protocol streams
    user_sessions: Arc<DashMap<Uuid, Vec<String>>>,
    /// Buffer size for SSE channels
    buffer_size: usize,
}

impl SseManager {
    /// Creates a new SSE manager with the specified buffer size
    #[must_use]
    pub fn new(buffer_size: usize) -> Self {
        Self {
            notification_streams: Arc::new(DashMap::new()),
            protocol_streams: Arc::new(DashMap::new()),
            a2a_task_streams: Arc::new(DashMap::new()),
            connection_metadata: Arc::new(DashMap::new()),
            user_sessions: Arc::new(DashMap::new()),
            buffer_size,
        }
    }
}

impl Default for SseManager {
    fn default() -> Self {
        // Use default buffer size from constants
        Self::new(SSE_BROADCAST_CHANNEL_SIZE)
    }
}

impl SseManager {
    /// Register a new OAuth notification stream for a user
    pub async fn register_notification_stream(&self, user_id: Uuid) -> broadcast::Receiver<String> {
        let stream = NotificationStream::new(self.buffer_size);
        let receiver = stream.subscribe().await;

        self.notification_streams.insert(user_id, stream);

        let connection_id = format!("notification_{user_id}");
        let metadata = ConnectionMetadata {
            connection_type: ConnectionType::Notification { user_id },
            created_at: Utc::now(),
            last_activity: Utc::now(),
        };
        self.connection_metadata.insert(connection_id, metadata);

        info!("Registered notification stream for user: {}", user_id);
        receiver
    }

    /// Register a new MCP protocol stream for a session
    pub async fn register_protocol_stream(
        &self,
        session_id: String,
        authorization: Option<String>,
        resources: Arc<ServerContext>,
    ) -> broadcast::Receiver<String> {
        let stream = McpProtocolStream::new(resources.clone());
        let receiver = stream.subscribe().await;

        self.protocol_streams.insert(session_id.clone(), stream);

        // Extract user_id from JWT token if provided
        let user_id = if let Some(auth) = authorization {
            if let Some(token) = auth.strip_prefix("Bearer ") {
                if let Ok(jwt_result) = validate_jwt_token_for_mcp(
                    token,
                    &resources.auth_manager,
                    &resources.jwks_manager,
                    &resources.repos,
                )
                .await
                {
                    Some(jwt_result.user_id)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

        // Track session for this user
        if let Some(user_id) = user_id {
            self.user_sessions
                .entry(user_id)
                .or_default()
                .push(session_id.clone());
            info!(
                "Registered protocol stream for session {} belonging to user {}",
                redact_session_id(&session_id),
                user_id
            );
        } else {
            info!(
                "Registered protocol stream for session: {}",
                redact_session_id(&session_id)
            );
        }

        let connection_id = format!("protocol_{session_id}");
        let metadata = ConnectionMetadata {
            connection_type: ConnectionType::Protocol {
                session_id: session_id.clone(),
            },
            created_at: Utc::now(),
            last_activity: Utc::now(),
        };
        self.connection_metadata.insert(connection_id, metadata);

        receiver
    }

    /// Send OAuth notification to a specific user
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No notification stream is found for the specified user
    /// - The underlying stream fails to send the notification
    pub async fn send_notification(
        &self,
        user_id: Uuid,
        notification: &OAuthNotification,
    ) -> Result<(), AppError> {
        if let Some(stream) = self.notification_streams.get(&user_id) {
            stream.send_notification(notification).await?;

            // Update last activity
            let connection_id = format!("notification_{user_id}");
            if let Some(mut metadata) = self.connection_metadata.get_mut(&connection_id) {
                metadata.last_activity = Utc::now();
            }

            Ok(())
        } else {
            Err(AppError::not_found(format!(
                "Notification stream for user {user_id}"
            )))
        }
    }

    /// Send OAuth notification to all MCP protocol streams for a user
    ///
    /// # Errors
    ///
    /// Returns an error if sending to any stream fails
    pub async fn send_oauth_notification_to_protocol_streams(
        &self,
        user_id: Uuid,
        notification: &OAuthNotification,
    ) -> Result<(), AppError> {
        let session_ids = self
            .user_sessions
            .get(&user_id)
            .map(|entry| entry.value().clone());

        if let Some(sessions) = session_ids {
            let mut sent_count = 0;

            for session_id in &sessions {
                if let Some(stream) = self.protocol_streams.get(session_id) {
                    if let Err(e) = stream.send_oauth_notification(notification).await {
                        warn!(
                            "Failed to send OAuth notification to session {}: {}",
                            redact_session_id(session_id),
                            e
                        );
                    } else {
                        sent_count += 1;
                    }
                }
            }

            if sent_count > 0 {
                info!(
                    "Sent OAuth notification to {} protocol stream(s) for user {}",
                    sent_count, user_id
                );
                Ok(())
            } else {
                Err(AppError::not_found(format!(
                    "Active protocol streams for user {user_id}"
                )))
            }
        } else {
            Err(AppError::not_found(format!(
                "Protocol streams for user {user_id}"
            )))
        }
    }

    /// Send MCP request to a protocol stream
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No protocol stream is found for the specified session ID
    /// - The underlying stream fails to handle the request
    pub async fn send_mcp_request(
        &self,
        session_id: &str,
        request: McpRequest,
    ) -> Result<(), AppError> {
        if let Some(stream) = self.protocol_streams.get(session_id) {
            stream.handle_request(request).await?;

            // Update last activity
            let connection_id = format!("protocol_{session_id}");
            if let Some(mut metadata) = self.connection_metadata.get_mut(&connection_id) {
                metadata.last_activity = Utc::now();
            }

            Ok(())
        } else {
            Err(AppError::not_found(format!(
                "Protocol stream for session {session_id}"
            )))
        }
    }

    /// Unregister a notification stream
    pub fn unregister_notification_stream(&self, user_id: Uuid) {
        self.notification_streams.remove(&user_id);

        let connection_id = format!("notification_{user_id}");
        self.connection_metadata.remove(&connection_id);

        info!("Unregistered notification stream for user: {}", user_id);
    }

    /// Unregister a protocol stream
    pub fn unregister_protocol_stream(&self, session_id: &str) {
        self.protocol_streams.remove(session_id);

        let connection_id = format!("protocol_{session_id}");
        self.connection_metadata.remove(&connection_id);

        // Clean up session from user_sessions to prevent memory leak
        self.user_sessions.retain(|_user_id, sessions| {
            sessions.retain(|s| s != session_id);
            // Keep user entry only if they still have active sessions
            !sessions.is_empty()
        });

        info!(
            "Unregistered protocol stream for session: {}",
            redact_session_id(session_id)
        );
    }

    /// Get count of active notification streams
    #[must_use]
    pub fn active_notification_streams(&self) -> usize {
        self.notification_streams.len()
    }

    /// Get count of active protocol streams
    #[must_use]
    pub fn active_protocol_streams(&self) -> usize {
        self.protocol_streams.len()
    }

    /// Get all connection metadata for monitoring
    #[must_use]
    pub fn get_connection_metadata(&self) -> HashMap<String, ConnectionMetadata> {
        self.connection_metadata
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect()
    }

    /// Clean up inactive connections based on timeout
    pub fn cleanup_inactive_connections(&self, timeout_seconds: u64) {
        let timeout_seconds = i64::try_from(timeout_seconds).unwrap_or(i64::MAX);
        let cutoff = Utc::now() - Duration::seconds(timeout_seconds);
        let mut to_remove = Vec::new();

        for entry in self.connection_metadata.iter() {
            if entry.value().last_activity < cutoff {
                to_remove.push((entry.key().clone(), entry.value().connection_type.clone()));
            }
        }

        for (connection_id, connection_type) in to_remove {
            match connection_type {
                ConnectionType::Notification { user_id } => {
                    self.unregister_notification_stream(user_id);
                }
                ConnectionType::Protocol { session_id } => {
                    self.unregister_protocol_stream(&session_id);
                }
                ConnectionType::A2ATask { task_id, .. } => {
                    self.unregister_a2a_task_stream(&task_id);
                }
            }
            info!("Cleaned up inactive connection: {}", connection_id);
        }
    }

    /// Register a new A2A task stream for a task
    pub fn register_a2a_task_stream(
        &self,
        task_id: String,
        client_id: String,
    ) -> broadcast::Receiver<String> {
        let stream = A2ATaskStream::new(self.buffer_size);
        let receiver = stream.subscribe();

        self.a2a_task_streams.insert(task_id.clone(), stream);

        let connection_id = format!("a2a_task_{task_id}");
        info!("Registered A2A task stream for task: {}", task_id);

        let metadata = ConnectionMetadata {
            connection_type: ConnectionType::A2ATask { task_id, client_id },
            created_at: Utc::now(),
            last_activity: Utc::now(),
        };
        self.connection_metadata.insert(connection_id, metadata);
        receiver
    }

    /// Send task status update to A2A task stream
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - No task stream is found for the specified `task_id`
    /// - The underlying stream fails to send the update
    pub fn send_a2a_task_update(&self, task_id: &str, event_data: String) -> Result<(), AppError> {
        if let Some(stream) = self.a2a_task_streams.get(task_id) {
            stream
                .send_update(event_data)
                .map_err(|e| AppError::internal(format!("Failed to send task update: {e}")))?;

            // Update last activity
            let connection_id = format!("a2a_task_{task_id}");
            if let Some(mut metadata) = self.connection_metadata.get_mut(&connection_id) {
                metadata.last_activity = Utc::now();
            }

            Ok(())
        } else {
            Err(AppError::not_found(format!(
                "A2A task stream for task {task_id}"
            )))
        }
    }

    /// Unregister an A2A task stream
    pub fn unregister_a2a_task_stream(&self, task_id: &str) {
        self.a2a_task_streams.remove(task_id);

        let connection_id = format!("a2a_task_{task_id}");
        self.connection_metadata.remove(&connection_id);

        info!("Unregistered A2A task stream for task: {}", task_id);
    }

    /// Get count of active A2A task streams
    #[must_use]
    pub fn active_a2a_task_streams(&self) -> usize {
        self.a2a_task_streams.len()
    }
}
