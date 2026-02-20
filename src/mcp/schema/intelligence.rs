// ABOUTME: MCP tool schema definitions for analytics, goals, configuration, and fitness config tools
// ABOUTME: Covers activity analysis, metrics, trends, goals, configuration management, and training zones
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use crate::constants::{
    json_fields::{ACTIVITY_ID, FORMAT, PROVIDER},
    tools::{
        ANALYZE_ACTIVITY, DELETE_FITNESS_CONFIG, GET_FITNESS_CONFIG, LIST_FITNESS_CONFIGS,
        SET_FITNESS_CONFIG,
    },
};

use super::{format_property, JsonSchema, PropertySchema, ToolSchema};

/// Create intelligence, analytics, goals, configuration, and fitness config tool schemas
pub(super) fn create_intelligence_tools() -> Vec<ToolSchema> {
    vec![
        // Advanced Analytics Tools
        create_analyze_activity_tool(),
        create_calculate_metrics_tool(),
        create_analyze_performance_trends_tool(),
        create_compare_activities_tool(),
        create_detect_patterns_tool(),
        create_set_goal_tool(),
        create_track_progress_tool(),
        create_suggest_goals_tool(),
        create_analyze_goal_feasibility_tool(),
        create_generate_recommendations_tool(),
        create_calculate_fitness_score_tool(),
        create_predict_performance_tool(),
        create_analyze_training_load_tool(),
        // Configuration Management Tools
        create_get_configuration_catalog_tool(),
        create_get_configuration_profiles_tool(),
        create_get_user_configuration_tool(),
        create_update_user_configuration_tool(),
        create_calculate_personalized_zones_tool(),
        create_validate_configuration_tool(),
        // Fitness Configuration Management Tools
        create_get_fitness_config_tool(),
        create_set_fitness_config_tool(),
        create_list_fitness_configs_tool(),
        create_delete_fitness_config_tool(),
    ]
}

/// Create the `analyze_activity` tool schema
fn create_analyze_activity_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        PROVIDER.to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Fitness provider name (e.g., 'strava', 'fitbit')".into()),
        },
    );

    properties.insert(
        ACTIVITY_ID.to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the activity to analyze".into()),
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: ANALYZE_ACTIVITY.to_owned(),
        description: "Perform deep analysis of an individual activity including insights, metrics, and anomaly detection".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec![PROVIDER.to_owned(), ACTIVITY_ID.to_owned()]),
        },
        annotations: None,
    }
}

/// Create the `calculate_metrics` tool schema
fn create_calculate_metrics_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "provider".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Fitness provider name".into()),
        },
    );

    properties.insert(
        "activity_id".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the activity".into()),
        },
    );

    properties.insert(
        "metrics".into(),
        PropertySchema {
            property_type: "array".into(),
            description: Some(
                "Specific metrics to calculate (e.g., ['trimp', 'power_to_weight', 'efficiency'])"
                    .to_owned(),
            ),
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: "calculate_metrics".into(),
        description: "Calculate advanced fitness metrics for an activity (TRIMP, power-to-weight ratio, efficiency scores, etc.)".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["provider".into(), "activity_id".into()]),
        },
        annotations: None,
    }
}

/// Create the `analyze_performance_trends` tool schema
fn create_analyze_performance_trends_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "provider".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Fitness provider name".into()),
        },
    );

    properties.insert(
        "timeframe".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Time period for analysis ('week', 'month', 'quarter', 'sixmonths', 'year')"
                    .to_owned(),
            ),
        },
    );

    properties.insert("metric".into(), PropertySchema {
        property_type: "string".into(),
        description: Some("Metric to analyze trends for ('pace', 'heart_rate', 'power', 'distance', 'duration')".into()),
    });

    properties.insert(
        "sport_type".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Filter by sport type (optional)".into()),
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: "analyze_performance_trends".into(),
        description: "Analyze performance trends over time with statistical analysis and insights"
            .to_owned(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["provider".into(), "timeframe".into(), "metric".into()]),
        },
        annotations: None,
    }
}

/// Create the `compare_activities` tool schema
fn create_compare_activities_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "provider".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Fitness provider name".into()),
        },
    );

    properties.insert(
        "activity_id".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Primary activity to compare".into()),
        },
    );

    properties.insert(
        "comparison_type".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Type of comparison ('similar_activities', 'personal_best', 'average', 'recent')"
                    .to_owned(),
            ),
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: "compare_activities".into(),
        description:
            "Compare an activity against similar activities, personal bests, or historical averages"
                .to_owned(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec![
                "provider".into(),
                "activity_id".into(),
                "comparison_type".into(),
            ]),
        },
        annotations: None,
    }
}

/// Create the `detect_patterns` tool schema
fn create_detect_patterns_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "provider".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Fitness provider name".into()),
        },
    );

    properties.insert("pattern_type".into(), PropertySchema {
        property_type: "string".into(),
        description: Some("Type of pattern to detect ('training_consistency', 'seasonal_trends', 'performance_plateaus', 'injury_risk')".into()),
    });

    properties.insert(
        "timeframe".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Time period for pattern analysis".into()),
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: "detect_patterns".into(),
        description: "Detect patterns in training data such as consistency trends, seasonal variations, or performance plateaus".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["provider".into(), "pattern_type".into()]),
        },
        annotations: None,
    }
}

/// Create the `set_goal` tool schema
fn create_set_goal_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "title".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Goal title".into()),
        },
    );

    properties.insert(
        "description".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Goal description".into()),
        },
    );

    properties.insert(
        "goal_type".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Type of goal ('distance', 'time', 'frequency', 'performance', 'custom')"
                    .to_owned(),
            ),
        },
    );

    properties.insert(
        "target_value".into(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Target value to achieve".into()),
        },
    );

    properties.insert(
        "target_date".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Target completion date (ISO format)".into()),
        },
    );

    properties.insert(
        "sport_type".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Sport type for the goal".into()),
        },
    );

    ToolSchema {
        name: "set_goal".into(),
        description: "Create and manage fitness goals with tracking and progress monitoring"
            .to_owned(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec![
                "title".into(),
                "goal_type".into(),
                "target_value".into(),
                "target_date".into(),
            ]),
        },
        annotations: None,
    }
}

/// Create the `track_progress` tool schema
fn create_track_progress_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "goal_id".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the goal to track".into()),
        },
    );

    ToolSchema {
        name: "track_progress".into(),
        description: "Track progress toward a specific goal with milestone achievements and completion estimates".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["goal_id".into()]),
        },
        annotations: None,
    }
}

/// Create the `suggest_goals` tool schema
fn create_suggest_goals_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "provider".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Fitness provider name".into()),
        },
    );

    properties.insert(
        "goal_category".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Category of goals to suggest ('distance', 'performance', 'consistency', 'all')"
                    .to_owned(),
            ),
        },
    );

    ToolSchema {
        name: "suggest_goals".into(),
        description: "Generate AI-powered goal suggestions based on user's activity history and fitness level".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["provider".into()]),
        },
        annotations: None,
    }
}

/// Create the `analyze_goal_feasibility` tool schema
fn create_analyze_goal_feasibility_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "goal_id".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("ID of the goal to analyze".into()),
        },
    );

    ToolSchema {
        name: "analyze_goal_feasibility".into(),
        description: "Assess whether a goal is realistic and achievable based on current performance and timeline".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["goal_id".into()]),
        },
        annotations: None,
    }
}

/// Create the `generate_recommendations` tool schema
fn create_generate_recommendations_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "provider".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Fitness provider name".into()),
        },
    );

    properties.insert(
        "recommendation_type".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Type of recommendations ('training', 'recovery', 'nutrition', 'equipment', 'all')"
                    .to_owned(),
            ),
        },
    );

    properties.insert(
        "activity_id".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Specific activity to base recommendations on (optional)".into()),
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: "generate_recommendations".into(),
        description:
            "Generate personalized training recommendations based on activity data and user profile"
                .to_owned(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["provider".into()]),
        },
        annotations: None,
    }
}

/// Create the `calculate_fitness_score` tool schema
fn create_calculate_fitness_score_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "provider".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Fitness provider for activity data (e.g., 'strava', 'garmin')".into(),
            ),
        },
    );

    properties.insert(
        "timeframe".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Time period for fitness assessment ('month', 'quarter', 'sixmonths')".into(),
            ),
        },
    );

    properties.insert(
        "sleep_provider".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Optional sleep/recovery provider (e.g., 'whoop', 'garmin'). If specified, factors recovery quality into fitness score.".into(),
            ),
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: "calculate_fitness_score".into(),
        description: "Calculate comprehensive fitness score based on recent training load, consistency, and performance trends. Optionally integrates sleep/recovery data for holistic assessment.".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["provider".into()]),
        },
        annotations: None,
    }
}

/// Create the `predict_performance` tool schema
fn create_predict_performance_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "provider".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Fitness provider name".into()),
        },
    );

    properties.insert(
        "target_sport".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Sport type for prediction".into()),
        },
    );

    properties.insert(
        "target_distance".into(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Target distance for performance prediction".into()),
        },
    );

    properties.insert(
        "target_date".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Target date for prediction (ISO format)".into()),
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: "predict_performance".into(),
        description: "Predict future performance capabilities based on current fitness trends and training history".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["provider".into(), "target_sport".into(), "target_distance".into()]),
        },
        annotations: None,
    }
}

/// Create the `analyze_training_load` tool schema
fn create_analyze_training_load_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "provider".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Fitness provider for activity data (e.g., 'strava', 'garmin')".into(),
            ),
        },
    );

    properties.insert(
        "timeframe".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Time period for load analysis ('week', 'month', 'quarter')".into()),
        },
    );

    properties.insert(
        "sleep_provider".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Optional sleep/recovery provider (e.g., 'whoop', 'garmin'). If specified, includes recovery metrics in training load analysis.".into(),
            ),
        },
    );

    properties.insert(FORMAT.to_owned(), format_property());

    ToolSchema {
        name: "analyze_training_load".into(),
        description:
            "Analyze training load balance, recovery needs, and load distribution. Optionally integrates sleep/recovery data for holistic load assessment."
                .to_owned(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["provider".into()]),
        },
        annotations: None,
    }
}

/// Create the `get_configuration_catalog` tool schema
fn create_get_configuration_catalog_tool() -> ToolSchema {
    ToolSchema {
        name: "get_configuration_catalog".into(),
        description: "Get the complete configuration catalog with all available parameters and their metadata".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(HashMap::new()),
            required: Some(vec![]),
        },
        annotations: None,
    }
}

/// Create the `get_configuration_profiles` tool schema
fn create_get_configuration_profiles_tool() -> ToolSchema {
    ToolSchema {
        name: "get_configuration_profiles".into(),
        description: "Get available configuration profiles (Research, Elite, Recreational, Beginner, Medical, etc.)".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(HashMap::new()),
            required: Some(vec![]),
        },
        annotations: None,
    }
}

/// Create the `get_user_configuration` tool schema
fn create_get_user_configuration_tool() -> ToolSchema {
    ToolSchema {
        name: "get_user_configuration".into(),
        description:
            "Get current user's configuration including active profile and parameter overrides"
                .to_owned(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(HashMap::new()),
            required: Some(vec![]),
        },
        annotations: None,
    }
}

/// Create the `update_user_configuration` tool schema
fn create_update_user_configuration_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "profile".into(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Configuration profile to apply (optional)".into()),
        },
    );

    properties.insert(
        "parameters".into(),
        PropertySchema {
            property_type: "object".into(),
            description: Some("Parameter overrides to apply (optional)".into()),
        },
    );

    ToolSchema {
        name: "update_user_configuration".into(),
        description: "Update user's configuration by applying a profile and/or parameter overrides"
            .to_owned(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec![]),
        },
        annotations: None,
    }
}

/// Create the `calculate_personalized_zones` tool schema
fn create_calculate_personalized_zones_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "vo2_max".into(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("VO2 max in ml/kg/min".into()),
        },
    );

    properties.insert(
        "resting_hr".into(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Resting heart rate in bpm (optional, defaults to 60)".into()),
        },
    );

    properties.insert(
        "max_hr".into(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Maximum heart rate in bpm (optional, defaults to 190)".into()),
        },
    );

    properties.insert(
        "lactate_threshold".into(),
        PropertySchema {
            property_type: "number".into(),
            description: Some(
                "Lactate threshold as percentage of VO2 max (optional, defaults to 0.85)"
                    .to_owned(),
            ),
        },
    );

    properties.insert(
        "sport_efficiency".into(),
        PropertySchema {
            property_type: "number".into(),
            description: Some("Sport efficiency factor (optional, defaults to 1.0)".into()),
        },
    );

    ToolSchema {
        name: "calculate_personalized_zones".into(),
        description: "Calculate personalized training zones (heart rate, pace, power) based on VO2 max and physiological parameters".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["vo2_max".into()]),
        },
        annotations: None,
    }
}

/// Create the `validate_configuration` tool schema
fn create_validate_configuration_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "parameters".into(),
        PropertySchema {
            property_type: "object".into(),
            description: Some("Configuration parameters to validate".into()),
        },
    );

    ToolSchema {
        name: "validate_configuration".into(),
        description:
            "Validate configuration parameters for physiological limits and scientific bounds"
                .to_owned(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["parameters".into()]),
        },
        annotations: None,
    }
}

// === FITNESS CONFIGURATION TOOLS ===

/// Create the `get_fitness_config` tool schema
fn create_get_fitness_config_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "configuration_name".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Name of the fitness configuration to retrieve (defaults to 'default')".into(),
            ),
        },
    );

    ToolSchema {
        name: GET_FITNESS_CONFIG.to_owned(),
        description: "Get fitness configuration settings including heart rate zones, power zones, and training parameters".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec![]), // configuration_name is optional
        },
        annotations: None,
    }
}

/// Create the `set_fitness_config` tool schema
fn create_set_fitness_config_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "configuration_name".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some(
                "Name of the fitness configuration to save (defaults to 'default')".into(),
            ),
        },
    );

    properties.insert(
        "configuration".to_owned(),
        PropertySchema {
            property_type: "object".into(),
            description: Some("Fitness configuration object containing zones, thresholds, and training parameters".into()),
        },
    );

    ToolSchema {
        name: SET_FITNESS_CONFIG.to_owned(),
        description: "Save fitness configuration settings for heart rate zones, power zones, and training parameters".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["configuration".to_owned()]), // configuration is required
        },
        annotations: None,
    }
}

/// Create the `list_fitness_configs` tool schema
fn create_list_fitness_configs_tool() -> ToolSchema {
    ToolSchema {
        name: LIST_FITNESS_CONFIGS.to_owned(),
        description: "List all available fitness configuration names for the user".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(HashMap::new()),
            required: Some(vec![]),
        },
        annotations: None,
    }
}

/// Create the `delete_fitness_config` tool schema
fn create_delete_fitness_config_tool() -> ToolSchema {
    let mut properties = HashMap::new();

    properties.insert(
        "configuration_name".to_owned(),
        PropertySchema {
            property_type: "string".into(),
            description: Some("Name of the fitness configuration to delete".into()),
        },
    );

    ToolSchema {
        name: DELETE_FITNESS_CONFIG.to_owned(),
        description: "Delete a specific fitness configuration by name".into(),
        input_schema: JsonSchema {
            schema_type: "object".into(),
            properties: Some(properties),
            required: Some(vec!["configuration_name".to_owned()]),
        },
        annotations: None,
    }
}
