// ABOUTME: MCP request processing and protocol handling for multi-tenant server
// ABOUTME: Validates, routes, and executes MCP protocol requests with proper error handling

use super::{
    resources::ServerResources,
    tool_handlers::{ToolHandlers, ToolRoutingContext},
    multitenant::{McpRequest, McpResponse, McpError},
};
use crate::constants::errors::{ERROR_INTERNAL_ERROR, ERROR_INVALID_PARAMS, ERROR_METHOD_NOT_FOUND};
use crate::constants::protocol::JSONRPC_VERSION;
use anyhow::Result;
use serde_json::Value;
use std::sync::Arc;
use tracing::{debug, error, warn};

/// Processes MCP protocol requests with validation, routing, and execution
pub struct McpRequestProcessor {
    resources: Arc<ServerResources>,
}

impl McpRequestProcessor {
    /// Create a new MCP request processor
    pub fn new(resources: Arc<ServerResources>) -> Self {
        Self { resources }
    }

    /// Handle an MCP request and return a response
    pub async fn handle_request(&self, request: McpRequest) -> Option<McpResponse> {
        let start_time = std::time::Instant::now();

        // Log request with optional truncation
        self.log_request(&request);

        // Handle notifications (no response needed)
        if request.method.starts_with("notifications/") {
            self.handle_notification(&request);
            self.log_completion("notification", start_time);
            return None;
        }

        // Process request and generate response
        let response = match self.process_request(request).await {
            Ok(response) => response,
            Err(e) => {
                error!("Failed to process MCP request: {}", e);
                McpResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: None,
                    result: None,
                    error: Some(McpError {
                        code: ERROR_INTERNAL_ERROR,
                        message: "Internal server error".to_string(),
                        data: None,
                    }),
                }
            }
        };

        self.log_completion("request", start_time);
        Some(response)
    }

    /// Process an MCP request and generate response
    async fn process_request(&self, request: McpRequest) -> Result<McpResponse> {
        // Validate request format
        self.validate_request(&request)?;

        // Route to appropriate handler based on method
        match request.method.as_str() {
            "initialize" => self.handle_initialize(&request).await,
            "ping" => self.handle_ping(&request).await,
            "tools/list" => self.handle_tools_list(&request).await,
            "tools/call" => self.handle_tools_call(&request).await,
            method if method.starts_with("resources/") => self.handle_resources(&request).await,
            method if method.starts_with("prompts/") => self.handle_prompts(&request).await,
            _ => self.handle_unknown_method(&request),
        }
    }

    /// Validate MCP request format and required fields
    fn validate_request(&self, request: &McpRequest) -> Result<()> {
        if request.jsonrpc != JSONRPC_VERSION {
            return Err(anyhow::anyhow!("Invalid JSON-RPC version"));
        }

        if request.method.is_empty() {
            return Err(anyhow::anyhow!("Missing method"));
        }

        Ok(())
    }

    /// Handle MCP initialize request
    async fn handle_initialize(&self, request: &McpRequest) -> Result<McpResponse> {
        debug!("Handling initialize request");

        let server_info = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {
                    "listChanged": true
                },
                "resources": {
                    "subscribe": true,
                    "listChanged": true
                },
                "prompts": {
                    "listChanged": true
                }
            },
            "serverInfo": {
                "name": "pierre-mcp-server",
                "version": env!("CARGO_PKG_VERSION")
            }
        });

        Ok(McpResponse {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: request.id.clone(),
            result: Some(server_info),
            error: None,
        })
    }

    /// Handle MCP ping request
    async fn handle_ping(&self, request: &McpRequest) -> Result<McpResponse> {
        debug!("Handling ping request");

        Ok(McpResponse {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: request.id.clone(),
            result: Some(serde_json::json!({})),
            error: None,
        })
    }

    /// Handle tools/list request
    async fn handle_tools_list(&self, request: &McpRequest) -> Result<McpResponse> {
        debug!("Handling tools/list request");

        let tool_handlers = ToolHandlers::new(self.resources.clone());
        let tools = tool_handlers.list_tools().await;

        Ok(McpResponse {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: request.id.clone(),
            result: Some(serde_json::json!({ "tools": tools })),
            error: None,
        })
    }

    /// Handle tools/call request
    async fn handle_tools_call(&self, request: &McpRequest) -> Result<McpResponse> {
        debug!("Handling tools/call request");

        let params = request.params.as_ref().ok_or_else(|| {
            anyhow::anyhow!("Missing parameters for tools/call")
        })?;

        let tool_name = params.get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing tool name"))?;

        let arguments = params.get("arguments")
            .cloned()
            .unwrap_or_default();

        // Create routing context from request
        let context = self.create_routing_context(request)?;

        // Execute tool
        let tool_handlers = ToolHandlers::new(self.resources.clone());
        match tool_handlers.call_tool(tool_name, arguments, context).await {
            Ok(result) => Ok(McpResponse {
                jsonrpc: JSONRPC_VERSION.to_string(),
                id: request.id.clone(),
                result: Some(result),
                error: None,
            }),
            Err(e) => {
                error!("Tool execution failed: {}", e);
                Ok(McpResponse {
                    jsonrpc: JSONRPC_VERSION.to_string(),
                    id: request.id.clone(),
                    result: None,
                    error: Some(McpError {
                        code: ERROR_INTERNAL_ERROR,
                        message: format!("Tool execution failed: {}", e),
                        data: None,
                    }),
                })
            }
        }
    }

    /// Handle resources requests
    async fn handle_resources(&self, request: &McpRequest) -> Result<McpResponse> {
        debug!("Handling resources request: {}", request.method);

        // TODO: Implement resource handling
        Ok(McpResponse {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: request.id.clone(),
            result: Some(serde_json::json!({ "resources": [] })),
            error: None,
        })
    }

    /// Handle prompts requests
    async fn handle_prompts(&self, request: &McpRequest) -> Result<McpResponse> {
        debug!("Handling prompts request: {}", request.method);

        // TODO: Implement prompt handling
        Ok(McpResponse {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: request.id.clone(),
            result: Some(serde_json::json!({ "prompts": [] })),
            error: None,
        })
    }

    /// Handle unknown method
    fn handle_unknown_method(&self, request: &McpRequest) -> Result<McpResponse> {
        warn!("Unknown MCP method: {}", request.method);

        Ok(McpResponse {
            jsonrpc: JSONRPC_VERSION.to_string(),
            id: request.id.clone(),
            result: None,
            error: Some(McpError {
                code: ERROR_METHOD_NOT_FOUND,
                message: format!("Method not found: {}", request.method),
                data: None,
            }),
        })
    }

    /// Create routing context from request
    fn create_routing_context(&self, request: &McpRequest) -> Result<ToolRoutingContext> {
        // Extract user context from auth token if present
        let (user_id, tenant_id) = if let Some(auth_token) = &request.auth_token {
            match self.extract_user_context(auth_token) {
                Ok((user_id, tenant_id)) => (Some(user_id), Some(tenant_id)),
                Err(e) => {
                    warn!("Failed to extract user context from auth token: {}", e);
                    (None, None)
                }
            }
        } else {
            (None, None)
        };

        Ok(ToolRoutingContext {
            user_id,
            tenant_id,
            protocol: "mcp".to_string(),
            request_id: request.id.clone(),
        })
    }

    /// Extract user context from auth token
    fn extract_user_context(&self, auth_token: &str) -> Result<(uuid::Uuid, uuid::Uuid)> {
        // TODO: Implement JWT token validation and extraction
        // This would involve validating the JWT and extracting user_id and tenant_id
        Err(anyhow::anyhow!("Auth token validation not implemented yet"))
    }

    /// Handle notification (no response required)
    fn handle_notification(&self, request: &McpRequest) {
        debug!("Handling notification: {}", request.method);

        match request.method.as_str() {
            "notifications/cancelled" => {
                debug!("Request cancelled notification received");
            }
            "notifications/progress" => {
                debug!("Progress notification received");
            }
            _ => {
                debug!("Unknown notification type: {}", request.method);
            }
        }
    }

    /// Log incoming request with optional truncation
    fn log_request(&self, request: &McpRequest) {
        let should_truncate = std::env::var("MCP_LOG_TRUNCATE")
            .map(|v| v != "false" && v != "0")
            .unwrap_or(true);

        if should_truncate {
            let request_summary = format!("{}(id={:?})", request.method, request.id);
            debug!(
                mcp_method = %request.method,
                mcp_id = ?request.id,
                mcp_params_preview = ?request.params.as_ref().map(|p| {
                    let s = p.to_string();
                    if s.len() > 100 {
                        format!("{}...[truncated]", &s[..100])
                    } else {
                        s
                    }
                }),
                auth_present = request.auth_token.is_some(),
                "Received MCP request: {}",
                request_summary
            );
        } else {
            debug!(
                mcp_request = ?request,
                "Received MCP request (full)"
            );
        }
    }

    /// Log request completion with timing
    fn log_completion(&self, request_type: &str, start_time: std::time::Instant) {
        let duration = start_time.elapsed();
        debug!(
            duration_ms = u64::try_from(duration.as_millis()).unwrap_or(0),
            "Completed MCP {} processing",
            request_type
        );
    }
}

/// Write MCP response to stdout for stdio transport
pub async fn write_response_to_stdout(
    response: &McpResponse,
    stdout: &Arc<tokio::sync::Mutex<tokio::io::Stdout>>,
) -> Result<()> {
    use tokio::io::AsyncWriteExt;

    let response_json = serde_json::to_string(response)?;
    debug!("Sending MCP response: {}", response_json);

    let mut stdout_lock = stdout.lock().await;
    stdout_lock.write_all(response_json.as_bytes()).await?;
    stdout_lock.write_all(b"\n").await?;
    stdout_lock.flush().await?;

    Ok(())
}