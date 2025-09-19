// ABOUTME: Server lifecycle management and coordination for multi-tenant MCP server
// ABOUTME: Handles server startup, transport coordination, and notification forwarding

use super::{
    mcp_request_processor::McpRequestProcessor,
    resources::ServerResources,
    transport_manager::TransportManager,
};
use super::multitenant::{McpRequest, McpResponse, OAuthCompletedNotification};
use super::sse_transport;
use anyhow::Result;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;
use tracing::{debug, info};

/// Manages server lifecycle, startup, and transport coordination
pub struct ServerLifecycle {
    resources: Arc<ServerResources>,
}

impl ServerLifecycle {
    /// Create a new server lifecycle manager
    pub fn new(resources: Arc<ServerResources>) -> Self {
        Self { resources }
    }

    /// Start unified server (HTTP + MCP) on specified port
    pub async fn run_unified_server(self, port: u16) -> Result<()> {
        let transport_manager = TransportManager::new(self.resources);
        transport_manager.start_all_transports(port).await
    }

    /// Run MCP server with only HTTP transport (for testing)
    pub async fn run_http_only(self, port: u16) -> Result<()> {
        info!(
            "Starting MCP server with HTTP transport only on port {}",
            port
        );

        self.run_http_server_with_resources(port, self.resources)
            .await
    }

    /// Run HTTP server with shared resources
    async fn run_http_server_with_resources(
        self,
        port: u16,
        resources: Arc<ServerResources>,
    ) -> Result<()> {
        use warp::Filter;

        let routes = Self::build_http_routes(resources);

        info!("Starting HTTP server on port {}", port);
        warp::serve(routes)
            .run(([0, 0, 0, 0], port))
            .await;

        Ok(())
    }

    /// Build HTTP routes for the server
    fn build_http_routes(
        resources: Arc<ServerResources>,
    ) -> impl warp::Filter<Extract = impl warp::Reply> + Clone {
        use warp::Filter;

        // Basic health check route
        let health = warp::path("health")
            .and(warp::get())
            .map(|| warp::reply::with_status("OK", warp::http::StatusCode::OK));

        // MCP endpoint route
        let mcp_routes = Self::build_mcp_routes(resources.clone());

        // OAuth routes
        let oauth_routes = Self::build_oauth_routes(resources);

        health.or(mcp_routes).or(oauth_routes)
    }

    /// Build MCP-specific routes
    fn build_mcp_routes(
        resources: Arc<ServerResources>,
    ) -> impl warp::Filter<Extract = impl warp::Reply> + Clone {
        use warp::Filter;

        warp::path("mcp")
            .and(warp::post())
            .and(warp::body::json())
            .and(warp::any().map(move || resources.clone()))
            .and_then(Self::handle_mcp_request)
    }

    /// Build OAuth-specific routes
    fn build_oauth_routes(
        resources: Arc<ServerResources>,
    ) -> impl warp::Filter<Extract = impl warp::Reply> + Clone {
        use warp::Filter;

        warp::path("oauth")
            .and(warp::path("authorize"))
            .and(warp::path::param())
            .and(warp::path::param())
            .and(warp::get())
            .and(warp::header::headers_cloned())
            .and(warp::any().map(move || resources.clone()))
            .and_then(Self::handle_oauth_authorize)
    }

    /// Handle MCP request via HTTP
    async fn handle_mcp_request(
        body: serde_json::Value,
        resources: Arc<ServerResources>,
    ) -> Result<impl warp::Reply, warp::Rejection> {
        let request: McpRequest = serde_json::from_value(body)
            .map_err(|e| {
                warp::reject::custom(ServerError::InvalidRequest(format!("Invalid JSON: {}", e)))
            })?;

        let processor = McpRequestProcessor::new(resources);
        if let Some(response) = processor.handle_request(request).await {
            let json = serde_json::to_string(&response)
                .map_err(|e| warp::reject::custom(ServerError::SerializationError(e)))?;
            Ok(warp::reply::with_header(
                json,
                "content-type",
                "application/json",
            ))
        } else {
            // No response for notifications
            Ok(warp::reply::with_header(
                "",
                "content-type",
                "application/json",
            ))
        }
    }

    /// Handle OAuth authorization request
    async fn handle_oauth_authorize(
        provider: String,
        user_id: String,
        headers: warp::http::HeaderMap,
        resources: Arc<ServerResources>,
    ) -> Result<impl warp::Reply, warp::Rejection> {
        // Delegate to OAuth flow manager
        super::oauth_flow_manager::OAuthFlowManager::new(resources)
            .handle_authorization_request(provider, user_id, headers)
            .await
    }

    /// Run stdio transport for MCP communication
    pub async fn run_stdio_transport(
        self,
        notification_receiver: tokio::sync::broadcast::Receiver<OAuthCompletedNotification>,
    ) -> Result<()> {
        use tokio::io::{AsyncBufReadExt, BufReader};

        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin).lines();
        let stdout = Arc::new(tokio::sync::Mutex::new(tokio::io::stdout()));

        // Spawn notification handler
        let notification_stdout = stdout.clone();
        let mut notification_rx = notification_receiver;
        tokio::spawn(async move {
            while let Ok(notification) = notification_rx.recv().await {
                Self::handle_oauth_notification(notification, &notification_stdout).await;
            }
        });

        info!("MCP stdio transport started");

        // Process stdin requests
        while let Some(line) = reader.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<McpRequest>(&line) {
                Ok(request) => {
                    Self::process_mcp_request(request, &self.resources, &stdout).await?;
                }
                Err(e) => {
                    debug!("Failed to parse MCP request: {} - Line: {}", e, line);
                }
            }
        }

        Ok(())
    }

    /// Run SSE notification forwarder
    pub async fn run_sse_notification_forwarder(
        &self,
        mut notification_receiver: tokio::sync::broadcast::Receiver<OAuthCompletedNotification>,
        sse_manager: Arc<crate::notifications::sse::SseConnectionManager>,
    ) -> Result<()> {
        info!("Starting SSE notification forwarder");

        while let Ok(notification) = notification_receiver.recv().await {
            sse_transport::forward_oauth_notification(&notification, &sse_manager).await?;
        }

        Ok(())
    }

    /// Handle OAuth completion notification
    async fn handle_oauth_notification(
        notification: OAuthCompletedNotification,
        stdout: &Arc<tokio::sync::Mutex<tokio::io::Stdout>>,
    ) {
        let notification_msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/oauth_completed",
            "params": {
                "provider": notification.provider,
                "user_id": notification.user_id,
                "success": notification.success,
                "message": notification.message
            }
        });

        if let Ok(json) = serde_json::to_string(&notification_msg) {
            let mut stdout_lock = stdout.lock().await;
            let _ = stdout_lock.write_all(json.as_bytes()).await;
            let _ = stdout_lock.write_all(b"\n").await;
            let _ = stdout_lock.flush().await;
        }
    }

    /// Write MCP response to stdout
    async fn write_response_to_stdout(
        response: &McpResponse,
        stdout: &Arc<tokio::sync::Mutex<tokio::io::Stdout>>,
    ) -> Result<()> {
        let response_json = serde_json::to_string(response)?;
        debug!("Sending MCP response: {}", response_json);

        let mut stdout_lock = stdout.lock().await;
        stdout_lock.write_all(response_json.as_bytes()).await?;
        stdout_lock.write_all(b"\n").await?;
        stdout_lock.flush().await?;

        Ok(())
    }

    /// Process MCP request and send response
    async fn process_mcp_request(
        request: McpRequest,
        resources: &Arc<ServerResources>,
        stdout: &Arc<tokio::sync::Mutex<tokio::io::Stdout>>,
    ) -> Result<()> {
        debug!(
            "Processing MCP request: method={}, id={:?}",
            request.method, request.id
        );

        let processor = McpRequestProcessor::new(resources.clone());
        if let Some(response) = processor.handle_request(request).await {
            Self::write_response_to_stdout(&response, stdout).await?;
        }

        Ok(())
    }
}

/// Server error types
#[derive(Debug)]
pub enum ServerError {
    InvalidRequest(String),
    SerializationError(serde_json::Error),
}

impl warp::reject::Reject for ServerError {}

impl std::fmt::Display for ServerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServerError::InvalidRequest(msg) => write!(f, "Invalid request: {}", msg),
            ServerError::SerializationError(e) => write!(f, "Serialization error: {}", e),
        }
    }
}

impl std::error::Error for ServerError {}