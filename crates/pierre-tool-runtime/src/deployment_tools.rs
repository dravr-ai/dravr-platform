// ABOUTME: Tools this deployment cannot answer with: left out of an external listing rather than offered and failed
// ABOUTME: A tool declares what it needs from the deployment; the listing reads the declaration against the config

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tools the deployment is not configured to answer with.
//!
//! A food-database tool on a deployment holding no USDA key can only fail, so
//! it is not offered. The tool declares the need
//! ([`RuntimeTool::requires_food_database`](crate::security::RuntimeTool::requires_food_database))
//! and nothing keeps a list of names. A call that names the tool anyway
//! reaches its own body, which says the key is not configured.

use pierre_config::environment::ServerConfig;
use pierre_mcp_schema::ToolSchema;
use secrecy::ExposeSecret;

use crate::registry::ToolRegistry;

/// Whether `config` holds a USDA key to call the food database with.
#[must_use]
pub fn holds_food_database_key(config: &ServerConfig) -> bool {
    config
        .usda_api_key
        .as_ref()
        .is_some_and(|key| !key.expose_secret().is_empty())
}

/// Whether this deployment can answer with `tool_name`: the predicate every
/// listing reads, the MCP and A2A ones and the chat catalogue alike.
#[must_use]
pub fn answers_with(registry: &ToolRegistry, tool_name: &str, config: &ServerConfig) -> bool {
    holds_food_database_key(config) || !registry.requires_food_database(tool_name)
}

/// Drop from `schemas` every tool this deployment can only fail.
pub fn retain_answerable(
    registry: &ToolRegistry,
    schemas: &mut Vec<ToolSchema>,
    config: &ServerConfig,
) {
    if holds_food_database_key(config) {
        return;
    }
    schemas.retain(|schema| answers_with(registry, &schema.name, config));
}

#[cfg(all(test, feature = "tools-nutrition"))]
mod tests {
    use std::sync::Arc;

    use secrecy::SecretString;

    use super::*;
    use crate::implementations::nutrition::create_nutrition_tools;

    fn registry() -> ToolRegistry {
        let mut registry = ToolRegistry::new();
        for tool in create_nutrition_tools() {
            registry.register_with_category(Arc::from(tool), "nutrition");
        }
        registry
    }

    fn config(key: Option<&str>) -> ServerConfig {
        ServerConfig {
            usda_api_key: key.map(SecretString::from),
            ..ServerConfig::default()
        }
    }

    fn listed(key: Option<&str>) -> Vec<String> {
        let registry = registry();
        let mut schemas = registry.all_schemas();
        retain_answerable(&registry, &mut schemas, &config(key));
        schemas.into_iter().map(|schema| schema.name).collect()
    }

    #[test]
    fn a_deployment_with_no_usda_key_does_not_offer_the_food_tools() {
        for key in [None, Some("")] {
            let names = listed(key);
            for food_tool in ["search_food", "get_food_details", "analyze_meal_nutrition"] {
                assert!(
                    !names.iter().any(|name| name == food_tool),
                    "bug: {food_tool} is listed by a deployment with no USDA key ({key:?})"
                );
            }
            assert!(
                names.iter().any(|name| name == "calculate_daily_nutrition"),
                "bug: a nutrition tool that calls no food database stays listed: {names:?}"
            );
        }
    }

    #[test]
    fn a_deployment_holding_the_key_offers_them() {
        let names = listed(Some("a-usda-key"));
        assert!(
            names.iter().any(|name| name == "search_food"),
            "bug: a deployment holding the USDA key must list search_food: {names:?}"
        );
    }
}
