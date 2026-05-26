// ABOUTME: pierre-server-local configuration submodules (admin + HTTP routes)
// ABOUTME: Data-config lives in pierre-config; admin overrides + routes need server-internal types
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Configuration module for Pierre MCP Server
//!
//! The data-only configuration types live in the `pierre-config` crate.
//! This module hosts the submodules that remain in `pierre-server` because
//! they need server-internal types (`ServerContext`, route handlers,
//! `AdminConfigService`).

use pierre_config::social;
use pierre_core::errors::{AppError, AppResult};
use pierre_intelligence::config::intelligence::IntelligenceConfig;
use tracing::{debug, info};

/// Admin configuration management with runtime parameter overrides
pub mod admin;
/// HTTP routes for configuration management
pub mod routes;

/// Initialize all configurations
///
/// # Errors
///
/// Returns an error if configuration initialization fails
pub fn init_configs() -> AppResult<()> {
    // Validate the layered intelligence config fails fast for bad env
    // overrides. The canonical live snapshot is owned by
    // `ServerContext::cageux_config_registry`; this call is a
    // start-of-process sanity check, not a storage seed.
    let intelligence_config = IntelligenceConfig::load()
        .map_err(|e| AppError::internal(format!("Intelligence config load failed: {e}")))?;

    debug!(
        "Intelligence config validated (min duration: {}s)",
        intelligence_config
            .activity_analyzer
            .analysis
            .min_duration_seconds
    );

    // Initialize global social insights config
    let social_config = social::global();
    debug!(
        "Social insights config initialized successfully (activity limit: {})",
        social_config.activity_fetch_limits.insight_context_limit
    );

    info!("All configurations initialized successfully");
    Ok(())
}
