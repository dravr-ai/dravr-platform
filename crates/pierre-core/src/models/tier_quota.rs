// ABOUTME: Tier-keyed quota defaults for Starter / Professional / Enterprise users
// ABOUTME: Consumed by UsageCounterService as the default_limit() fallback before admin_config_overrides
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::{Deserialize, Serialize};

use crate::models::user::UserTier;

/// Per-tier quota defaults. Populated from the canonical table below
/// at startup, then layered under any `admin_config_overrides` row
/// that targets the same `counter_key` for the same scope.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct TierQuotaConfig {
    /// Daily cap on user-originated chat messages.
    pub daily_messages: i64,
    /// Daily cap on billable LLM tokens (prompt + completion, excluding cached).
    pub daily_tokens: i64,
    /// Weekly cap on billable LLM tokens.
    pub weekly_tokens: i64,
    /// Monthly cap on billable LLM tokens.
    pub monthly_tokens: i64,
    /// Cap on new chat conversations created per day.
    pub max_conversations_per_day: i64,
    /// Cap on messages within a single conversation (lifetime).
    pub max_messages_per_conversation: i64,
    /// Cap on messages with a given coach per day.
    pub max_messages_per_coach_per_day: i64,
    /// Cap on `coach_sessions` rows with `archived_at IS NULL`.
    pub max_active_coaches: i64,
    /// Cap on data-only MCP tool calls per day (Strava fetches, etc.).
    /// These count toward `daily_tool_calls` but not toward `daily_tokens`.
    pub daily_tool_calls: i64,
    /// Monthly cost cap in USD. Tenant LLM spend that exceeds this number
    /// is reported to Stripe as a metered overage event by the billing
    /// cron. `f64::INFINITY` disables overage billing for the tier.
    pub monthly_cost_cap_usd: f64,
}

impl TierQuotaConfig {
    /// Resolve the default quota config for a given tier.
    #[must_use]
    pub const fn for_tier(tier: &UserTier) -> Self {
        match tier {
            UserTier::Starter => STARTER,
            UserTier::Professional => PROFESSIONAL,
            UserTier::Enterprise => ENTERPRISE,
        }
    }
}

/// Starter tier — free / evaluation usage limits.
pub const STARTER: TierQuotaConfig = TierQuotaConfig {
    daily_messages: 50,
    daily_tokens: 500_000,
    weekly_tokens: 2_000_000,
    monthly_tokens: 8_000_000,
    max_conversations_per_day: 10,
    max_messages_per_conversation: 100,
    max_messages_per_coach_per_day: 25,
    max_active_coaches: 3,
    daily_tool_calls: 200,
    // Starter is opt-in to upgrade — no overage billing.
    monthly_cost_cap_usd: f64::INFINITY,
};

/// Professional tier — individual paid users.
pub const PROFESSIONAL: TierQuotaConfig = TierQuotaConfig {
    daily_messages: 500,
    daily_tokens: 5_000_000,
    weekly_tokens: 25_000_000,
    monthly_tokens: 100_000_000,
    max_conversations_per_day: 100,
    max_messages_per_conversation: 1_000,
    max_messages_per_coach_per_day: 200,
    max_active_coaches: 20,
    daily_tool_calls: 2_000,
    // $50/month included; spillover billed via Stripe Meters.
    monthly_cost_cap_usd: 50.0,
};

/// Enterprise tier — uncapped in practice; sentinel `i64::MAX` makes the
/// defaults compose with the standard "if counter >= limit reject" path
/// without branching on tier.
pub const ENTERPRISE: TierQuotaConfig = TierQuotaConfig {
    daily_messages: i64::MAX,
    daily_tokens: i64::MAX,
    weekly_tokens: i64::MAX,
    monthly_tokens: i64::MAX,
    max_conversations_per_day: i64::MAX,
    max_messages_per_conversation: i64::MAX,
    max_messages_per_coach_per_day: i64::MAX,
    max_active_coaches: i64::MAX,
    daily_tool_calls: i64::MAX,
    // Enterprise contracts negotiate seat-based pricing; no auto-overage.
    monthly_cost_cap_usd: f64::INFINITY,
};
