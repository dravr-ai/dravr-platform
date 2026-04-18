// ABOUTME: User configuration tools for personalized training settings.
// ABOUTME: Implements catalog, profiles, user config, zones calculation, and validation.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # User Configuration Tools
//!
//! This module provides tools for user configuration with direct business logic:
//! - `GetConfigurationCatalogTool` - Get available configuration options
//! - `GetConfigurationProfilesTool` - Get configuration profile templates
//! - `GetUserConfigurationTool` - Get user's current configuration
//! - `UpdateUserConfigurationTool` - Update user configuration
//! - `CalculatePersonalizedZonesTool` - Calculate training zones
//! - `ValidateConfigurationTool` - Validate configuration values

use std::collections::HashMap;

use async_trait::async_trait;
use serde_json::Value;

use crate::errors::AppResult;
use crate::mcp::schema::{JsonSchema, PropertySchema};
use crate::protocols::universal::handlers;
use crate::tools::context::ToolExecutionContext;
use crate::tools::result::ToolResult;
use crate::tools::traits::{McpTool, ToolCapabilities};
use crate::tools::universal_delegate::delegate_to_handler;

// ============================================================================
// GetConfigurationCatalogTool
// ============================================================================

/// Tool for getting the complete configuration catalog.
pub struct GetConfigurationCatalogTool;

#[async_trait]
impl McpTool for GetConfigurationCatalogTool {
    fn name(&self) -> &'static str {
        "get_configuration_catalog"
    }

    fn description(&self) -> &'static str {
        "Get the complete catalog of available configuration options"
    }

    fn input_schema(&self) -> JsonSchema {
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(HashMap::new()),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        delegate_to_handler(
            context,
            args,
            "get_configuration_catalog",
            handlers::handle_get_configuration_catalog,
        )
        .await
    }
}

// ============================================================================
// GetConfigurationProfilesTool
// ============================================================================

/// Tool for getting available configuration profiles.
pub struct GetConfigurationProfilesTool;

#[async_trait]
impl McpTool for GetConfigurationProfilesTool {
    fn name(&self) -> &'static str {
        "get_configuration_profiles"
    }

    fn description(&self) -> &'static str {
        "Get available configuration profile templates"
    }

    fn input_schema(&self) -> JsonSchema {
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(HashMap::new()),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        delegate_to_handler(
            context,
            args,
            "get_configuration_profiles",
            handlers::handle_get_configuration_profiles,
        )
        .await
    }
}

// ============================================================================
// GetUserConfigurationTool
// ============================================================================

/// Tool for getting user's current configuration.
pub struct GetUserConfigurationTool;

#[async_trait]
impl McpTool for GetUserConfigurationTool {
    fn name(&self) -> &'static str {
        "get_user_configuration"
    }

    fn description(&self) -> &'static str {
        "Get your current training configuration settings"
    }

    fn input_schema(&self) -> JsonSchema {
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(HashMap::new()),
            required: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        delegate_to_handler(
            context,
            args,
            "get_user_configuration",
            handlers::handle_get_user_configuration,
        )
        .await
    }
}

// ============================================================================
// UpdateUserConfigurationTool
// ============================================================================

/// Tool for updating user configuration.
pub struct UpdateUserConfigurationTool;

#[async_trait]
impl McpTool for UpdateUserConfigurationTool {
    fn name(&self) -> &'static str {
        "update_user_configuration"
    }

    fn description(&self) -> &'static str {
        "Update your training configuration settings"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "profile".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("Profile name to apply".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "parameters".to_owned(),
            PropertySchema {
                property_type: "object".to_owned(),
                description: Some("Configuration parameters to update".to_owned()),
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
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        delegate_to_handler(
            context,
            args,
            "update_user_configuration",
            handlers::handle_update_user_configuration,
        )
        .await
    }
}

// ============================================================================
// CalculatePersonalizedZonesTool
// ============================================================================

/// Tool for calculating personalized training zones.
pub struct CalculatePersonalizedZonesTool;

#[async_trait]
impl McpTool for CalculatePersonalizedZonesTool {
    fn name(&self) -> &'static str {
        "calculate_personalized_zones"
    }

    fn description(&self) -> &'static str {
        "Calculate personalized training zones based on your fitness metrics"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "vo2_max".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some("VO2 max in ml/kg/min (required)".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "resting_hr".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Resting heart rate in bpm".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "max_hr".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Maximum heart rate in bpm".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "lactate_threshold".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some("Lactate threshold".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "sport_efficiency".to_owned(),
            PropertySchema {
                property_type: "number".to_owned(),
                description: Some("Sport efficiency factor".to_owned()),
                ..Default::default()
            },
        );
        properties.insert(
            "ftp".to_owned(),
            PropertySchema {
                property_type: "integer".to_owned(),
                description: Some("Functional Threshold Power in watts".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["vo2_max".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        delegate_to_handler(
            context,
            args,
            "calculate_personalized_zones",
            handlers::handle_calculate_personalized_zones,
        )
        .await
    }
}

// ============================================================================
// ValidateConfigurationTool
// ============================================================================

/// Tool for validating configuration parameters.
pub struct ValidateConfigurationTool;

#[async_trait]
impl McpTool for ValidateConfigurationTool {
    fn name(&self) -> &'static str {
        "validate_configuration"
    }

    fn description(&self) -> &'static str {
        "Validate configuration parameters for physiological correctness"
    }

    fn input_schema(&self) -> JsonSchema {
        let mut properties = HashMap::new();
        properties.insert(
            "parameters".to_owned(),
            PropertySchema {
                property_type: "object".to_owned(),
                description: Some("Configuration parameters to validate".to_owned()),
                ..Default::default()
            },
        );
        JsonSchema {
            schema_type: "object".to_owned(),
            properties: Some(properties),
            required: Some(vec!["parameters".to_owned()]),
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(&self, args: Value, context: &ToolExecutionContext) -> AppResult<ToolResult> {
        delegate_to_handler(
            context,
            args,
            "validate_configuration",
            handlers::handle_validate_configuration,
        )
        .await
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all configuration tools for registration
#[must_use]
pub fn create_configuration_tools() -> Vec<Box<dyn McpTool>> {
    vec![
        Box::new(GetConfigurationCatalogTool),
        Box::new(GetConfigurationProfilesTool),
        Box::new(GetUserConfigurationTool),
        Box::new(UpdateUserConfigurationTool),
        Box::new(CalculatePersonalizedZonesTool),
        Box::new(ValidateConfigurationTool),
    ]
}
