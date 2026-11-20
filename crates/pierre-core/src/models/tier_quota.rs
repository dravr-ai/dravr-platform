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
    /// Daily cap on billable LLM tokens: fresh prompt + cache-write prompt +
    /// completion. Prompt-cache reads are weighted by the chat pipeline's
    /// `CACHE_READ_QUOTA_WEIGHT_PERCENT`, which is zero (carnet#691).
    pub daily_tokens: i64,
    /// Weekly cap on billable LLM tokens, counted like `daily_tokens`.
    pub weekly_tokens: i64,
    /// Monthly cap on billable LLM tokens.
    pub monthly_tokens: i64,
    /// Cap on new chat conversations created per day.
    pub max_conversations_per_day: i64,
    /// Cap on messages within a single conversation (lifetime).
    pub max_messages_per_conversation: i64,
    /// Cap on messages with a given agent per day.
    pub max_messages_per_agent_per_day: i64,
    /// Cap on `agent_sessions` rows with `archived_at IS NULL`.
    pub max_active_agents: i64,
    /// Cap on data-only MCP tool calls per day (Strava fetches, etc.).
    /// These count toward `daily_tool_calls` but not toward `daily_tokens`.
    pub daily_tool_calls: i64,
    /// Weekly cap on the same data-only MCP tool calls `daily_tool_calls`
    /// counts.
    pub weekly_tool_calls: i64,
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
///
/// LIMITATION(registre#1062): `STARTER` has no monthly checked-reply cap, and its daily tool-call cap is checked on `/mcp` calls only, never by a chat turn's quota check.
pub const STARTER: TierQuotaConfig = TierQuotaConfig {
    daily_messages: 50,
    daily_tokens: 1_000_000,
    weekly_tokens: 4_000_000,
    monthly_tokens: 16_000_000,
    max_conversations_per_day: 10,
    max_messages_per_conversation: 100,
    max_messages_per_agent_per_day: 25,
    max_active_agents: 3,
    daily_tool_calls: 200,
    weekly_tool_calls: 500,
    // Starter is opt-in to upgrade — no overage billing.
    monthly_cost_cap_usd: f64::INFINITY,
};

/// Professional tier — individual paid users.
pub const PROFESSIONAL: TierQuotaConfig = TierQuotaConfig {
    daily_messages: 500,
    daily_tokens: 10_000_000,
    weekly_tokens: 50_000_000,
    monthly_tokens: 200_000_000,
    max_conversations_per_day: 100,
    max_messages_per_conversation: 1_000,
    max_messages_per_agent_per_day: 200,
    max_active_agents: 20,
    daily_tool_calls: 2_000,
    weekly_tool_calls: 500,
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
    max_messages_per_agent_per_day: i64::MAX,
    max_active_agents: i64::MAX,
    daily_tool_calls: i64::MAX,
    weekly_tool_calls: i64::MAX,
    // Enterprise contracts negotiate seat-based pricing; no auto-overage.
    monthly_cost_cap_usd: f64::INFINITY,
};

/// How many reads of an athlete's signed-in provider session Dravr makes
/// before it refuses the next one (carnet#1661).
///
/// Counted per tier and per backend: sciotte on Strava, Garmin, COROS or
/// `TrainingPeaks`.
///
/// A read the athlete is waiting on (a tool call, an app route) is `asked`;
/// work nobody is waiting on (walks, sweeps, health sync) is `background`.
/// Each has a 15-minute burst and a day. A Garmin or COROS athlete's first
/// wellness copy reads its days against `wellness_first_fill_daily` in place
/// of the background day, still inside the background burst.
///
/// These are the compiled defaults; every number is an admin parameter
/// (`sciotte_reads.*`), changed per tier with `pierre-cli config set`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionReadCaps {
    /// Reads an athlete is waiting on, per UTC day.
    pub asked_daily: u32,
    /// Reads an athlete is waiting on, per 15-minute window.
    pub asked_15min: u32,
    /// Reads nobody is waiting on, per UTC day.
    pub background_daily: u32,
    /// Reads nobody is waiting on, per 15-minute window.
    pub background_15min: u32,
    /// The days of a Garmin or COROS athlete's first wellness copy, per UTC
    /// day, counted apart from `background_daily`.
    pub wellness_first_fill_daily: u32,
}

impl SessionReadCaps {
    /// The compiled caps of `tier` (JF's numbers on carnet#1661).
    #[must_use]
    pub const fn for_tier(tier: &UserTier) -> Self {
        let (asked_daily, background_daily) = match tier {
            UserTier::Starter => (150, 300),
            UserTier::Professional => (300, 600),
            UserTier::Enterprise => (600, 1_200),
        };
        Self {
            asked_daily,
            asked_15min: 30,
            background_daily,
            background_15min: 30,
            wellness_first_fill_daily: 400,
        }
    }
}

/// The admin config category every [`SessionReadCap`] is a parameter of.
pub const SESSION_READ_CAPS_CATEGORY: &str = "sciotte_reads";

/// One number of [`SessionReadCaps`], as its admin parameter names it: one
/// parameter per tier, `sciotte_reads.<name>.<tier>`, so an override of one
/// tier never flattens the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionReadCap {
    /// [`SessionReadCaps::asked_daily`].
    AskedDaily,
    /// [`SessionReadCaps::asked_15min`].
    Asked15Min,
    /// [`SessionReadCaps::background_daily`].
    BackgroundDaily,
    /// [`SessionReadCaps::background_15min`].
    Background15Min,
    /// [`SessionReadCaps::wellness_first_fill_daily`].
    WellnessFirstFillDaily,
}

impl SessionReadCap {
    /// Every number of [`SessionReadCaps`].
    pub const ALL: [Self; 5] = [
        Self::AskedDaily,
        Self::Asked15Min,
        Self::BackgroundDaily,
        Self::Background15Min,
        Self::WellnessFirstFillDaily,
    ];

    /// The parameter's middle segment, the field's name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::AskedDaily => "asked_daily",
            Self::Asked15Min => "asked_15min",
            Self::BackgroundDaily => "background_daily",
            Self::Background15Min => "background_15min",
            Self::WellnessFirstFillDaily => "wellness_first_fill_daily",
        }
    }

    /// The number `caps` holds for this cap.
    #[must_use]
    pub const fn of(self, caps: &SessionReadCaps) -> u32 {
        match self {
            Self::AskedDaily => caps.asked_daily,
            Self::Asked15Min => caps.asked_15min,
            Self::BackgroundDaily => caps.background_daily,
            Self::Background15Min => caps.background_15min,
            Self::WellnessFirstFillDaily => caps.wellness_first_fill_daily,
        }
    }

    /// Set this cap's number in `caps` to `value`.
    pub const fn set(self, caps: &mut SessionReadCaps, value: u32) {
        match self {
            Self::AskedDaily => caps.asked_daily = value,
            Self::Asked15Min => caps.asked_15min = value,
            Self::BackgroundDaily => caps.background_daily = value,
            Self::Background15Min => caps.background_15min = value,
            Self::WellnessFirstFillDaily => caps.wellness_first_fill_daily = value,
        }
    }

    /// The admin parameter holding this cap for `tier`, e.g.
    /// `sciotte_reads.asked_daily.starter`.
    #[must_use]
    pub fn config_key(self, tier: &UserTier) -> String {
        format!(
            "{SESSION_READ_CAPS_CATEGORY}.{}.{}",
            self.name(),
            tier.as_str()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every field of `caps`, in declaration order.
    const fn fields(caps: SessionReadCaps) -> [u32; 5] {
        [
            caps.asked_daily,
            caps.asked_15min,
            caps.background_daily,
            caps.background_15min,
            caps.wellness_first_fill_daily,
        ]
    }

    #[test]
    fn a_higher_tier_never_reads_less_than_a_lower_one() {
        let ladder = [
            UserTier::Starter,
            UserTier::Professional,
            UserTier::Enterprise,
        ]
        .map(|tier| fields(SessionReadCaps::for_tier(&tier)));
        for pair in ladder.windows(2) {
            for (lower, higher) in pair[0].iter().zip(pair[1]) {
                assert!(
                    *lower <= higher,
                    "bug: a higher tier's session-read cap {higher} is below a lower tier's {lower}"
                );
            }
        }
    }

    #[test]
    fn every_burst_fits_inside_its_day() {
        for tier in [
            UserTier::Starter,
            UserTier::Professional,
            UserTier::Enterprise,
        ] {
            let caps = SessionReadCaps::for_tier(&tier);
            assert!(
                caps.asked_15min <= caps.asked_daily
                    && caps.background_15min <= caps.background_daily
                    && caps.background_15min <= caps.wellness_first_fill_daily,
                "bug: a {tier} burst is larger than the day it is spent from"
            );
        }
    }

    #[test]
    fn each_cap_sets_and_reads_its_own_field_only() {
        let mut caps = SessionReadCaps::for_tier(&UserTier::Starter);
        for (value, cap) in (1_000_u32..).zip(SessionReadCap::ALL) {
            cap.set(&mut caps, value);
        }
        let read: Vec<u32> = SessionReadCap::ALL
            .iter()
            .map(|cap| cap.of(&caps))
            .collect();
        assert_eq!(
            read,
            fields(caps).to_vec(),
            "bug: a cap reads a field other than the one it sets"
        );
        assert_eq!(
            read,
            vec![1_000, 1_001, 1_002, 1_003, 1_004],
            "bug: a cap's set landed on another cap's field"
        );
    }

    /// The keys are stored in `admin_config_overrides` rows and typed into
    /// `pierre-cli config set`: a persisted value, pinned on purpose.
    #[test]
    fn a_caps_parameter_names_its_field_and_tier() {
        assert_eq!(
            SessionReadCap::AskedDaily.config_key(&UserTier::Starter),
            "sciotte_reads.asked_daily.starter",
            "bug: the stored parameter key of a session-read cap changed"
        );
        assert_eq!(
            SessionReadCap::WellnessFirstFillDaily.config_key(&UserTier::Enterprise),
            "sciotte_reads.wellness_first_fill_daily.enterprise",
            "bug: the stored parameter key of a session-read cap changed"
        );
    }
}
