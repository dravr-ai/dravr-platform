// ABOUTME: Protocol data conversion between different fitness platform formats
// ABOUTME: Transforms data between Strava and internal universal formats
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Protocol Converter
//!
//! Converts between different protocol formats (MCP, A2A) and the universal
//! format: `mcp_to_universal`, `universal_to_mcp`, `detect_protocol`, and
//! the tool-format helpers. A2A requests are dispatched natively by
//! `pierre_a2a::A2AServer` (no universal-format conversion step).

use crate::protocol::{UniversalRequest, UniversalResponse};
use crate::protocols::{ProtocolError, ProtocolType};
use pierre_mcp_schema::{Content, ToolCall, ToolResponse};
use serde_json::Value;
use tracing::debug;

/// Protocol converter for translating between protocol formats
pub struct ProtocolConverter;

impl ProtocolConverter {
    /// Convert MCP tool call to universal format
    #[must_use]
    pub fn mcp_to_universal(
        tool_call: ToolCall,
        user_id: &str,
        tenant_id: Option<String>,
    ) -> UniversalRequest {
        UniversalRequest {
            tool_name: tool_call.name,
            parameters: tool_call
                .arguments
                .unwrap_or_else(|| Value::Object(serde_json::Map::new())),
            user_id: user_id.to_owned(),
            protocol: "mcp".into(),
            tenant_id,
        }
    }

    /// Convert universal response to MCP format
    ///
    /// A failure is rendered by [`Self::error_result`]: its payload — the
    /// `error_code` a Guardian block or a tenant disable carries, the failure
    /// a tool chose to report as data — reaches the client as JSON in
    /// `content`, never as `structuredContent`.
    #[must_use]
    pub fn universal_to_mcp(response: UniversalResponse) -> ToolResponse {
        if response.success {
            // Generate human-readable content based on the response data
            let result_text = Self::format_response_content(response.result.as_ref());

            ToolResponse {
                content: vec![Content::text(result_text)],
                is_error: false,
                structured_content: response.result,
            }
        } else {
            Self::error_result(
                format!(
                    "Error: {}",
                    response.error.unwrap_or_else(|| "Unknown error".into())
                ),
                response.result,
            )
        }
    }

    /// Render a refusal as the `isError` result a `tools/call` answers with.
    ///
    /// `message` is the first text block. `data`, the refusal's
    /// machine-readable part — a quota refusal's counter, reset instant and
    /// wait, a Guardian block's `error_code` and `reason`, a provider
    /// reconnect's `error_code` and `provider` — follows as a second text
    /// block holding its JSON, so a client parses `content[1]` to act on it.
    ///
    /// Never as `structuredContent`. MCP requires a tool that declares an
    /// `outputSchema` to answer with `structuredContent` conforming to it and
    /// makes no exception for `isError` results, and the official TypeScript
    /// SDK's `Client.callTool` validates it on an error result too. A refusal
    /// payload matches no tool's answer schema, so carried there it turns the
    /// refusal into a `-32602` protocol error and the client never reads the
    /// reason. Most tools declare a schema, and the refusal paths are shared by
    /// all of them, so an error result carries no structured part at all.
    #[must_use]
    pub fn error_result(message: String, data: Option<Value>) -> ToolResponse {
        let mut content = vec![Content::text(message)];
        if let Some(data) = data.filter(|data| !data.is_null()) {
            content.push(Content::text(data.to_string()));
        }
        ToolResponse {
            content,
            is_error: true,
            structured_content: None,
        }
    }

    /// The text block a `tools/call` result carries beside `structuredContent`.
    ///
    /// The payload's JSON, compact, for every tool. MCP asks a tool that
    /// returns `structuredContent` to repeat it as serialized JSON in a text
    /// block for clients that read only `content` (the Claude API's MCP
    /// connector passes only `content` to the model), and Claude Code passes
    /// only `structuredContent`. Repeating the JSON shows both kinds of client
    /// the same reply, every item and every caveat the payload carries, where
    /// composed prose would show one of them a summary. The JSON is also where
    /// a provider's text is safe, as a quoted string value.
    fn format_response_content(result: Option<&Value>) -> String {
        result.map_or_else(
            || "No data available".to_owned(),
            |data| serde_json::to_string(data).unwrap_or_else(|_| "{}".into()),
        )
    }

    /// Detect protocol type from request format
    ///
    /// # Errors
    ///
    /// Returns an error if the request data is not valid JSON or if the protocol type cannot be determined.
    pub fn detect_protocol(request_data: &str) -> Result<ProtocolType, ProtocolError> {
        // Try to parse as JSON first
        let json: Value = serde_json::from_str(request_data).map_err(|e| {
            debug!(error = %e, "Failed to parse request as JSON during protocol detection");
            ProtocolError::SerializationError("Invalid JSON during protocol detection".into())
        })?;

        // Check for A2A indicators: the A2A 1.0 JSON-RPC method names are
        // PascalCase (identical to the gRPC method names).
        if json.get("jsonrpc").is_some() && json.get("method").is_some() {
            if let Some(method) = json.get("method").and_then(|m| m.as_str()) {
                const A2A_METHODS: [&str; 11] = [
                    "SendMessage",
                    "SendStreamingMessage",
                    "GetTask",
                    "ListTasks",
                    "CancelTask",
                    "SubscribeToTask",
                    "CreateTaskPushNotificationConfig",
                    "GetTaskPushNotificationConfig",
                    "ListTaskPushNotificationConfigs",
                    "DeleteTaskPushNotificationConfig",
                    "GetExtendedAgentCard",
                ];
                if A2A_METHODS.contains(&method) {
                    return Ok(ProtocolType::A2A);
                }
            }
        }

        // Check for MCP indicators
        if json.get("method").is_some() {
            if let Some(method) = json.get("method").and_then(|m| m.as_str()) {
                if method == "tools/call" || method == "initialize" {
                    return Ok(ProtocolType::MCP);
                }
            }
        }

        Err(ProtocolError::InvalidRequest(
            "Unknown protocol format".into(),
        ))
    }
}
