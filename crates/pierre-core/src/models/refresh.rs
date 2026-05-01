// ABOUTME: Data freshness model and refresh configuration for provider data sync
// ABOUTME: Pure value types for evaluating staleness — strategies decide when to trigger refresh
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// How fresh a piece of provider data is based on time since last sync.
///
/// This is a pure value type — it does not decide *whether* to refresh.
/// Strategies inspect the freshness and make that decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DataFreshness {
    /// Synced less than 1 hour ago
    Fresh,
    /// Synced 1–4 hours ago
    Recent,
    /// Synced 4–24 hours ago
    Stale,
    /// Synced 1–7 days ago
    Outdated,
    /// Synced more than 7 days ago, or never synced
    Expired,
}

/// One hour in seconds.
const FRESH_THRESHOLD_SECS: u64 = 3_600;
/// Four hours in seconds.
const RECENT_THRESHOLD_SECS: u64 = 14_400;
/// Twenty-four hours in seconds.
const STALE_THRESHOLD_SECS: u64 = 86_400;
/// Seven days in seconds.
const OUTDATED_THRESHOLD_SECS: u64 = 604_800;

impl DataFreshness {
    /// Determine freshness from the age of the data.
    #[must_use]
    pub fn from_age(age: Duration) -> Self {
        let secs = age.as_secs();
        if secs < FRESH_THRESHOLD_SECS {
            Self::Fresh
        } else if secs < RECENT_THRESHOLD_SECS {
            Self::Recent
        } else if secs < STALE_THRESHOLD_SECS {
            Self::Stale
        } else if secs < OUTDATED_THRESHOLD_SECS {
            Self::Outdated
        } else {
            Self::Expired
        }
    }

    /// Determine freshness from a last-sync timestamp.
    ///
    /// Returns `Expired` if `last_sync` is `None` (never synced).
    #[must_use]
    pub fn from_last_sync(last_sync: Option<DateTime<Utc>>) -> Self {
        last_sync.map_or(Self::Expired, |ts| {
            let age = Utc::now().signed_duration_since(ts);
            if age.num_seconds() < 0 {
                // Clock skew — treat as fresh
                Self::Fresh
            } else {
                #[allow(clippy::cast_sign_loss)]
                Self::from_age(Duration::from_secs(age.num_seconds() as u64))
            }
        })
    }

    /// Human-readable label for injection into coach system prompts.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Fresh => "fresh (< 1h ago)",
            Self::Recent => "recent (1–4h ago)",
            Self::Stale => "stale (4–24h ago)",
            Self::Outdated => "outdated (1–7 days ago)",
            Self::Expired => "expired (> 7 days or never synced)",
        }
    }
}

impl fmt::Display for DataFreshness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Per-provider freshness snapshot for a single user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderFreshness {
    /// Provider name (e.g. "strava", "whoop", "garmin")
    pub provider: String,
    /// When data was last successfully synced, if ever.
    pub last_sync_at: Option<DateTime<Utc>>,
    /// Computed freshness level.
    pub freshness: DataFreshness,
}

/// Summary of all connected providers' freshness for a user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshStatus {
    /// Providers currently being refreshed (sync in progress).
    pub refreshing: Vec<String>,
    /// Providers whose data is fresh enough (no refresh needed).
    pub fresh: Vec<String>,
    /// Per-provider freshness details.
    pub details: Vec<ProviderFreshness>,
}

/// Configuration controlling when and how provider data is refreshed.
///
/// Sensible defaults are provided via `Default` — 90% of tenants should
/// never need to touch these. Tenant-level overrides are available via
/// admin config for power users.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshConfig {
    /// Whether on-chat refresh triggering is enabled.
    pub on_chat_enabled: bool,
    /// Maximum data age before on-chat refresh triggers (seconds).
    pub on_chat_max_age_secs: u64,
    /// Only trigger refresh on the first message of a session.
    pub on_chat_first_message_only: bool,
    /// Inject a hint into the coach's context about data freshness.
    pub inject_coach_hint: bool,
    /// Providers eligible for refresh. Empty means all connected providers.
    pub providers: Vec<String>,
}

/// Configuration for the scheduled background sync.
///
/// Controls the polling interval, maximum acceptable data age, and concurrency
/// limits for the background sync loop that keeps provider data up to date.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledRefreshConfig {
    /// Whether scheduled background sync is enabled.
    pub enabled: bool,
    /// Base polling interval between sync cycles (seconds).
    pub poll_interval_secs: u64,
    /// Maximum data age before a provider is considered stale (seconds).
    pub max_data_age_secs: u64,
    /// Maximum number of concurrent sync operations across all users.
    pub max_concurrent_syncs: usize,
}

/// Default poll interval: 1 hour.
const DEFAULT_POLL_INTERVAL_SECS: u64 = 3_600;
/// Default max data age: 24 hours.
const DEFAULT_MAX_DATA_AGE_SECS: u64 = 86_400;
/// Default max concurrent syncs.
const DEFAULT_MAX_CONCURRENT_SYNCS: usize = 10;

impl Default for ScheduledRefreshConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            poll_interval_secs: DEFAULT_POLL_INTERVAL_SECS,
            max_data_age_secs: DEFAULT_MAX_DATA_AGE_SECS,
            max_concurrent_syncs: DEFAULT_MAX_CONCURRENT_SYNCS,
        }
    }
}

/// Weights for adjusting scheduled sync frequency based on user activity patterns.
///
/// Users who train daily benefit from more frequent syncs, while casual or
/// inactive users can be polled less often to conserve provider API quota.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartScheduleWeights {
    /// How many activities in the last 7 days indicates a "daily trainer".
    pub daily_trainer_threshold: u32,
    /// Poll interval for daily trainers (more frequent).
    pub daily_trainer_interval_secs: u64,
    /// Poll interval for casual/weekend users (less frequent).
    pub casual_interval_secs: u64,
    /// Default poll interval when activity count is unknown.
    pub default_interval_secs: u64,
}

/// Daily trainer threshold: 5 activities in 7 days.
const DEFAULT_DAILY_TRAINER_THRESHOLD: u32 = 5;
/// Daily trainer poll interval: 1 hour.
const DEFAULT_DAILY_TRAINER_INTERVAL_SECS: u64 = 3_600;
/// Casual user poll interval: 4 hours.
const DEFAULT_CASUAL_INTERVAL_SECS: u64 = 14_400;
/// Default poll interval: 2 hours.
const DEFAULT_SMART_INTERVAL_SECS: u64 = 7_200;

impl Default for SmartScheduleWeights {
    fn default() -> Self {
        Self {
            daily_trainer_threshold: DEFAULT_DAILY_TRAINER_THRESHOLD,
            daily_trainer_interval_secs: DEFAULT_DAILY_TRAINER_INTERVAL_SECS,
            casual_interval_secs: DEFAULT_CASUAL_INTERVAL_SECS,
            default_interval_secs: DEFAULT_SMART_INTERVAL_SECS,
        }
    }
}

/// Default on-chat max age: 4 hours.
const DEFAULT_ON_CHAT_MAX_AGE_SECS: u64 = 14_400;

impl Default for RefreshConfig {
    fn default() -> Self {
        Self {
            on_chat_enabled: true,
            on_chat_max_age_secs: DEFAULT_ON_CHAT_MAX_AGE_SECS,
            on_chat_first_message_only: true,
            inject_coach_hint: true,
            providers: Vec::new(),
        }
    }
}

impl RefreshConfig {
    /// Maximum data age as a `Duration`.
    #[must_use]
    pub fn on_chat_max_age(&self) -> Duration {
        Duration::from_secs(self.on_chat_max_age_secs)
    }

    /// Whether the given provider is eligible for refresh under this config.
    #[must_use]
    pub fn is_provider_eligible(&self, provider: &str) -> bool {
        self.providers.is_empty() || self.providers.iter().any(|p| p == provider)
    }

    /// Whether a provider at the given freshness should be refreshed on chat.
    #[must_use]
    pub fn should_refresh_on_chat(&self, age: Duration) -> bool {
        self.on_chat_enabled && age > self.on_chat_max_age()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn freshness_from_age() {
        assert_eq!(
            DataFreshness::from_age(Duration::from_secs(0)),
            DataFreshness::Fresh
        );
        assert_eq!(
            DataFreshness::from_age(Duration::from_mins(30)),
            DataFreshness::Fresh
        );
        assert_eq!(
            DataFreshness::from_age(Duration::from_hours(1)),
            DataFreshness::Recent
        );
        assert_eq!(
            DataFreshness::from_age(Duration::from_hours(2)),
            DataFreshness::Recent
        );
        assert_eq!(
            DataFreshness::from_age(Duration::from_hours(4)),
            DataFreshness::Stale
        );
        assert_eq!(
            DataFreshness::from_age(Duration::from_hours(24)),
            DataFreshness::Outdated
        );
        assert_eq!(
            DataFreshness::from_age(Duration::from_hours(24 * 7)),
            DataFreshness::Expired
        );
    }

    #[test]
    fn freshness_from_none_is_expired() {
        assert_eq!(DataFreshness::from_last_sync(None), DataFreshness::Expired);
    }

    #[test]
    fn freshness_from_recent_timestamp() {
        let two_hours_ago = Utc::now() - chrono::Duration::hours(2);
        assert_eq!(
            DataFreshness::from_last_sync(Some(two_hours_ago)),
            DataFreshness::Recent
        );
    }

    #[test]
    fn config_defaults_are_sensible() {
        let cfg = RefreshConfig::default();
        assert!(cfg.on_chat_enabled);
        assert_eq!(cfg.on_chat_max_age_secs, 14_400);
        assert!(cfg.on_chat_first_message_only);
        assert!(cfg.inject_coach_hint);
        assert!(cfg.providers.is_empty());
    }

    #[test]
    fn provider_eligibility() {
        let cfg = RefreshConfig::default();
        assert!(cfg.is_provider_eligible("strava")); // empty list = all eligible

        let cfg = RefreshConfig {
            providers: vec!["strava".to_owned(), "whoop".to_owned()],
            ..Default::default()
        };
        assert!(cfg.is_provider_eligible("strava"));
        assert!(!cfg.is_provider_eligible("garmin"));
    }

    #[test]
    fn should_refresh_on_chat() {
        let cfg = RefreshConfig::default();
        assert!(!cfg.should_refresh_on_chat(Duration::from_hours(1))); // 1h < 4h threshold
        assert!(cfg.should_refresh_on_chat(Duration::from_secs(14_401))); // > 4h

        let disabled = RefreshConfig {
            on_chat_enabled: false,
            ..Default::default()
        };
        assert!(!disabled.should_refresh_on_chat(Duration::from_secs(999_999)));
    }

    #[test]
    fn scheduled_refresh_config_defaults() {
        let cfg = ScheduledRefreshConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.poll_interval_secs, 3_600);
        assert_eq!(cfg.max_data_age_secs, 86_400);
        assert_eq!(cfg.max_concurrent_syncs, 10);
    }

    #[test]
    fn smart_schedule_weights_defaults() {
        let w = SmartScheduleWeights::default();
        assert_eq!(w.daily_trainer_threshold, 5);
        assert_eq!(w.daily_trainer_interval_secs, 3_600);
        assert_eq!(w.casual_interval_secs, 14_400);
        assert_eq!(w.default_interval_secs, 7_200);
    }
}
