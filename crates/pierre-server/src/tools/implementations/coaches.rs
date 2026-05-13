// ABOUTME: AI coach management tools with direct database access.
// ABOUTME: Implements list_coaches, create_coach, get_coach, etc. using CoachesManager.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # AI Coach Management Tools
//!
//! This module provides tools for AI coach management with direct business logic:
//! - `ListCoachesTool` - List available coaches
//! - `CreateCoachTool` - Create a custom coach
//! - `GetCoachTool` - Get coach details
//! - `UpdateCoachTool` - Update coach settings
//! - `DeleteCoachTool` - Delete a coach
//! - `ToggleCoachFavoriteTool` - Toggle favorite status
//! - `SearchCoachesTool` - Search coaches
//! - `ActivateCoachTool` - Activate a coach
//! - `DeactivateCoachTool` - Deactivate the active coach
//! - `GetActiveCoachTool` - Get currently active coach
//! - `HideCoachTool` - Hide a coach from listings
//! - `ShowCoachTool` - Show a hidden coach
//! - `ListHiddenCoachesTool` - List hidden coaches

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::Value;

use crate::errors::AppResult;
use crate::mcp::schema::{JsonSchema, PropertySchema, ToolAnnotations};
use crate::protocols::universal::handlers;
use crate::tools::context::ToolExecutionContext;
use crate::tools::dispatch::dispatch_handler;
use crate::tools::result::ToolResult;
use crate::tools::traits::{McpTool, ToolCapabilities};

/// Annotations for idempotent write operations (create, update)
fn write_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Annotations for destructive operations (delete)
fn destructive_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(true),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Annotations for read-only coach retrieval operations
fn read_only_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

// ============================================================================
// ListCoachesTool
// ============================================================================

/// Tool for listing available AI coaches.
pub struct ListCoachesTool;

#[async_trait]
impl McpTool for ListCoachesTool {
    fn name(&self) -> &'static str {
        "list_coaches"
    }

    fn description(&self) -> &'static str {
        "List available AI coaches for personalized training guidance"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "category".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Filter by category".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "include_system".to_owned(),
            PropertySchema {
                property_type: "boolean".to_owned(),
                description: Some("Include system coaches. Default: true".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "favorites_only".to_owned(),
            PropertySchema {
                property_type: "boolean".to_owned(),
                description: Some("Only show favorites. Default: false".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "limit".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Max results. Default: 50".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "offset".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Pagination offset. Default: 0".to_owned()),
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
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(ctx, args, "list_coaches", handlers::handle_list_coaches).await
    }
}

// ============================================================================
// CreateCoachTool
// ============================================================================

/// Tool for creating a custom AI coach.
pub struct CreateCoachTool;

#[async_trait]
impl McpTool for CreateCoachTool {
    fn name(&self) -> &'static str {
        "create_coach"
    }

    fn description(&self) -> &'static str {
        "Create a custom AI coach with personalized training guidance"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "title".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Display title for the coach".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "system_prompt".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("System prompt that shapes AI responses".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "description".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Description of the coach".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "category".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Category: training, nutrition, recovery, recipes, custom".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "tags".to_owned(),
            PropertySchema {
                property_type: "array".to_owned(),
                description: Some("Tags for organization".to_owned()),
                items: Some(Box::new(PropertySchema {
                    property_type: "string".to_owned(),
                    description: Some("Tag label".to_owned()),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        properties.insert(
            "sample_prompts".to_owned(),
            PropertySchema {
                property_type: "array".to_owned(),
                description: Some("Example prompts to show users".to_owned()),
                items: Some(Box::new(PropertySchema {
                    property_type: "string".to_owned(),
                    description: Some("Sample prompt text".to_owned()),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["title".to_owned(), "system_prompt".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(write_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(ctx, args, "create_coach", handlers::handle_create_coach).await
    }
}

// ============================================================================
// GetCoachTool
// ============================================================================

/// Tool for getting coach details.
pub struct GetCoachTool;

#[async_trait]
impl McpTool for GetCoachTool {
    fn name(&self) -> &'static str {
        "get_coach"
    }

    fn description(&self) -> &'static str {
        "Get detailed information about a specific coach"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach to retrieve".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(ctx, args, "get_coach", handlers::handle_get_coach).await
    }
}

// ============================================================================
// UpdateCoachTool
// ============================================================================

/// Tool for updating coach settings.
pub struct UpdateCoachTool;

#[async_trait]
impl McpTool for UpdateCoachTool {
    fn name(&self) -> &'static str {
        "update_coach"
    }

    fn description(&self) -> &'static str {
        "Update an existing coach's settings"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach to update".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "title".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("New title".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "system_prompt".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("New system prompt".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "description".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("New description".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "category".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("New category".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "tags".to_owned(),
            PropertySchema {
                property_type: "array".to_owned(),
                description: Some("New tags".to_owned()),
                items: Some(Box::new(PropertySchema {
                    property_type: "string".to_owned(),
                    description: Some("Tag label".to_owned()),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(write_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(ctx, args, "update_coach", handlers::handle_update_coach).await
    }
}

// ============================================================================
// DeleteCoachTool
// ============================================================================

/// Tool for deleting a coach.
pub struct DeleteCoachTool;

#[async_trait]
impl McpTool for DeleteCoachTool {
    fn name(&self) -> &'static str {
        "delete_coach"
    }

    fn description(&self) -> &'static str {
        "Delete a coach"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach to delete".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(destructive_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(ctx, args, "delete_coach", handlers::handle_delete_coach).await
    }
}

// ============================================================================
// ToggleCoachFavoriteTool
// ============================================================================

/// Tool for toggling coach favorite status.
pub struct ToggleCoachFavoriteTool;

#[async_trait]
impl McpTool for ToggleCoachFavoriteTool {
    fn name(&self) -> &'static str {
        "toggle_coach_favorite"
    }

    fn description(&self) -> &'static str {
        "Toggle the favorite status of a coach"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(write_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(
            ctx,
            args,
            "toggle_coach_favorite",
            handlers::handle_toggle_coach_favorite,
        )
        .await
    }
}

// ============================================================================
// SearchCoachesTool
// ============================================================================

/// Tool for searching coaches.
pub struct SearchCoachesTool;

#[async_trait]
impl McpTool for SearchCoachesTool {
    fn name(&self) -> &'static str {
        "search_coaches"
    }

    fn description(&self) -> &'static str {
        "Search for coaches by query. Returns up to 20 results by default. Check the `has_more` field before requesting additional results with offset."
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "query".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Search query".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "category".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Filter by category".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "limit".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Maximum results per request. Default: 20, max: 100".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "offset".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Pagination offset. Default: 0. Only use if previous response had has_more=true".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["query".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(ctx, args, "search_coaches", handlers::handle_search_coaches).await
    }
}

// ============================================================================
// ActivateCoachTool
// ============================================================================

/// Tool for activating a coach.
pub struct ActivateCoachTool;

#[async_trait]
impl McpTool for ActivateCoachTool {
    fn name(&self) -> &'static str {
        "activate_coach"
    }

    fn description(&self) -> &'static str {
        "Activate a coach for personalized training guidance"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach to activate".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(write_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(ctx, args, "activate_coach", handlers::handle_activate_coach).await
    }
}

// ============================================================================
// DeactivateCoachTool
// ============================================================================

/// Tool for deactivating the current coach.
pub struct DeactivateCoachTool;

#[async_trait]
impl McpTool for DeactivateCoachTool {
    fn name(&self) -> &'static str {
        "deactivate_coach"
    }

    fn description(&self) -> &'static str {
        "Deactivate the current coach and return to default AI guidance"
    }

    fn input_schema(&self) -> JsonSchema {
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(HashMap::new()),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(write_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(
            ctx,
            args,
            "deactivate_coach",
            handlers::handle_deactivate_coach,
        )
        .await
    }
}

// ============================================================================
// GetActiveCoachTool
// ============================================================================

/// Tool for getting the currently active coach.
pub struct GetActiveCoachTool;

#[async_trait]
impl McpTool for GetActiveCoachTool {
    fn name(&self) -> &'static str {
        "get_active_coach"
    }

    fn description(&self) -> &'static str {
        "Get the currently active coach"
    }

    fn input_schema(&self) -> JsonSchema {
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(HashMap::new()),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(
            ctx,
            args,
            "get_active_coach",
            handlers::handle_get_active_coach,
        )
        .await
    }
}

// ============================================================================
// HideCoachTool
// ============================================================================

/// Tool for hiding a coach from listings.
pub struct HideCoachTool;

#[async_trait]
impl McpTool for HideCoachTool {
    fn name(&self) -> &'static str {
        "hide_coach"
    }

    fn description(&self) -> &'static str {
        "Hide a coach from listings"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach to hide".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(write_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(ctx, args, "hide_coach", handlers::handle_hide_coach).await
    }
}

// ============================================================================
// ShowCoachTool
// ============================================================================

/// Tool for showing a hidden coach.
pub struct ShowCoachTool;

#[async_trait]
impl McpTool for ShowCoachTool {
    fn name(&self) -> &'static str {
        "show_coach"
    }

    fn description(&self) -> &'static str {
        "Show a previously hidden coach"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "coach_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("ID of the coach to show".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["coach_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::WRITES_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(write_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(ctx, args, "show_coach", handlers::handle_show_coach).await
    }
}

// ============================================================================
// ListHiddenCoachesTool
// ============================================================================

/// Tool for listing hidden coaches.
pub struct ListHiddenCoachesTool;

#[async_trait]
impl McpTool for ListHiddenCoachesTool {
    fn name(&self) -> &'static str {
        "list_hidden_coaches"
    }

    fn description(&self) -> &'static str {
        "List all hidden coaches"
    }

    fn input_schema(&self) -> JsonSchema {
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(HashMap::new()),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::COACHES | ToolCapabilities::READS_DATA
    }

    fn annotations(&self) -> Option<ToolAnnotations> {
        Some(read_only_annotations())
    }

    async fn execute(&self, args: Value, ctx: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(
            ctx,
            args,
            "list_hidden_coaches",
            handlers::handle_list_hidden_coaches,
        )
        .await
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all coach tools for registration
#[must_use]
pub fn create_coach_tools() -> Vec<Box<dyn McpTool>> {
    vec![
        Box::new(ListCoachesTool),
        Box::new(CreateCoachTool),
        Box::new(GetCoachTool),
        Box::new(UpdateCoachTool),
        Box::new(DeleteCoachTool),
        Box::new(ToggleCoachFavoriteTool),
        Box::new(SearchCoachesTool),
        Box::new(ActivateCoachTool),
        Box::new(DeactivateCoachTool),
        Box::new(GetActiveCoachTool),
        Box::new(HideCoachTool),
        Box::new(ShowCoachTool),
        Box::new(ListHiddenCoachesTool),
    ]
}
