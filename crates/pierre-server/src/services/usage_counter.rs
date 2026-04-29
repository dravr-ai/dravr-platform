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
use pierre_core::models::{TierQuotaConfig, UserTier};
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
    ///
    /// # Errors
    /// Returns an error if repository operations fail.
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
    ///
    /// # Errors
    /// Returns an error if repository operations fail.
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
    /// Defaults to `UserTier::Starter` for the tier-based fallback —
    /// least-permissive choice when the caller doesn't have the tier
    /// resolved. Use [`Self::check_limit_for_tier`] to pass the
    /// caller's actual tier through.
    ///
    /// # Errors
    /// Returns an error if repository operations fail.
    pub async fn check_limit(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_type: &str,
    ) -> AppResult<LimitCheckResult> {
        self.check_limit_for_tier(tenant_id, user_id, counter_type, &UserTier::Starter)
            .await
    }

    /// Tier-aware limit check. Phase 3 chat write paths that have
    /// already resolved the user's [`UserTier`] should always go
    /// through this variant so [`TierQuotaConfig`] caps apply before
    /// any global admin-config fallback.
    ///
    /// # Errors
    /// Returns an error if repository operations fail.
    pub async fn check_limit_for_tier(
        &self,
        tenant_id: &str,
        user_id: &str,
        counter_type: &str,
        tier: &UserTier,
    ) -> AppResult<LimitCheckResult> {
        let config_key = counter_type_to_config_key(counter_type);
        let limit = self
            .read_limit_override_i64(&config_key, Some(tenant_id))
            .await?
            .unwrap_or_else(|| default_limit(counter_type, tier));

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

    /// Tier-aware limit check scoped to a sub-dimension (per
    /// conversation, per coach, per tool).
    ///
    /// The counter is stored under a synthetic `counter_type` of the
    /// form `<base>:<dimension>` so each scope is tracked
    /// independently while still sharing the tier-keyed default cap
    /// of `<base>`. Period and reset semantics follow `<base>`'s
    /// daily/weekly prefix.
    ///
    /// # Errors
    /// Returns an error if repository operations fail.
    pub async fn check_limit_with_dimension_for_tier(
        &self,
        tenant_id: &str,
        user_id: &str,
        base_counter_type: &str,
        dimension: &str,
        tier: &UserTier,
    ) -> AppResult<LimitCheckResult> {
        let scoped_counter_type = format!("{base_counter_type}:{dimension}");
        let config_key = counter_type_to_config_key(base_counter_type);
        let limit = self
            .read_config_i64(&config_key, Some(tenant_id))
            .await?
            .unwrap_or_else(|| default_limit(base_counter_type, tier));

        let warning_pct = self
            .read_config_i64("usage_quotas.warning_threshold_percent", Some(tenant_id))
            .await?
            .unwrap_or(80);

        let burst_multiplier = self
            .read_config_f64("usage_quotas.burst_multiplier", Some(tenant_id))
            .await?
            .unwrap_or(1.5);

        let current = self
            .get_current(tenant_id, user_id, &scoped_counter_type)
            .await?;

        #[allow(clippy::cast_precision_loss)]
        let hard_limit = (limit as f64 * burst_multiplier) as i64;
        #[allow(clippy::cast_precision_loss)]
        let warning_threshold = (limit as f64 * warning_pct as f64 / 100.0) as i64;

        let allowed = current < hard_limit;
        let burst_zone = current >= limit;
        let warning = current >= warning_threshold;

        let resets_at = next_reset_time(base_counter_type);

        debug!(
            tenant_id,
            user_id,
            base_counter_type,
            dimension,
            current,
            limit,
            hard_limit,
            allowed,
            burst_zone,
            "Scoped usage limit check"
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

    /// Increment a scoped counter (per conversation / coach / tool)
    /// using the same `<base>:<dimension>` synthetic `counter_type` the
    /// dimensioned limit check inspects.
    ///
    /// # Errors
    /// Returns an error if repository operations fail.
    pub async fn increment_with_dimension(
        &self,
        tenant_id: &str,
        user_id: &str,
        base_counter_type: &str,
        dimension: &str,
        amount: i64,
    ) -> AppResult<i64> {
        let scoped_counter_type = format!("{base_counter_type}:{dimension}");
        self.increment(tenant_id, user_id, &scoped_counter_type, amount)
            .await
    }

    /// Delete counters older than the given number of days
    ///
    /// Computes a cutoff date and removes all counter records with periods before it.
    ///
    /// # Errors
    /// Returns an error if repository operations fail.
    pub async fn prune_old_counters(&self, days: i64) -> AppResult<u64> {
        let cutoff = Utc::now() - Duration::days(days);
        let cutoff_str = cutoff.format("%Y-%m-%d").to_string();
        let deleted = self.repo.delete_old_counters(&cutoff_str).await?;
        Ok(deleted)
    }

    /// Read an integer config value (falls back to parameter
    /// definition default when no override exists).
    async fn read_config_i64(&self, key: &str, tenant_id: Option<&str>) -> AppResult<Option<i64>> {
        Ok(self
            .config
            .get_value(key, tenant_id)
            .await?
            .and_then(|v| v.as_i64()))
    }

    /// Read an explicit integer override only — returns `None` when
    /// no admin override is set so the caller can fall back to the
    /// tier-keyed default in [`pierre_core::models::TierQuotaConfig`].
    /// The flat catalog default (e.g. `usage_quotas.daily_message_cap
    /// = 50`) would otherwise shadow Professional / Enterprise caps.
    async fn read_limit_override_i64(
        &self,
        key: &str,
        tenant_id: Option<&str>,
    ) -> AppResult<Option<i64>> {
        Ok(self
            .config
            .get_override_value(key, tenant_id)
            .await?
            .and_then(|v| v.as_i64()))
    }

    /// Read a float config value (falls back to parameter definition
    /// default when no override exists).
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

/// Default limits when config has no value for a counter type.
///
/// Tier-keyed counters resolve through [`TierQuotaConfig::for_tier`] so
/// Starter / Professional / Enterprise defaults stay in lockstep with
/// the single source of truth in `pierre_core::models::tier_quota`.
/// Counter types not yet represented in [`TierQuotaConfig`] keep their
/// historical defaults so admin overrides remain the only knob.
fn default_limit(counter_type: &str, tier: &UserTier) -> i64 {
    let q = TierQuotaConfig::for_tier(tier);
    match counter_type {
        "daily_messages" => q.daily_messages,
        "daily_tokens" => q.daily_tokens,
        "weekly_tokens" => q.weekly_tokens,
        "monthly_tokens" => q.monthly_tokens,
        "daily_tool_calls" => q.daily_tool_calls,
        "daily_conversations" => q.max_conversations_per_day,
        "conversation_messages" => q.max_messages_per_conversation,
        "daily_coach_messages" => q.max_messages_per_coach_per_day,
        "active_coaches" => q.max_active_coaches,
        // Counter types we have not yet keyed to tier fall back to the
        // historical defaults; admins can override via admin_config.
        "weekly_messages" => 250,
        "weekly_tool_calls" | "weekly_activity_summary" => 500,
        "daily_activity_detailed" => 20,
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
