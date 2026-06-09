// ABOUTME: WebSocket implementation for real-time communication and live data streaming
// ABOUTME: Handles WebSocket connections, message routing, and real-time fitness data updates
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// NOTE: All `.clone()` calls in this file are Safe - they are necessary for:
// - Arc resource clones for multi-tenant concurrent access
// - String ownership transfers for WebSocket message construction

//! `WebSocket` support for real-time updates
//!
//! Provides real-time updates for API key usage, rate limit status,
//! and system metrics via `WebSocket` connections.

use pierre_auth::admin::jwks::JwksManager;

use crate::constants::rate_limits::{WEBSOCKET_CHANNEL_CAPACITY, WEBSOCKET_MAX_CONNECTION_SECS};
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use pierre_auth::auth::{AuthManager, AuthResult};
use pierre_config::environment::RateLimitConfig;
use pierre_core::errors::{AppError, AppResult};
use pierre_middleware::McpAuthMiddleware;
// UsageRepository dispatched through repos.usage
use dashmap::DashMap;
use pierre_database::{RepositoryRegistry, UsageRepos};
use serde::{Deserialize, Serialize};
use std::env;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc::unbounded_channel, mpsc::UnboundedSender};
use tokio::time::{interval, sleep, timeout, Duration};
use tracing::{debug, trace, warn};
use uuid::Uuid;

/// Upper bound on how long to wait for the forwarding task to flush queued frames
/// (e.g. the max-lifetime close frame) before abandoning the connection on cleanup.
const WS_DRAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// WebSocket message types for real-time communication
#[non_exhaustive]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WebSocketMessage {
    /// Client authentication message
    #[serde(rename = "auth")]
    Authentication {
        /// JWT authentication token
        token: String,
    },
    /// Subscribe to specific topics
    #[serde(rename = "subscribe")]
    Subscribe {
        /// List of topics to subscribe to
        topics: Vec<String>,
    },
    /// System-wide statistics update
    #[serde(rename = "system_stats")]
    SystemStats {
        /// Total requests across all keys today
        total_requests_today: u64,
        /// Total requests across all keys this month
        total_requests_this_month: u64,
        /// Number of active WebSocket connections
        active_connections: usize,
    },
    /// Error message to client
    #[serde(rename = "error")]
    Error {
        /// Error description
        message: String,
    },
    /// Success confirmation message
    #[serde(rename = "success")]
    Success {
        /// Success message
        message: String,
    },
}

/// Manages WebSocket connections and message broadcasting
#[derive(Clone)]
pub struct WebSocketManager {
    /// Narrow view: `WebSocketManager` only consults `repos.usage` for the
    /// `system_stats` push. JWT/API-key authentication runs through
    /// `auth_middleware`, which owns its own registry handle internally.
    repos: UsageRepos,
    auth_middleware: McpAuthMiddleware,
    clients: Arc<DashMap<Uuid, ClientConnection>>,
    broadcast_tx: broadcast::Sender<WebSocketMessage>,
    /// Maximum lifetime of a single connection before the server closes it. Bounds
    /// how long an idle/background client can pin a serverless instance open via
    /// auto-reconnect. Sourced from `WEBSOCKET_MAX_CONNECTION_SECS`.
    max_connection_duration: Duration,
}

#[derive(Debug)]
struct ClientConnection {
    subscriptions: Vec<String>,
    tx: UnboundedSender<Message>,
}

impl WebSocketManager {
    /// Creates a new WebSocket manager instance.
    ///
    /// Takes the full `RepositoryRegistry` because the constructor
    /// forwards an Arc-clone into `McpAuthMiddleware::new`, which holds
    /// the registry directly for auth lookups across `users`,
    /// `api_keys`, and `usage` (cross-cuts `AuthRepos` + `UsageRepos`).
    /// The narrowing for the manager itself is at the `repos` field —
    /// `UsageRepos` for the `system_stats` push.
    #[must_use]
    pub fn new(
        repos: &Arc<RepositoryRegistry>,
        auth_manager: &Arc<AuthManager>,
        jwks_manager: &Arc<JwksManager>,
        rate_limit_config: RateLimitConfig,
    ) -> Self {
        let (broadcast_tx, _) = broadcast::channel(WEBSOCKET_CHANNEL_CAPACITY);
        let auth_middleware = McpAuthMiddleware::new(
            (**auth_manager).clone(),
            repos.clone(),
            jwks_manager.clone(),
            rate_limit_config,
        ); // Safe: Arc clones for middleware creation

        let max_connection_secs = env::var("WEBSOCKET_MAX_CONNECTION_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .filter(|secs| *secs > 0)
            .unwrap_or(WEBSOCKET_MAX_CONNECTION_SECS);

        Self {
            repos: repos.usage_repos(),
            auth_middleware,
            clients: Arc::new(DashMap::new()),
            broadcast_tx,
            max_connection_duration: Duration::from_secs(max_connection_secs),
        }
    }

    /// Handle authentication message and return authenticated user ID
    async fn handle_auth_message(
        &self,
        token: &str,
        tx: &UnboundedSender<Message>,
    ) -> Option<Uuid> {
        match self.authenticate_user(token).await {
            Ok(auth_result) => {
                let success_msg = WebSocketMessage::Success {
                    message: "Authentication successful".into(),
                };
                if let Ok(json) = serde_json::to_string(&success_msg) {
                    if let Err(e) = tx.send(Message::Text(json.into())) {
                        warn!(
                            user_id = %auth_result.user_id,
                            error = ?e,
                            "Failed to send authentication success message over WebSocket"
                        );
                    }
                }
                Some(auth_result.user_id)
            }
            Err(e) => {
                let error_msg = WebSocketMessage::Error {
                    message: format!("Authentication failed: {e}"),
                };
                if let Ok(json) = serde_json::to_string(&error_msg) {
                    if let Err(send_err) = tx.send(Message::Text(json.into())) {
                        warn!(
                            auth_error = %e,
                            send_error = ?send_err,
                            "Failed to send authentication error message over WebSocket"
                        );
                    }
                }
                None
            }
        }
    }

    /// Handle subscribe message and update subscriptions
    fn handle_subscribe_message(
        topics: Vec<String>,
        is_authenticated: bool,
        tx: &UnboundedSender<Message>,
    ) -> Vec<String> {
        if is_authenticated {
            let success_msg = WebSocketMessage::Success {
                message: format!("Subscribed to {} topics", topics.len()),
            };
            if let Ok(json) = serde_json::to_string(&success_msg) {
                if let Err(e) = tx.send(Message::Text(json.into())) {
                    warn!(
                        topic_count = topics.len(),
                        error = ?e,
                        "Failed to send subscription confirmation over WebSocket"
                    );
                }
            }
            topics
        } else {
            let error_msg = WebSocketMessage::Error {
                message: "Authentication required".into(),
            };
            if let Ok(json) = serde_json::to_string(&error_msg) {
                if let Err(e) = tx.send(Message::Text(json.into())) {
                    warn!(
                        error = ?e,
                        "Failed to send authentication required error message over WebSocket"
                    );
                }
            }
            Vec::new()
        }
    }

    /// Handle incoming WebSocket connection
    ///
    /// Registers the client in the shared clients map upon authentication so that
    /// broadcast messages reach connected users in real-time. The client is removed
    /// from the map when the connection closes.
    pub async fn handle_connection(&self, ws: WebSocket) {
        let (mut ws_tx, mut ws_rx) = ws.split();
        let (tx, mut rx) = unbounded_channel();

        let connection_id = Uuid::new_v4();
        let clients = self.clients.clone(); // Safe: Arc clone for async access

        // Spawn task to forward messages to `WebSocket`
        let mut ws_send_task = tokio::spawn(async move {
            while let Some(message) = rx.recv().await {
                if ws_tx.send(message).await.is_err() {
                    break;
                }
            }
        });

        // Bound the connection lifetime: once it elapses the server politely closes
        // the socket so idle/background clients release this instance rather than
        // pinning it open indefinitely through auto-reconnect.
        let max_lifetime = sleep(self.max_connection_duration);
        tokio::pin!(max_lifetime);

        // Handle incoming messages
        loop {
            let msg = tokio::select! {
                () = &mut max_lifetime => {
                    let close = Message::Close(Some(CloseFrame {
                        code: 1000,
                        reason: "max_lifetime".into(),
                    }));
                    if tx.send(close).is_err() {
                        debug!(%connection_id, "WebSocket already closed before max-lifetime close frame");
                    }
                    break;
                }
                msg = ws_rx.next() => match msg {
                    Some(msg) => msg,
                    None => break,
                },
            };
            match msg {
                Ok(Message::Text(text)) => {
                    self.handle_text_frame(text.as_str(), &tx, connection_id, &clients)
                        .await;
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }

        // Clean up on disconnect. Drop every sender (the local handle plus the clone
        // stored in the clients map) so the forwarding task drains any queued frames
        // — notably the max-lifetime close frame — then exits on its own. Awaiting it
        // with a bound guarantees delivery without hanging on a half-dead socket.
        self.clients.remove(&connection_id);
        drop(tx);
        if timeout(WS_DRAIN_TIMEOUT, &mut ws_send_task).await.is_err() {
            debug!(%connection_id, "WebSocket forwarder drain timed out on close; aborting");
            ws_send_task.abort();
        }
    }

    /// Process a single inbound text frame: authenticate, update subscriptions, or
    /// report a parse error back to the client. Extracted from `handle_connection`
    /// to keep that connection loop's control flow within cognitive-complexity limits.
    async fn handle_text_frame(
        &self,
        text: &str,
        tx: &UnboundedSender<Message>,
        connection_id: Uuid,
        clients: &Arc<DashMap<Uuid, ClientConnection>>,
    ) {
        match serde_json::from_str::<WebSocketMessage>(text) {
            Ok(WebSocketMessage::Authentication { token }) => {
                if self.handle_auth_message(&token, tx).await.is_some() {
                    // Register client immediately so broadcasts reach this connection
                    let client = ClientConnection {
                        subscriptions: Vec::new(),
                        tx: tx.clone(), // Safe: mpsc::Sender clone for client storage
                    };
                    clients.insert(connection_id, client);
                }
            }
            Ok(WebSocketMessage::Subscribe { topics }) => {
                let is_authenticated = clients.contains_key(&connection_id);
                let new_subscriptions =
                    Self::handle_subscribe_message(topics, is_authenticated, tx);
                // Update subscriptions in the shared map
                if let Some(mut client) = clients.get_mut(&connection_id) {
                    client.subscriptions = new_subscriptions;
                }
            }
            Err(e) => {
                let error_msg = WebSocketMessage::Error {
                    message: format!("Invalid message format: {e}"),
                };
                if let Ok(json) = serde_json::to_string(&error_msg) {
                    if let Err(send_err) = tx.send(Message::Text(json.into())) {
                        warn!(
                            parse_error = %e,
                            send_error = ?send_err,
                            "Failed to send invalid message format error over WebSocket"
                        );
                    }
                }
            }
            _ => {}
        }
    }

    /// Authenticate `WebSocket` user with JWT
    async fn authenticate_user(&self, token: &str) -> AppResult<AuthResult> {
        let auth_header = if token.starts_with("Bearer ") {
            token.to_owned()
        } else {
            format!("Bearer {token}")
        };

        self.auth_middleware
            .authenticate_request(Some(&auth_header))
            .await
            .map_err(|e| AppError::internal(format!("WebSocket authentication failed: {e}")))
    }

    /// Broadcast system statistics
    ///
    /// # Errors
    ///
    /// Returns an error if:\n    /// - System statistics retrieval fails\n    /// - Message serialization fails\n    /// - Broadcasting to clients fails
    pub async fn broadcast_system_stats(&self) -> AppResult<()> {
        let stats = self
            .get_system_stats()
            .await
            .map_err(|e| AppError::internal(format!("Failed to get system stats: {e}")))?;
        let message = WebSocketMessage::SystemStats {
            total_requests_today: stats.total_requests_today,
            total_requests_this_month: stats.total_requests_this_month,
            active_connections: self.clients.len(),
        };

        self.broadcast_to_all(&message, "system");
        Ok(())
    }

    /// Broadcast message to all subscribers of a topic
    fn broadcast_to_all(&self, message: &WebSocketMessage, topic: &str) {
        // Use broadcast channel for efficient message distribution
        if let Err(e) = self.broadcast_tx.send(message.clone()) {
            // Safe: broadcast channel needs ownership while we reuse message below
            trace!("Failed to send broadcast message: {}", e);
        }

        // Also send directly to subscribed clients for immediate delivery
        for entry in self.clients.iter() {
            let client = entry.value();
            if client.subscriptions.contains(&topic.to_owned()) {
                if let Ok(msg_text) = serde_json::to_string(message) {
                    if let Err(e) = client.tx.send(Message::Text(msg_text.into())) {
                        warn!(
                            topic = %topic,
                            error = ?e,
                            "Failed to broadcast message to client over WebSocket"
                        );
                    }
                }
            }
        }
    }

    /// Get current system statistics
    async fn get_system_stats(&self) -> AppResult<SystemStats> {
        // Query the database for real statistics
        let (today_count, month_count) = self
            .repos
            .usage
            .get_system_stats(None)
            .await
            .map_err(|e| AppError::database(e.to_string()))?;

        debug!(
            "System statistics: {} requests today, {} this month",
            today_count, month_count
        );

        Ok(SystemStats {
            total_requests_today: today_count,
            total_requests_this_month: month_count,
        })
    }

    /// Start background task for periodic updates
    pub fn start_periodic_updates(&self) {
        let manager = self.clone(); // Safe: Arc clone for background task
        tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(30)); // Update every 30 seconds

            loop {
                interval.tick().await;

                // Broadcast system stats
                if let Err(e) = manager.broadcast_system_stats().await {
                    warn!("Failed to broadcast system stats: {}", e);
                }
            }
        });
    }
}

#[derive(Debug)]
struct SystemStats {
    total_requests_today: u64,
    total_requests_this_month: u64,
}
