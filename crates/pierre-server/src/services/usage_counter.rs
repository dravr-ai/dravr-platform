// ABOUTME: Usage counter service for quota enforcement with burst zones and warnings
// ABOUTME: Wraps UsageCounterRepository with config-driven limits, period computation, and pruning
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Usage Counter Service
//!
//! Provides quota-aware operations on top of raw counter storage. Handles:
//! - Automatic period computation (daily = `YYYY-MM-DD`, weekly = most recent Sunday)
//! - Limit checking with warning thresholds and burst zones
//! - Counter pruning for old periods

use chrono::{Datelike, Duration, Utc, Weekday};
use serde::Serialize;
use tracing::debug;

use crate::config::admin::service::AdminConfigService;
use crate::errors::AppResult;
use pierre_database::database::repositories::UsageCounterRepository;

/// Result of a limit check against a usage counter
#[derive(Debug, Clone, Serialize)]
pub struct LimitCheckResult {
    /// Whether the request is allowed (under hard limit)
    pub allowed: bool,
    /// Current counter value
    pub current: i64,
    /// Configured soft limit
    pub limit: i64,
    /// Whether the user is approaching the limit (at or above warning threshold)
    pub warning: bool,
    /// Whether the user is in the burst zone (between limit and hard limit)
    pub burst_zone: bool,
    /// ISO 8601 timestamp when the counter resets
    pub resets_at: String,
}

/// Service for managing usage counters with config-driven quotas
pub struct UsageCounterService<'a> {
    repo: &'a dyn UsageCounterRepository,
    config: &'a AdminConfigService,
}

impl<'a> UsageCounterService<'a> {
    /// Create a new service with the given repository and config
    #[must_use]
    pub fn new(repo: &'a dyn UsageCounterRepository, config: &'a AdminConfigService) -> Self {
        Self { repo, config }
    }

    /// Increment a counter and return the new value
    ///
    /// Automatically computes the current period based on `counter_type` prefix:
    /// - `daily_*` → `YYYY-MM-DD`
    /// - `weekly_*` → most recent Sunday `YYYY-MM-DD`
    pub async fn increment(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_type: &str,
        amount: i64,
    ) -> AppResult<i64> {
        let period = current_period(counter_type);
        let record = self
            .repo
            .increment_counter(tenant_id, user_id, counter_type, &period, amount)
            .await?;
        Ok(record.value)
    }

    /// Get the current value of a counter for the current period
    ///
    /// Returns 0 if no counter exists for the current period.
    pub async fn get_current(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_type: &str,
    ) -> AppResult<i64> {
        let period = current_period(counter_type);
        let record = self
            .repo
            .get_counter(tenant_id, user_id, counter_type, &period)
            .await?;
        Ok(record.value)
    }

    /// Check whether a counter is within its configured limits
    ///
    /// Uses the admin config to look up:
    /// - The soft limit for this counter type
    /// - The burst multiplier (hard limit = soft limit * multiplier)
    /// - The warning threshold percentage
    pub async fn check_limit(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_type: &str,
    ) -> AppResult<LimitCheckResult> {
        let config_key = counter_type_to_config_key(counter_type);
        let limit = self
            .read_config_i64(&config_key, Some(tenant_id))
            .await?
            .unwrap_or_else(|| default_limit(counter_type));

        let warning_pct = self
            .read_config_i64("usage_quotas.warning_threshold_percent", Some(tenant_id))
            .await?
            .unwrap_or(80);

        let burst_multiplier = self
            .read_config_f64("usage_quotas.burst_multiplier", Some(tenant_id))
            .await?
            .unwrap_or(1.5);

        let current = self.get_current(tenant_id, user_id, counter_type).await?;

        #[allow(clippy::cast_precision_loss)]
        let hard_limit = (limit as f64 * burst_multiplier) as i64;
        #[allow(clippy::cast_precision_loss)]
        let warning_threshold = (limit as f64 * warning_pct as f64 / 100.0) as i64;

        let allowed = current < hard_limit;
        let burst_zone = current >= limit;
        let warning = current >= warning_threshold;

        let resets_at = next_reset_time(counter_type);

        debug!(
            tenant_id,
            user_id,
            counter_type,
            current,
            limit,
            hard_limit,
            allowed,
            burst_zone,
            "Usage limit check"
        );

        Ok(LimitCheckResult {
            allowed,
            current,
            limit,
            warning,
            burst_zone,
            resets_at,
        })
    }

    /// Delete counters older than the given number of days
    ///
    /// Computes a cutoff date and removes all counter records with periods before it.
    pub async fn prune_old_counters(&self, days: i64) -> AppResult<u64> {
        let cutoff = Utc::now() - Duration::days(days);
        let cutoff_str = cutoff.format("%Y-%m-%d").to_string();
        let deleted = self.repo.delete_old_counters(&cutoff_str).await?;
        Ok(deleted)
    }

    /// Read an integer config value
    async fn read_config_i64(&self, key: &str, tenant_id: Option<&str>) -> AppResult<Option<i64>> {
        Ok(self
            .config
            .get_value(key, tenant_id)
            .await?
            .and_then(|v| v.as_i64()))
    }

    /// Read a float config value
    async fn read_config_f64(&self, key: &str, tenant_id: Option<&str>) -> AppResult<Option<f64>> {
        Ok(self
            .config
            .get_value(key, tenant_id)
            .await?
            .and_then(|v| v.as_f64()))
    }
}

/// Compute the current period string for a counter type
///
/// - `daily_*` → `YYYY-MM-DD` (today)
/// - `weekly_*` → `YYYY-MM-DD` (most recent Sunday, or today if Sunday)
fn current_period(counter_type: &str) -> String {
    let now = Utc::now();

    if counter_type.starts_with("weekly_") {
        let days_since_sunday = now.weekday().num_days_from_sunday();
        let sunday = now - Duration::days(i64::from(days_since_sunday));
        sunday.format("%Y-%m-%d").to_string()
    } else {
        // Default to daily period for daily_* and any unrecognized prefix
        now.format("%Y-%m-%d").to_string()
    }
}

/// Map counter type to its admin config key for the limit value
fn counter_type_to_config_key(counter_type: &str) -> String {
    let param = match counter_type {
        "daily_messages" => "daily_message_cap",
        "weekly_messages" => "weekly_message_cap",
        "daily_tool_calls" => "daily_tool_call_limit",
        "weekly_tool_calls" => "weekly_tool_call_limit",
        "daily_tokens" => "daily_token_budget",
        "weekly_tokens" => "weekly_token_budget",
        "daily_activity_summary" => "daily_activity_summary_limit",
        "weekly_activity_summary" => "weekly_activity_summary_limit",
        "daily_activity_detailed" => "daily_activity_detailed_limit",
        "weekly_activity_detailed" => "weekly_activity_detailed_limit",
        other => other,
    };
    format!("usage_quotas.{param}")
}

/// Default limits when config has no value for a counter type
fn default_limit(counter_type: &str) -> i64 {
    match counter_type {
        "daily_messages" => 50,
        "weekly_messages" => 250,
        "weekly_tool_calls" | "weekly_activity_summary" => 500,
        "daily_tokens" => 500_000,
        "weekly_tokens" => 2_000_000,
        "daily_activity_detailed" => 20,
        // daily_tool_calls, daily_activity_summary, weekly_activity_detailed,
        // and unrecognized counter types default to 100
        _ => 100,
    }
}

/// Compute the next reset time for a counter type
///
/// - `daily_*` → next midnight UTC
/// - `weekly_*` → next Sunday midnight UTC
fn next_reset_time(counter_type: &str) -> String {
    let now = Utc::now();

    let days_ahead = if counter_type.starts_with("weekly_") {
        match now.weekday() {
            Weekday::Sun => 7,
            other => 7 - other.num_days_from_sunday(),
        }
    } else {
        1
    };

    let reset_date = (now + Duration::days(i64::from(days_ahead))).date_naive();
    format!("{reset_date}T00:00:00Z")
}
