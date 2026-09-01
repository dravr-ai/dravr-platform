// ABOUTME: Protocol data conversion between different fitness platform formats
// ABOUTME: Transforms data between Strava, Fitbit, and internal universal formats
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Protocol Converter
//!
//! Converts between different protocol formats (MCP, A2A) and the universal
//! format: `mcp_to_universal`, `universal_to_mcp`, `detect_protocol`, and
//! the tool-format helpers. A2A requests are dispatched natively by
//! `pierre_a2a::A2AServer` (no universal-format conversion step).

use crate::protocol::{UniversalRequest, UniversalResponse, UniversalTool};
use crate::protocols::{ProtocolError, ProtocolType};
use pierre_mcp_schema::{Content, Tool, ToolCall, ToolResponse};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt::Write;
use tracing::{debug, warn};

/// Individual activity response from fitness platforms
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityResponse {
    /// Activity identifier
    #[serde(default)]
    pub id: String,
    /// Activity name
    #[serde(default)]
    pub name: String,
    /// Sport/activity type
    #[serde(default)]
    pub sport_type: String,
    /// Distance in meters
    #[serde(default)]
    pub distance_meters: Option<f64>,
    /// Duration in seconds
    #[serde(default)]
    pub duration_seconds: Option<u64>,
}

/// Response containing multiple activities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivitiesResponse {
    /// List of activities
    pub activities: Vec<ActivityResponse>,
}

/// Athlete profile response from fitness platforms
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AthleteResponse {
    /// Athlete identifier
    pub id: u64,
    /// First name
    #[serde(default)]
    pub firstname: String,
    /// Last name
    #[serde(default)]
    pub lastname: String,
    /// Username
    #[serde(default)]
    pub username: String,
}

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
            progress_token: None,
            cancellation_token: None,
            progress_reporter: None,
        }
    }

    /// Convert universal response to MCP format
    #[must_use]
    pub fn universal_to_mcp(response: UniversalResponse) -> ToolResponse {
        if response.success {
            // Generate human-readable content based on the response data
            let result_text = Self::format_response_content(response.result.as_ref());

            ToolResponse {
                content: vec![Content::Text { text: result_text }],
                is_error: false,
                structured_content: response.result,
            }
        } else {
            ToolResponse {
                content: vec![Content::Text {
                    text: format!(
                        "Error: {}",
                        response.error.unwrap_or_else(|| "Unknown error".into())
                    ),
                }],
                is_error: true,
                // S11: preserve the structured error payload on failure too (it
                // carries `error_code`, e.g. `guardian_denied`), so MCP/SSE
                // callers can machine-detect a block instead of only seeing prose.
                structured_content: response.result,
            }
        }
    }

    /// Format response content into human-readable text
    fn format_response_content(result: Option<&Value>) -> String {
        let Some(data) = result else {
            return "No data available".to_owned();
        };

        // Try to deserialize as activities response
        if let Ok(activities_resp) = serde_json::from_value::<ActivitiesResponse>(data.clone()) {
            let count = activities_resp.activities.len();
            let mut text = format!("Retrieved {count} activities:\n\n");

            for (i, activity) in activities_resp.activities.iter().enumerate().take(10) {
                let name = if activity.name.is_empty() {
                    "Unnamed Activity"
                } else {
                    &activity.name
                };
                let activity_type = if activity.sport_type.is_empty() {
                    "Unknown"
                } else {
                    &activity.sport_type
                };
                let distance = activity
                    .distance_meters
                    .map_or_else(|| "N/A".to_owned(), |d| format!("{:.2} km", d / 1000.0));
                let moving_time = activity.duration_seconds.map_or_else(
                    || "N/A".to_owned(),
                    |t| {
                        let hours = t / 3600;
                        let minutes = (t % 3600) / 60;
                        if hours > 0 {
                            format!("{hours}h {minutes}m")
                        } else {
                            format!("{minutes}m")
                        }
                    },
                );
                let activity_id = if activity.id.is_empty() {
                    "unknown"
                } else {
                    &activity.id
                };

                writeln!(
                    &mut text,
                    "{}. {} - {} | {} | {} | ID: {}",
                    i + 1,
                    name,
                    activity_type,
                    distance,
                    moving_time,
                    activity_id
                )
                .unwrap_or_else(|_| warn!("Failed to write activity line"));
            }

            if count > 10 {
                writeln!(&mut text, "\n... and {} more activities", count - 10)
                    .unwrap_or_else(|_| warn!("Failed to write activity count"));
            }

            return text;
        }

        // Try to deserialize as athlete response
        if let Ok(athlete) = serde_json::from_value::<AthleteResponse>(data.clone()) {
            let name = format!("{} {}", athlete.firstname, athlete.lastname)
                .trim()
                .to_owned();
            let username = if athlete.username.is_empty() {
                "N/A"
            } else {
                &athlete.username
            };

            return format!(
                "Athlete Profile:\nName: {name}\nUsername: @{username}\nID: {}",
                athlete.id
            );
        }

        // Default: pretty-print JSON
        serde_json::to_string_pretty(data).unwrap_or_else(|_| "{}".into())
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

    /// Convert tool definition to A2A format
    #[must_use]
    pub fn tool_to_a2a_format(tool: &UniversalTool) -> Value {
        serde_json::json!({
            "name": tool.name,
            "description": tool.description,
            "parameters": {
                "type": "object",
                "properties": {},
                "required": []
            }
        })
    }

    /// Convert tool definition to MCP format
    #[must_use]
    pub fn tool_to_mcp_format(tool: &UniversalTool) -> Tool {
        Tool {
            name: tool.name.clone(), // Safe: String ownership needed for MCP tool schema
            description: tool.description.clone(), // Safe: String ownership for MCP tool schema
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
            annotations: None,
            output_schema: None,
        }
    }
}
