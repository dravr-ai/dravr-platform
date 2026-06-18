// ABOUTME: Recipe management tools for meal planning and nutrition.
// ABOUTME: Implements validate_recipe, save_recipe, list_recipes, search_recipes, etc.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Recipe Management Tools
//!
//! This module provides tools for recipe management with direct business logic:
//! - `GetRecipeConstraintsTool` - Get macro targets for recipe generation
//! - `ValidateRecipeTool` - Validate recipe nutrition via USDA
//! - `SaveRecipeTool` - Save a new recipe
//! - `ListRecipesTool` - List user's recipes
//! - `GetRecipeTool` - Get recipe details
//! - `DeleteRecipeTool` - Delete a recipe
//! - `SearchRecipesTool` - Search recipes

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{capabilities_to_tronc, tool_definition, tool_result_to_response};
use crate::runtime::ToolRuntime;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_core::errors::AppResult;
use pierre_mcp_schema::{JsonSchema, PropertySchema};
use pierre_tools_core::ToolResult;

mod inner;

// ============================================================================
// GetRecipeConstraintsTool
// ============================================================================

/// Tool for getting recipe constraints and macro targets.
pub struct GetRecipeConstraintsTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetRecipeConstraintsTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "calories".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some("Target calories for the meal".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "tdee".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some("User's Total Daily Energy Expenditure".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "meal_timing".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("pre_training, post_training, rest_day, or general".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "dietary_restrictions".to_owned(),
            PropertySchema {
                property_type: "array".to_owned(),
                description: Some("Dietary restrictions like gluten_free, vegan".to_owned()),
                items: Some(Box::new(PropertySchema {
                    property_type: "string".to_owned(),
                    description: Some("Dietary restriction".to_owned()),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        properties.insert(
            "max_prep_time_mins".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Maximum preparation time".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "max_cook_time_mins".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Maximum cooking time".to_owned()),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        };
        tool_definition(
            "get_recipe_constraints",
            "Get macro targets and constraints for LLM recipe generation",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> =
            async move { Ok(inner::handle_get_recipe_constraints(&context, &args)) }.await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// ValidateRecipeTool
// ============================================================================

/// Tool for validating recipe nutrition via USDA.
pub struct ValidateRecipeTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ValidateRecipeTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "name".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Recipe name".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "servings".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Number of servings".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "ingredients".to_owned(),
            PropertySchema {
                property_type: "array".to_owned(),
                description: Some("Array of {name, amount, unit}".to_owned()),
                items: Some(Box::new(PropertySchema {
                    property_type: "object".to_owned(),
                    properties: Some(HashMap::from([
                        (
                            "name".to_owned(),
                            PropertySchema {
                                property_type: "string".to_owned(),
                                description: Some("Ingredient name".to_owned()),
                                ..Default::default()
                            },
                        ),
                        (
                            "amount".to_owned(),
                            PropertySchema {
                                property_type: "number".to_owned(),
                                description: Some("Quantity".to_owned()),
                                ..Default::default()
                            },
                        ),
                        (
                            "unit".to_owned(),
                            PropertySchema {
                                property_type: "string".to_owned(),
                                description: Some("Unit of measurement".to_owned()),
                                ..Default::default()
                            },
                        ),
                    ])),
                    required: Some(vec![
                        "name".to_owned(),
                        "amount".to_owned(),
                        "unit".to_owned(),
                    ]),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["servings".to_owned(), "ingredients".to_owned()]),
        };
        tool_definition(
            "validate_recipe",
            "Validate recipe nutrition using USDA FoodData Central",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> =
            async move { inner::handle_validate_recipe(&context, args).await }.await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// SaveRecipeTool
// ============================================================================

/// Tool for saving a recipe.
pub struct SaveRecipeTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for SaveRecipeTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "name".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Recipe name".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "servings".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Number of servings".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "instructions".to_owned(),
            PropertySchema {
                property_type: "array".to_owned(),
                description: Some("Array of instruction steps".to_owned()),
                items: Some(Box::new(PropertySchema {
                    property_type: "string".to_owned(),
                    description: Some("Instruction step".to_owned()),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        properties.insert(
            "ingredients".to_owned(),
            PropertySchema {
                property_type: "array".to_owned(),
                description: Some("Array of {name, amount, unit}".to_owned()),
                items: Some(Box::new(PropertySchema {
                    property_type: "object".to_owned(),
                    properties: Some(HashMap::from([
                        (
                            "name".to_owned(),
                            PropertySchema {
                                property_type: "string".to_owned(),
                                description: Some("Ingredient name".to_owned()),
                                ..Default::default()
                            },
                        ),
                        (
                            "amount".to_owned(),
                            PropertySchema {
                                property_type: "number".to_owned(),
                                description: Some("Quantity".to_owned()),
                                ..Default::default()
                            },
                        ),
                        (
                            "unit".to_owned(),
                            PropertySchema {
                                property_type: "string".to_owned(),
                                description: Some("Unit of measurement".to_owned()),
                                ..Default::default()
                            },
                        ),
                    ])),
                    required: Some(vec![
                        "name".to_owned(),
                        "amount".to_owned(),
                        "unit".to_owned(),
                    ]),
                    ..Default::default()
                })),
                ..Default::default()
            },
        );
        properties.insert(
            "description".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Recipe description".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "meal_timing".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("pre_training, post_training, rest_day, or general".to_owned()),
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
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec![
                "name".to_owned(),
                "servings".to_owned(),
                "instructions".to_owned(),
                "ingredients".to_owned(),
            ]),
        };
        tool_definition(
            "save_recipe",
            "Save a validated recipe to your collection",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> =
            async move { inner::handle_save_recipe(&context, args).await }.await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// ListRecipesTool
// ============================================================================

/// Tool for listing user's recipes.
pub struct ListRecipesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ListRecipesTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "meal_timing".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Filter by meal timing".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "limit".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Maximum results (default: 20)".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "offset".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Pagination offset".to_owned()),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        };
        tool_definition("list_recipes", "List your saved recipes", schema, None)
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> =
            async move { inner::handle_list_recipes(&context, args).await }.await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetRecipeTool
// ============================================================================

/// Tool for getting a specific recipe.
pub struct GetRecipeTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetRecipeTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "recipe_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Recipe ID".to_owned()),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["recipe_id".to_owned()]),
        };
        tool_definition(
            "get_recipe",
            "Get details of a specific recipe",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> =
            async move { inner::handle_get_recipe(&context, args).await }.await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// DeleteRecipeTool
// ============================================================================

/// Tool for deleting a recipe.
pub struct DeleteRecipeTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for DeleteRecipeTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "recipe_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Recipe ID to delete".to_owned()),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["recipe_id".to_owned()]),
        };
        tool_definition(
            "delete_recipe",
            "Delete a recipe from your collection",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> =
            async move { inner::handle_delete_recipe(&context, args).await }.await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// SearchRecipesTool
// ============================================================================

/// Tool for searching recipes.
pub struct SearchRecipesTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for SearchRecipesTool {
    fn definition(&self) -> Tool {
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
            "limit".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Maximum results (default: 10)".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "offset".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Pagination offset".to_owned()),
                ..Default::default()
            },
        );
        let schema = JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["query".to_owned()]),
        };
        tool_definition(
            "search_recipes",
            "Search your recipes by name, tags, or description",
            schema,
            None,
        )
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA)
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> =
            async move { inner::handle_search_recipes(&context, args).await }.await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all recipe tools for registration
#[must_use]
pub fn create_recipe_tools() -> Vec<Box<dyn McpTool<dyn ToolRuntime>>> {
    vec![
        Box::new(GetRecipeConstraintsTool),
        Box::new(ValidateRecipeTool),
        Box::new(SaveRecipeTool),
        Box::new(ListRecipesTool),
        Box::new(GetRecipeTool),
        Box::new(DeleteRecipeTool),
        Box::new(SearchRecipesTool),
    ]
}
