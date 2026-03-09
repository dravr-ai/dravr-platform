// ABOUTME: MCP tool schema definitions for recipe management tools ("Combat des Chefs" architecture)
// ABOUTME: Covers recipe constraints, validation, CRUD operations, and search
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use crate::constants::{
    json_fields::FORMAT,
    tools::{
        DELETE_RECIPE, GET_RECIPE, GET_RECIPE_CONSTRAINTS, LIST_RECIPES, SAVE_RECIPE,
        SEARCH_RECIPES, VALIDATE_RECIPE,
    },
};

use super::{format_property, JsonSchema, PropertySchema, ToolSchema};

/// Create recipe management tool schemas
pub(super) fn create_recipe_tools() -> Vec<ToolSchema> {
    vec![
        create_get_recipe_constraints_tool(),
        create_validate_recipe_tool(),
        create_save_recipe_tool(),
        create_list_recipes_tool(),
        create_get_recipe_tool(),
        create_delete_recipe_tool(),
        create_search_recipes_tool(),
    ]
}

/// Create the `get_recipe_constraints` tool schema
fn create_get_recipe_constraints_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "meal_timing".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Training phase for macro targets: 'pre_training', 'post_training', 'rest_day', or 'general'".into(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "target_calories".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some(
                "Target calories for the meal (optional, for portion guidance)".into(),
            ),
            ..Default::default()
        },
    );

    ToolSchema {
        name: GET_RECIPE_CONSTRAINTS.to_owned(),
        description: "Get macro targets and constraints for LLM recipe generation based on training phase. Returns protein/carbs/fat percentages optimized for meal timing (e.g., high carbs pre-training, high protein post-training). Use this before generating recipes to ensure nutrition alignment.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["meal_timing".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `validate_recipe` tool schema
fn create_validate_recipe_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "name".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Recipe name".into()),
            ..Default::default()
        },
    );

    properties.insert(
        "servings".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Number of servings the recipe makes".into()),
            ..Default::default()
        },
    );

    properties.insert(
        "ingredients".to_owned(),
        PropertySchema {
            property_type: "array".into(),
            description: Some(
                "Array of ingredients with: name (string), amount (number), unit (string: 'grams', 'cups', 'tablespoons', 'teaspoons', 'pieces', 'ounces', 'milliliters'), fdc_id (number, optional USDA food ID for validation)".into(),
            ),
            items: Some(Box::new(PropertySchema {
                property_type: "object".into(),
                properties: Some(HashMap::from([
                    ("name".to_owned(), PropertySchema {
                        property_type: "string".into(),
                        description: Some("Ingredient name".into()),
                        ..Default::default()
                    }),
                    ("amount".to_owned(), PropertySchema {
                        property_type: "number".into(),
                        description: Some("Quantity of the ingredient".into()),
                        ..Default::default()
                    }),
                    ("unit".to_owned(), PropertySchema {
                        property_type: "string".into(),
                        description: Some("Unit of measurement".into()),
                        ..Default::default()
                    }),
                    ("fdc_id".to_owned(), PropertySchema {
                        property_type: "number".into(),
                        description: Some("Optional USDA FoodData Central food ID".into()),
                        ..Default::default()
                    }),
                ])),
                required: Some(vec!["name".to_owned(), "amount".to_owned(), "unit".to_owned()]),
                ..Default::default()
            })),
            ..Default::default()
        },
    );

    properties.insert(
        "meal_timing".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Intended meal timing: 'pre_training', 'post_training', 'rest_day', or 'general'"
                    .into(),
            ),
            ..Default::default()
        },
    );

    ToolSchema {
        name: VALIDATE_RECIPE.to_owned(),
        description: "Validate a recipe's nutrition against USDA database and calculate per-serving macros. Converts units to grams and looks up ingredients in USDA FoodData Central. Returns validation results with calculated calories, protein, carbs, fat, and any warnings about missing foods or macro targets.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec![
                "name".to_owned(),
                "servings".to_owned(),
                "ingredients".to_owned(),
            ]),
        },
        annotations: None,
    }
}

/// Create the `save_recipe` tool schema
fn create_save_recipe_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "name".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Recipe name".into()),
            ..Default::default()
        },
    );

    properties.insert(
        "description".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Recipe description (optional)".into()),
            ..Default::default()
        },
    );

    properties.insert(
        "servings".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Number of servings".into()),
            ..Default::default()
        },
    );

    properties.insert(
        "prep_time_mins".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Preparation time in minutes (optional)".into()),
            ..Default::default()
        },
    );

    properties.insert(
        "cook_time_mins".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Cooking time in minutes (optional)".into()),
            ..Default::default()
        },
    );

    properties.insert(
        "ingredients".to_owned(),
        PropertySchema {
            property_type: "array".into(),
            description: Some(
                "Array of ingredients with: name (string), amount (number), unit (string), grams (number), fdc_id (number, optional), preparation (string, optional)".into(),
            ),
            items: Some(Box::new(PropertySchema {
                property_type: "object".into(),
                properties: Some(HashMap::from([
                    ("name".to_owned(), PropertySchema {
                        property_type: "string".into(),
                        description: Some("Ingredient name".into()),
                        ..Default::default()
                    }),
                    ("amount".to_owned(), PropertySchema {
                        property_type: "number".into(),
                        description: Some("Quantity of the ingredient".into()),
                        ..Default::default()
                    }),
                    ("unit".to_owned(), PropertySchema {
                        property_type: "string".into(),
                        description: Some("Unit of measurement".into()),
                        ..Default::default()
                    }),
                    ("grams".to_owned(), PropertySchema {
                        property_type: "number".into(),
                        description: Some("Weight in grams".into()),
                        ..Default::default()
                    }),
                    ("fdc_id".to_owned(), PropertySchema {
                        property_type: "number".into(),
                        description: Some("Optional USDA FoodData Central food ID".into()),
                        ..Default::default()
                    }),
                    ("preparation".to_owned(), PropertySchema {
                        property_type: "string".into(),
                        description: Some("Optional preparation method".into()),
                        ..Default::default()
                    }),
                ])),
                required: Some(vec!["name".to_owned(), "amount".to_owned(), "unit".to_owned(), "grams".to_owned()]),
                ..Default::default()
            })),
            ..Default::default()
        },
    );

    properties.insert(
        "instructions".to_owned(),
        PropertySchema {
            property_type: "array".into(),
            description: Some("Array of instruction steps as strings".into()),
            items: Some(Box::new(PropertySchema {
                property_type: "string".into(),
                description: Some("Instruction step".into()),
                ..Default::default()
            })),
            ..Default::default()
        },
    );

    properties.insert(
        "tags".to_owned(),
        PropertySchema {
            property_type: "array".into(),
            description: Some(
                "Array of tags (optional, e.g., ['high-protein', 'quick', 'vegetarian'])".into(),
            ),
            items: Some(Box::new(PropertySchema {
                property_type: "string".into(),
                description: Some("Tag label".into()),
                ..Default::default()
            })),
            ..Default::default()
        },
    );

    properties.insert(
        "meal_timing".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Meal timing category: 'pre_training', 'post_training', 'rest_day', or 'general'"
                    .into(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "cached_nutrition".to_owned(),
        PropertySchema {
            property_type: "object".into(),
            description: Some(
                "Pre-validated nutrition data with: calories, protein_g, carbs_g, fat_g, fiber_g (optional), sodium_mg (optional), sugar_g (optional)".into(),
            ),
            ..Default::default()
        },
    );

    ToolSchema {
        name: SAVE_RECIPE.to_owned(),
        description: "Save a validated recipe to user's personal collection. Should be called after validate_recipe to ensure nutrition data is accurate. Stores recipe with cached nutrition for quick access.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec![
                "name".to_owned(),
                "servings".to_owned(),
                "ingredients".to_owned(),
                "instructions".to_owned(),
            ]),
        },
        annotations: None,
    }
}

/// Create the `list_recipes` tool schema
fn create_list_recipes_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "meal_timing".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Filter by meal timing: 'pre_training', 'post_training', 'rest_day', or 'general' (optional)".into(),
            ),
            ..Default::default()
        },
    );

    properties.insert(
        "limit".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Maximum number of recipes to return (default: 20)".into()),
            ..Default::default()
        },
    );

    properties.insert(
        "offset".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Number of recipes to skip for pagination (default: 0)".into()),
            ..Default::default()
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: LIST_RECIPES.to_owned(),
        description: "List user's saved recipes with optional filtering by meal timing. Returns recipe summaries with name, description, meal timing, and cached nutrition per serving.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec![]),
        },
        annotations: None,
    }
}

/// Create the `get_recipe` tool schema
fn create_get_recipe_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "recipe_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the recipe to retrieve".into()),
            ..Default::default()
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: GET_RECIPE.to_owned(),
        description: "Get a specific recipe by ID from user's collection. Returns full recipe details including ingredients, instructions, and nutrition data.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["recipe_id".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `delete_recipe` tool schema
fn create_delete_recipe_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "recipe_id".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the recipe to delete".into()),
            ..Default::default()
        },
    );

    ToolSchema {
        name: DELETE_RECIPE.to_owned(),
        description: "Delete a recipe from user's collection. This action cannot be undone.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["recipe_id".to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `search_recipes` tool schema
fn create_search_recipes_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "query".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Search query for recipe name, description, or tags".into()),
            ..Default::default()
        },
    );

    properties.insert(
        "limit".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Maximum number of results to return (default: 10, max: 100)".into()),
            ..Default::default()
        },
    );

    properties.insert(
        "offset".to_owned(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Number of results to skip (for pagination, default: 0)".into()),
            ..Default::default()
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: SEARCH_RECIPES.to_owned(),
        description: "Search user's recipes by name, description, or tags. Returns matching recipes with relevance ranking.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["query".to_owned()]),
        },
        annotations: None,
    }
}
