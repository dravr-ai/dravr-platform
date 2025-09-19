// ABOUTME: Transport coordination for MCP server with stdio, HTTP, and SSE transports
// ABOUTME: Manages notification channels and coordinates multiple transport methods

use super::resources::ServerResources;
use crate::mcp::schema::OAuthCompletedNotification;
use anyhow::Result;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{error, info, warn};

/// Manages multiple transport methods for MCP communication
pub struct TransportManager {
    resources: Arc<ServerResources>,
    notification_sender: broadcast::Sender<OAuthCompletedNotification>,
}

impl TransportManager {
    /// Create a new transport manager with shared resources
    pub fn new(resources: Arc<ServerResources>) -> Self {
        let (notification_sender, _) = broadcast::channel(100);
        Self {
            resources,
            notification_sender,
        }
    }

    /// Start all transport methods (stdio, HTTP, SSE) in coordinated fashion
    pub async fn start_all_transports(&self, port: u16) -> Result<()> {
        info!("Transport manager coordinating all transports on port {}", port);

        // For now, delegate to the existing implementation
        // TODO: Move the actual transport coordination logic here
        self.start_legacy_unified_server(port).await
    }

    /// Temporary method that delegates to existing transport coordination
    /// TODO: Replace this with proper transport management
    async fn start_legacy_unified_server(&self, port: u16) -> Result<()> {
        // This is the original complex logic - we'll move it here piece by piece
        info!("Starting MCP server with stdio and HTTP transports");

        // Create notification channels for both transports using broadcast for multiple receivers
        let (notification_sender, _): (
            tokio::sync::broadcast::Sender<crate::mcp::schema::OAuthCompletedNotification>,
            tokio::sync::broadcast::Receiver<crate::mcp::schema::OAuthCompletedNotification>,
        ) = tokio::sync::broadcast::channel(100); // Buffer up to 100 notifications

        // Create separate receivers for stdio and SSE
        let notification_receiver = notification_sender.subscribe();
        let sse_notification_receiver = notification_sender.subscribe();

        // Set up notification sender in resources for OAuth callbacks
        let mut resources_clone = (*self.resources).clone();
        resources_clone.set_oauth_notification_sender(notification_sender);
        let shared_resources = Arc::new(resources_clone);

        // Start stdio transport in background
        let resources_for_stdio = shared_resources.clone();
        let stdio_handle = tokio::spawn(async move {
            let stdio_transport = StdioTransport::new(resources_for_stdio);
            match stdio_transport.run(notification_receiver).await {
                Ok(()) => info!("stdio transport completed successfully"),
                Err(e) => warn!("stdio transport failed: {}", e),
            }
        });

        // Monitor stdio transport in background
        tokio::spawn(async move {
            match stdio_handle.await {
                Ok(()) => info!("stdio transport task completed"),
                Err(e) => warn!("stdio transport task failed: {}", e),
            }
        });

        // Start SSE notification forwarder task
        let resources_for_sse = shared_resources.clone();
        let sse_manager_for_notifications = resources_for_sse.sse_manager.clone();
        tokio::spawn(async move {
            let sse_forwarder = SseNotificationForwarder::new(resources_for_sse);
            if let Err(e) = sse_forwarder
                .run(sse_notification_receiver, sse_manager_for_notifications)
                .await
            {
                error!("SSE notification forwarder failed: {}", e);
            }
        });

        // Run unified HTTP server with all routes (OAuth2, MCP, etc.) - this should run indefinitely
        loop {
            info!("Starting unified HTTP server on port {}", port);

            // Clone shared resources for each iteration since run_http_server_with_resources takes ownership
            match super::multitenant::MultiTenantMcpServer::run_http_server_with_resources(port, shared_resources.clone()).await {
                Ok(()) => {
                    error!("HTTP server unexpectedly completed - this should never happen");
                    error!("HTTP server should run indefinitely. Restarting in 5 seconds...");
                    tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                }
                Err(e) => {
                    error!("HTTP server failed: {}", e);
                    error!("Restarting HTTP server in 10 seconds...");
                    tokio::time::sleep(tokio::time::Duration::from_secs(10)).await;
                }
            }
        }
    }

}

/// Handles stdio transport for MCP communication
pub struct StdioTransport {
    resources: Arc<ServerResources>,
}

impl StdioTransport {
    pub fn new(resources: Arc<ServerResources>) -> Self {
        Self { resources }
    }

    pub async fn run(
        &self,
        notification_receiver: broadcast::Receiver<OAuthCompletedNotification>,
    ) -> Result<()> {
        use tokio::io::{AsyncBufReadExt, BufReader};

        info!("MCP stdio transport ready - listening on stdin/stdout");

        let stdin = tokio::io::stdin();
        let mut lines = BufReader::new(stdin).lines();

        // Spawn notification handler for stdio transport
        let resources_for_notifications = self.resources.clone();
        let notification_handle = tokio::spawn(async move {
            Self::handle_stdio_notifications(notification_receiver, resources_for_notifications)
                .await
        });

        // Main stdio loop
        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }

            match self.process_stdio_line(&line).await {
                Ok(response) => {
                    if let Some(resp) = response {
                        println!("{}", resp);
                    }
                }
                Err(e) => {
                    warn!("Error processing stdio input: {}", e);
                    let error_response = serde_json::json!({
                        "jsonrpc": "2.0",
                        "error": {
                            "code": -32603,
                            "message": "Internal error"
                        },
                        "id": null
                    });
                    println!("{}", error_response);
                }
            }
        }

        // Clean up notification handler
        notification_handle.abort();
        Ok(())
    }

    async fn process_stdio_line(&self, line: &str) -> Result<Option<String>> {
        // Parse JSON-RPC request
        let _request: serde_json::Value = serde_json::from_str(line)?;

        // TODO: Process MCP request (this will be moved to McpRequestProcessor)
        // For now, return a simple response
        let response = serde_json::json!({
            "jsonrpc": "2.0",
            "result": "transport_manager_active",
            "id": 1
        });

        Ok(Some(serde_json::to_string(&response)?))
    }

    async fn handle_stdio_notifications(
        mut receiver: broadcast::Receiver<OAuthCompletedNotification>,
        _resources: Arc<ServerResources>,
    ) -> Result<()> {
        info!("Stdio notification handler ready");

        while let Ok(notification) = receiver.recv().await {
            info!("Received OAuth notification for stdio: {:?}", notification);
            // Send notification to stdio client
            let notification_json = serde_json::to_string(&notification)?;
            println!("{}", notification_json);
        }

        Ok(())
    }
}

/// Handles SSE notification forwarding
pub struct SseNotificationForwarder {
    resources: Arc<ServerResources>,
}

impl SseNotificationForwarder {
    pub fn new(resources: Arc<ServerResources>) -> Self {
        Self { resources }
    }

    pub async fn run(
        &self,
        mut notification_receiver: broadcast::Receiver<OAuthCompletedNotification>,
        sse_manager: Arc<crate::notifications::sse::SseConnectionManager>,
    ) -> Result<()> {
        info!("SSE notification forwarder ready - waiting for OAuth notifications");

        loop {
            match notification_receiver.recv().await {
                Ok(notification) => {
                    info!("Forwarding OAuth notification to SSE clients: {:?}", notification);

                    let notification_json = serde_json::to_value(&notification)?;
                    sse_manager
                        .send_to_all(notification_json)
                        .await
                        .unwrap_or_else(|e| {
                            warn!("Failed to send SSE notification: {}", e);
                        });
                }
                Err(broadcast::error::RecvError::Closed) => {
                    info!("OAuth notification channel closed, shutting down SSE forwarder");
                    break;
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    warn!("SSE notification forwarder lagged, skipped {} notifications", skipped);
                    continue;
                }
            }
        }

        Ok(())
    }
}