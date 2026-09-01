// ABOUTME: Central SSE manager for OAuth notification streams, keyed by user
// ABOUTME: Owns stream registration, fan-out, metadata and inactive-connection cleanup
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::notifications::NotificationStream;
use async_trait::async_trait;
use chrono::{Duration, Utc};
use dashmap::DashMap;
use pierre_config::constants::network_config::SSE_BROADCAST_CHANNEL_SIZE;
use pierre_core::errors::AppError;
use pierre_core::models::OAuthNotification;
use pierre_services::provider_refresh::SyncNotifier;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::broadcast;
use tracing::info;
use uuid::Uuid;

/// Connection types for SSE streams.
///
/// Protocol-stream connections were removed with the session-keyed
/// `GET /mcp/sse/{session_id}` endpoint: revision 2026-07-28 deleted both
/// protocol sessions and the standalone GET stream.
#[derive(Debug, Clone)]
pub enum ConnectionType {
    /// OAuth notification stream for a specific user
    Notification {
        /// User ID for the notification stream
        user_id: Uuid,
    },
}

/// SSE connection metadata
#[derive(Debug, Clone)]
pub struct ConnectionMetadata {
    /// Type of SSE connection
    pub connection_type: ConnectionType,
    /// When the connection was established
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// Timestamp of last activity on this connection
    pub last_activity: chrono::DateTime<chrono::Utc>,
}

/// SSE manager for OAuth notification streams.
///
/// Uses `DashMap` for shard-level concurrent access — operations on different
/// keys never contend, eliminating the global `RwLock` bottleneck that
/// serialised all stream lookups and metadata updates.
#[derive(Clone)]
pub struct SseManager {
    notification_streams: Arc<DashMap<Uuid, NotificationStream>>,
    connection_metadata: Arc<DashMap<String, ConnectionMetadata>>,
    /// Buffer size for SSE channels
    buffer_size: usize,
}

impl SseManager {
    /// Creates a new SSE manager with the specified buffer size.
    #[must_use]
    pub fn new(buffer_size: usize) -> Self {
        Self {
            notification_streams: Arc::new(DashMap::new()),
            connection_metadata: Arc::new(DashMap::new()),
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

    /// Unregister a notification stream
    pub fn unregister_notification_stream(&self, user_id: Uuid) {
        self.notification_streams.remove(&user_id);

        let connection_id = format!("notification_{user_id}");
        self.connection_metadata.remove(&connection_id);

        info!("Unregistered notification stream for user: {}", user_id);
    }

    /// Get count of active notification streams
    #[must_use]
    pub fn active_notification_streams(&self) -> usize {
        self.notification_streams.len()
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
            }
            info!("Cleaned up inactive connection: {}", connection_id);
        }
    }
}

// Bridge so pierre-services::provider_refresh can push sync-completed
// notifications without depending on the pierre-server SSE/MCP machinery.
// The inherent `send_notification` method above stays the canonical
// entry point; this impl is a thin forward.
#[async_trait]
impl SyncNotifier for SseManager {
    async fn send_notification(
        &self,
        user_id: Uuid,
        notification: &OAuthNotification,
    ) -> Result<(), AppError> {
        Self::send_notification(self, user_id, notification).await
    }
}
