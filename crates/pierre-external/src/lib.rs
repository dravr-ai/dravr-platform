// ABOUTME: External API client modules (USDA FoodData Central)
// ABOUTME: Provides nutritional data integration and caching

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! External API Clients
//!
//! This module contains clients for external APIs used by the pierre MCP server.

/// USDA FoodData Central API client for nutritional data
pub mod usda_client;

/// Per-100 g figures read from a USDA food, and the generic-entry pick
pub mod usda_nutrients;

/// Re-export commonly used types from USDA client
pub use usda_client::{
    FoodDetails, FoodNutrient, FoodSearchPaginatedResponse, FoodSearchResult, UsdaClient,
    UsdaClientConfig,
};
