// ABOUTME: Data access tools implementing the McpTool trait as wrappers.
// ABOUTME: Delegates to existing handlers for get_activities, get_athlete, get_stats.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Data Access Tools
//!
//! This module contains tools for accessing fitness data:
//! - `GetActivitiesTool` - Retrieve user activities with filtering and pagination
//! - `GetAthleteTool` - Get athlete profile information
//! - `GetStatsTool` - Get aggregated activity statistics
//! - `GetSleepSessionsTool` - Query stored sleep sessions
//! - `GetRecoveryMetricsTool` - Query stored recovery and readiness metrics
//! - `GetHealthSnapshotsTool` - Query stored health snapshots (body composition, vitals)
//! - `ListDataSourcesTool` - List connected data sources (devices and providers)
//!
//! These tools wrap the universal protocol handlers and expose them via the
//! `McpTool` interface.

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::Value;

use crate::errors::AppResult;
use crate::mcp::schema::{JsonSchema, PropertySchema, ToolAnnotations};
use crate::protocols::universal::handlers;
use crate::tools::context::ToolExecutionContext;
use crate::tools::result::ToolResult;
use crate::tools::traits::{McpTool, ToolCapabilities};
use crate::tools::universal_delegate::delegate_to_handler;

/// Annotations for read-only data retrieval tools
fn read_only_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

// ============================================================================
// GetActivitiesTool - Retrieve user activities
// ============================================================================

/// Tool for retrieving user activities from fitness providers.
///
/// Supports filtering by sport type, date ranges, pagination, and
/// different output modes (summary/detailed) and formats (json/toon).
pub struct GetActivitiesTool;

#[async_trait]
impl McpTool for GetActivitiesTool {
    fn name(&self) -> &'static str {
        "get_activities"
    }

    fn description(&self) -> &'static str {
        "Retrieve user's fitness activities from connected providers with optional filtering by sport type, date range, and pagination support"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();

        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Fitness provider to query (e.g., 'strava', 'fitbit'). Defaults to configured default provider.".to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "limit".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some(
                    "Maximum number of activities to return (1-400). Use the smallest value that answers the question: 1 for 'last activity', 5-10 for 'this week', 20 for broader queries. Response includes has_more and pagination info for follow-up requests.".to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "offset".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Number of activities to skip for pagination.".to_owned()),
                ..Default::default()
            },
        );

        properties.insert(
            "sport_type".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Filter by sport type (e.g., 'run', 'ride', 'swim'). Case-insensitive."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "before".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some(
                    "Unix timestamp - return activities before this time.".to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "after".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Unix timestamp - return activities after this time.".to_owned()),
                ..Default::default()
            },
        );

        properties.insert(
            "mode".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Output mode: 'summary' (default, minimal fields) or 'detailed' (full activity data).".to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "format".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Output format: 'json' (default) or 'toon' (token-efficient for LLMs)."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None, // All parameters are optional
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    #[tracing::instrument(
        skip(self, args, context),
        fields(
            tool = "get_activities",
            user_id = %context.user_id,
            tenant_id = tracing::field::Empty,
            provider = tracing::field::Empty,
        )
    )]
    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        // Provider arg is optional — when omitted the universal handler
        // falls back to the user's configured default, so log "default"
        // rather than emit an empty field.
        let provider = args
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or("default");
        let span = tracing::Span::current();
        span.record("provider", tracing::field::display(&provider));
        if let Some(tenant_id) = context.tenant_id {
            span.record("tenant_id", tracing::field::display(&tenant_id));
        }

        // notify: chat-triggered fetch of activities from a fitness provider.
        // The LLM invoking this tool counts as user_initiated — the user
        // asked the question that prompted the tool call.
        tracing::info!(
            target: "notify",
            event = "provider.fetch_started",
            provider = %provider,
            trigger = "user_initiated",
            "fetching activities from provider"
        );

        delegate_to_handler(
            context,
            args,
            "get_activities",
            handlers::handle_get_activities,
        )
        .await
    }
}

// ============================================================================
// GetAthleteTool - Get athlete profile
// ============================================================================

/// Tool for retrieving the user's athlete profile from a fitness provider.
pub struct GetAthleteTool;

#[async_trait]
impl McpTool for GetAthleteTool {
    fn name(&self) -> &'static str {
        "get_athlete"
    }

    fn description(&self) -> &'static str {
        "Retrieve the user's athlete profile from connected fitness providers including personal details and preferences"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();

        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Fitness provider to query (e.g., 'strava', 'fitbit'). Defaults to configured default provider.".to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "format".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Output format: 'json' (default) or 'toon' (token-efficient for LLMs)."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        delegate_to_handler(context, args, "get_athlete", handlers::handle_get_athlete).await
    }
}

// ============================================================================
// GetStatsTool - Get activity statistics
// ============================================================================

/// Tool for retrieving aggregated activity statistics from a fitness provider.
pub struct GetStatsTool;

#[async_trait]
impl McpTool for GetStatsTool {
    fn name(&self) -> &'static str {
        "get_stats"
    }

    fn description(&self) -> &'static str {
        "Retrieve aggregated activity statistics from connected fitness providers including totals, records, and year-to-date metrics"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();

        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Fitness provider to query (e.g., 'strava', 'fitbit'). Defaults to configured default provider.".to_owned(),
                ),
                ..Default::default()
            },
        );

        properties.insert(
            "format".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Output format: 'json' (default) or 'toon' (token-efficient for LLMs)."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        delegate_to_handler(context, args, "get_stats", handlers::handle_get_stats).await
    }
}

// ============================================================================
// Helper - shared schema builders for stored health-data tools
// ============================================================================

/// Build the standard date-range + format property set used by stored
/// health-data queries (sleep, recovery, snapshots). Inferred from the
/// handler bodies in `handlers/health_data.rs`, which read `start`, `end`,
/// and `format` from `request.parameters`.
fn date_range_properties() -> HashMap<String, PropertySchema> {
    let mut properties = HashMap::new();
    properties.insert(
        "start".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "Start of the date range as RFC3339 timestamp. Defaults to 30 days ago.".to_owned(),
            ),
            ..Default::default()
        },
    );
    properties.insert(
        "end".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "End of the date range as RFC3339 timestamp. Defaults to now.".to_owned(),
            ),
            ..Default::default()
        },
    );
    properties.insert(
        "format".to_owned(),
        PropertySchema {
            property_type: "string".to_owned(),
            description: Some(
                "Output format: 'json' (default) or 'toon' (token-efficient for LLMs).".to_owned(),
            ),
            ..Default::default()
        },
    );
    properties
}

// ============================================================================
// GetSleepSessionsTool - Query stored sleep sessions
// ============================================================================

/// Tool for querying stored sleep sessions from the database.
pub struct GetSleepSessionsTool;

#[async_trait]
impl McpTool for GetSleepSessionsTool {
    fn name(&self) -> &'static str {
        "get_sleep_sessions"
    }

    fn description(&self) -> &'static str {
        "Get stored sleep sessions from the database"
    }

    fn input_schema(&self) -> JsonSchema {
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(date_range_properties()),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        delegate_to_handler(
            context,
            args,
            "get_sleep_sessions",
            handlers::handle_get_sleep_sessions,
        )
        .await
    }
}

// ============================================================================
// GetRecoveryMetricsTool - Query stored recovery and readiness metrics
// ============================================================================

/// Tool for querying stored recovery and readiness data from the database.
pub struct GetRecoveryMetricsTool;

#[async_trait]
impl McpTool for GetRecoveryMetricsTool {
    fn name(&self) -> &'static str {
        "get_recovery_metrics"
    }

    fn description(&self) -> &'static str {
        "Get stored recovery and readiness metrics"
    }

    fn input_schema(&self) -> JsonSchema {
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(date_range_properties()),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        delegate_to_handler(
            context,
            args,
            "get_recovery_metrics",
            handlers::handle_get_recovery_metrics,
        )
        .await
    }
}

// ============================================================================
// GetHealthSnapshotsTool - Query stored health snapshots
// ============================================================================

/// Tool for querying stored body composition and vitals snapshots.
pub struct GetHealthSnapshotsTool;

#[async_trait]
impl McpTool for GetHealthSnapshotsTool {
    fn name(&self) -> &'static str {
        "get_health_snapshots"
    }

    fn description(&self) -> &'static str {
        "Get stored health snapshots (body composition, vitals)"
    }

    fn input_schema(&self) -> JsonSchema {
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(date_range_properties()),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        delegate_to_handler(
            context,
            args,
            "get_health_snapshots",
            handlers::handle_get_health_snapshots,
        )
        .await
    }
}

// ============================================================================
// ListDataSourcesTool - List connected devices and providers
// ============================================================================

/// Tool for listing connected data sources (devices and providers) for the user.
pub struct ListDataSourcesTool;

#[async_trait]
impl McpTool for ListDataSourcesTool {
    fn name(&self) -> &'static str {
        "list_data_sources"
    }

    fn description(&self) -> &'static str {
        "List connected data sources (devices and providers)"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "format".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Output format: 'json' (default) or 'toon' (token-efficient for LLMs)."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        delegate_to_handler(
            context,
            args,
            "list_data_sources",
            handlers::handle_list_data_sources,
        )
        .await
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all data access tools for registration
#[must_use]
pub fn create_data_tools() -> Vec<Box<dyn McpTool>> {
    vec![
        Box::new(GetActivitiesTool),
        Box::new(GetAthleteTool),
        Box::new(GetStatsTool),
        Box::new(GetSleepSessionsTool),
        Box::new(GetRecoveryMetricsTool),
        Box::new(GetHealthSnapshotsTool),
        Box::new(ListDataSourcesTool),
    ]
}
