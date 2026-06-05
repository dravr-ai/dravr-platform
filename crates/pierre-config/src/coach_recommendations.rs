// ABOUTME: Tunable knobs for personalized coach recommendations (activity scan + scoring)
// ABOUTME: Env-backed so the curated coach set can be retuned without a code change
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use serde::{Deserialize, Serialize};
use std::env;
use std::str::FromStr;

/// Default values for the coach-recommendation tuning knobs.
mod defaults {
    /// Look-back window (days) for the recommendation activity scan.
    pub const WINDOW_DAYS: u32 = 90;
    /// Max activities pulled per connected provider during the scan.
    pub const ACTIVITY_LIMIT_PER_PROVIDER: usize = 50;
    /// How long a computed sport profile stays cached (seconds).
    pub const PROFILE_TTL_SECS: u64 = 6 * 60 * 60;
    /// Minimum activity count for a sport to count as "active".
    pub const MIN_ACTIVITIES_FOR_SPORT: u32 = 3;
    /// Minimum share of total activities for a sport to count as "active".
    pub const MIN_SHARE_FOR_SPORT: f32 = 0.15;
    /// Recommendation score for a sport-agnostic coach once its providers are met.
    pub const SPORT_AGNOSTIC_BASE_SCORE: f32 = 0.5;
    /// Multiplier applied to a sport match when the user is below `min_activities`.
    pub const BELOW_MIN_ACTIVITIES_PENALTY: f32 = 0.5;
    /// Maximum coaches surfaced in the "Recommended for you" set.
    pub const MAX_RECOMMENDED: usize = 3;
    /// Candidate pool size the deterministic prefilter hands to the LLM
    /// re-ranking step. Larger gives the model more to choose from at the
    /// cost of a longer prompt; the model still returns at most
    /// `MAX_RECOMMENDED`.
    pub const RERANK_POOL_SIZE: usize = 8;
}

/// Tuning knobs for the personalized coach recommender.
///
/// Every field is overridable via a `COACH_REC_*` environment variable (set in
/// `.envrc`), so the curated set can be retuned in production without a deploy
/// of new code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoachRecommendationConfig {
    /// Look-back window (days) for the recommendation activity scan.
    pub window_days: u32,
    /// Max activities pulled per connected provider during the scan.
    pub activity_limit_per_provider: usize,
    /// How long a computed sport profile stays cached (seconds).
    pub profile_ttl_secs: u64,
    /// Minimum activity count for a sport to count as "active".
    pub min_activities_for_sport: u32,
    /// Minimum share of total activities for a sport to count as "active".
    pub min_share_for_sport: f32,
    /// Recommendation score for a sport-agnostic coach once its providers are met.
    pub sport_agnostic_base_score: f32,
    /// Multiplier applied to a sport match when the user is below `min_activities`.
    pub below_min_activities_penalty: f32,
    /// Maximum coaches surfaced in the "Recommended for you" set.
    pub max_recommended: usize,
    /// Candidate pool size handed to the LLM re-ranking step.
    pub rerank_pool_size: usize,
}

impl Default for CoachRecommendationConfig {
    fn default() -> Self {
        Self {
            window_days: defaults::WINDOW_DAYS,
            activity_limit_per_provider: defaults::ACTIVITY_LIMIT_PER_PROVIDER,
            profile_ttl_secs: defaults::PROFILE_TTL_SECS,
            min_activities_for_sport: defaults::MIN_ACTIVITIES_FOR_SPORT,
            min_share_for_sport: defaults::MIN_SHARE_FOR_SPORT,
            sport_agnostic_base_score: defaults::SPORT_AGNOSTIC_BASE_SCORE,
            below_min_activities_penalty: defaults::BELOW_MIN_ACTIVITIES_PENALTY,
            max_recommended: defaults::MAX_RECOMMENDED,
            rerank_pool_size: defaults::RERANK_POOL_SIZE,
        }
    }
}

impl CoachRecommendationConfig {
    /// Load the configuration from `COACH_REC_*` environment variables,
    /// falling back to defaults for any unset or unparseable value.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            window_days: parse_env("COACH_REC_WINDOW_DAYS", defaults::WINDOW_DAYS),
            activity_limit_per_provider: parse_env(
                "COACH_REC_ACTIVITY_LIMIT_PER_PROVIDER",
                defaults::ACTIVITY_LIMIT_PER_PROVIDER,
            ),
            profile_ttl_secs: parse_env("COACH_REC_PROFILE_TTL_SECS", defaults::PROFILE_TTL_SECS),
            min_activities_for_sport: parse_env(
                "COACH_REC_MIN_ACTIVITIES_FOR_SPORT",
                defaults::MIN_ACTIVITIES_FOR_SPORT,
            ),
            min_share_for_sport: parse_env(
                "COACH_REC_MIN_SHARE_FOR_SPORT",
                defaults::MIN_SHARE_FOR_SPORT,
            ),
            sport_agnostic_base_score: parse_env(
                "COACH_REC_SPORT_AGNOSTIC_BASE_SCORE",
                defaults::SPORT_AGNOSTIC_BASE_SCORE,
            ),
            below_min_activities_penalty: parse_env(
                "COACH_REC_BELOW_MIN_ACTIVITIES_PENALTY",
                defaults::BELOW_MIN_ACTIVITIES_PENALTY,
            ),
            max_recommended: parse_env("COACH_REC_MAX_RECOMMENDED", defaults::MAX_RECOMMENDED),
            rerank_pool_size: parse_env("COACH_REC_RERANK_POOL_SIZE", defaults::RERANK_POOL_SIZE),
        }
    }
}

/// Parse an environment variable into `T`, falling back to `default` when the
/// variable is unset or fails to parse.
fn parse_env<T: FromStr>(key: &str, default: T) -> T {
    env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
