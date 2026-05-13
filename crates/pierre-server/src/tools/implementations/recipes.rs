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

use async_trait::async_trait;
use serde_json::Value;

use crate::errors::AppResult;
use crate::mcp::schema::{JsonSchema, PropertySchema};
use crate::protocols::universal::handlers;
use crate::tools::context::ToolExecutionContext;
use crate::tools::dispatch::dispatch_handler;
use crate::tools::result::ToolResult;
use crate::tools::traits::{McpTool, ToolCapabilities};

// ============================================================================
// GetRecipeConstraintsTool
// ============================================================================

/// Tool for getting recipe constraints and macro targets.
pub struct GetRecipeConstraintsTool;

#[async_trait]
impl McpTool for GetRecipeConstraintsTool {
    fn name(&self) -> &'static str {
        "get_recipe_constraints"
    }

    fn description(&self) -> &'static str {
        "Get macro targets and constraints for LLM recipe generation"
    }

    fn input_schema(&self) -> JsonSchema {
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
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(
            context,
            args,
            "get_recipe_constraints",
            handlers::handle_get_recipe_constraints,
        )
        .await
    }
}

// ============================================================================
// ValidateRecipeTool
// ============================================================================

/// Tool for validating recipe nutrition via USDA.
pub struct ValidateRecipeTool;

#[async_trait]
impl McpTool for ValidateRecipeTool {
    fn name(&self) -> &'static str {
        "validate_recipe"
    }

    fn description(&self) -> &'static str {
        "Validate recipe nutrition using USDA FoodData Central"
    }

    fn input_schema(&self) -> JsonSchema {
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
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["servings".to_owned(), "ingredients".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(
            context,
            args,
            "validate_recipe",
            handlers::handle_validate_recipe,
        )
        .await
    }
}

// ============================================================================
// SaveRecipeTool
// ============================================================================

/// Tool for saving a recipe.
pub struct SaveRecipeTool;

#[async_trait]
impl McpTool for SaveRecipeTool {
    fn name(&self) -> &'static str {
        "save_recipe"
    }

    fn description(&self) -> &'static str {
        "Save a validated recipe to your collection"
    }

    fn input_schema(&self) -> JsonSchema {
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
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec![
                "name".to_owned(),
                "servings".to_owned(),
                "instructions".to_owned(),
                "ingredients".to_owned(),
            ]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(context, args, "save_recipe", handlers::handle_save_recipe).await
    }
}

// ============================================================================
// ListRecipesTool
// ============================================================================

/// Tool for listing user's recipes.
pub struct ListRecipesTool;

#[async_trait]
impl McpTool for ListRecipesTool {
    fn name(&self) -> &'static str {
        "list_recipes"
    }

    fn description(&self) -> &'static str {
        "List your saved recipes"
    }

    fn input_schema(&self) -> JsonSchema {
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
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(context, args, "list_recipes", handlers::handle_list_recipes).await
    }
}

// ============================================================================
// GetRecipeTool
// ============================================================================

/// Tool for getting a specific recipe.
pub struct GetRecipeTool;

#[async_trait]
impl McpTool for GetRecipeTool {
    fn name(&self) -> &'static str {
        "get_recipe"
    }

    fn description(&self) -> &'static str {
        "Get details of a specific recipe"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "recipe_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Recipe ID".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["recipe_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(context, args, "get_recipe", handlers::handle_get_recipe).await
    }
}

// ============================================================================
// DeleteRecipeTool
// ============================================================================

/// Tool for deleting a recipe.
pub struct DeleteRecipeTool;

#[async_trait]
impl McpTool for DeleteRecipeTool {
    fn name(&self) -> &'static str {
        "delete_recipe"
    }

    fn description(&self) -> &'static str {
        "Delete a recipe from your collection"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "recipe_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Recipe ID to delete".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["recipe_id".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(
            context,
            args,
            "delete_recipe",
            handlers::handle_delete_recipe,
        )
        .await
    }
}

// ============================================================================
// SearchRecipesTool
// ============================================================================

/// Tool for searching recipes.
pub struct SearchRecipesTool;

#[async_trait]
impl McpTool for SearchRecipesTool {
    fn name(&self) -> &'static str {
        "search_recipes"
    }

    fn description(&self) -> &'static str {
        "Search your recipes by name, tags, or description"
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
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["query".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        dispatch_handler(
            context,
            args,
            "search_recipes",
            handlers::handle_search_recipes,
        )
        .await
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all recipe tools for registration
#[must_use]
pub fn create_recipe_tools() -> Vec<Box<dyn McpTool>> {
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
