// ABOUTME: Social insights configuration for coach-mediated sharing features
// ABOUTME: Configurable thresholds for milestones, streaks, and relevance scoring
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Social Insights Configuration Module
//!
//! Provides configurable thresholds and scoring for social insight generation.
//! All values can be overridden via environment variables with the `SOCIAL_INSIGHTS_` prefix.
//!
//! Type definitions are in `pierre_core::config::social` and re-exported here.
//! Runtime functionality (global singleton, env loading) lives in this module.

pub use pierre_core::config::social::*;

use std::env;
use std::str::FromStr;
use std::sync::OnceLock;
use tracing::warn;

use super::intelligence::ConfigError;

/// Global configuration singleton
static SOCIAL_INSIGHTS_CONFIG: OnceLock<SocialInsightsConfig> = OnceLock::new();

/// Get the global social insights configuration instance
pub fn global() -> &'static SocialInsightsConfig {
    SOCIAL_INSIGHTS_CONFIG.get_or_init(|| {
        load().unwrap_or_else(|e| {
            warn!("Failed to load social insights config: {e}, using defaults");
            SocialInsightsConfig::default()
        })
    })
}

/// Load configuration from environment
///
/// # Errors
///
/// Returns an error if environment variables contain invalid values or validation fails
fn load() -> Result<SocialInsightsConfig, ConfigError> {
    let mut config = SocialInsightsConfig::default();
    config = apply_env_overrides(config)?;
    // validate() returns AppResult; convert to ConfigError for consistency with load()
    config
        .validate()
        .map_err(|e| ConfigError::Parse(e.to_string()))?;
    Ok(config)
}

/// Validate the configuration using `ConfigError` (for environment loading context)
///
/// # Errors
///
/// Returns an error if configuration values are invalid
pub fn validate_with_config_error(config: &SocialInsightsConfig) -> Result<(), ConfigError> {
    config
        .validate()
        .map_err(|e| ConfigError::Parse(e.to_string()))
}

/// Helper function to parse and apply an environment variable override
fn apply_env_var<T: FromStr>(env_var_name: &str, target: &mut T) -> Result<(), ConfigError> {
    if let Ok(val) = env::var(env_var_name) {
        *target = val
            .parse()
            .map_err(|_| ConfigError::Parse(format!("Invalid {env_var_name}")))?;
    }
    Ok(())
}

/// Apply environment variable overrides
fn apply_env_overrides(
    mut config: SocialInsightsConfig,
) -> Result<SocialInsightsConfig, ConfigError> {
    // Milestone thresholds
    apply_env_var(
        "SOCIAL_INSIGHTS_MIN_ACTIVITIES_FOR_MILESTONE",
        &mut config.milestone_thresholds.min_activities_for_milestone,
    )?;

    // Streak configuration
    apply_env_var(
        "SOCIAL_INSIGHTS_STREAK_LOOKBACK_DAYS",
        &mut config.streak_config.lookback_days,
    )?;
    apply_env_var(
        "SOCIAL_INSIGHTS_STREAK_MIN_FOR_SHARING",
        &mut config.streak_config.min_for_sharing,
    )?;

    // Activity fetch limits
    apply_env_var(
        "SOCIAL_INSIGHTS_ACTIVITY_LIMIT",
        &mut config.activity_fetch_limits.insight_context_limit,
    )?;
    apply_env_var(
        "SOCIAL_INSIGHTS_TRAINING_CONTEXT_LIMIT",
        &mut config.activity_fetch_limits.training_context_limit,
    )?;
    apply_env_var(
        "SOCIAL_INSIGHTS_MAX_CLIENT_LIMIT",
        &mut config.activity_fetch_limits.max_client_limit,
    )?;

    // Min relevance score
    apply_env_var(
        "SOCIAL_INSIGHTS_MIN_RELEVANCE_SCORE",
        &mut config.min_relevance_score,
    )?;

    Ok(config)
}
