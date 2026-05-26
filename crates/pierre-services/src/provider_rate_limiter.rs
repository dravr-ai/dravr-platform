// ABOUTME: App-wide rate limiter for external fitness provider APIs
// ABOUTME: Prevents exceeding Strava/Fitbit/Garmin/WHOOP rate limits across all tenants
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use pierre_core::constants::rate_limits;
use tracing::{info, warn};

/// Tracks API call counts per provider within sliding windows.
/// Shared across all tenants to enforce app-wide provider rate limits.
///
/// Uses [`DashMap`] for lock-free concurrent access from multiple tokio tasks.
pub struct ProviderRateLimiter {
    /// Map of `provider_name` -> (`call_count`, `window_start`).
    windows: Arc<DashMap<String, (u32, Instant)>>,
    /// Map of `provider_name` -> (`max_calls`, `window_duration`).
    limits: Arc<DashMap<String, (u32, Duration)>>,
}

/// One day as a `Duration`.
const ONE_DAY: Duration = Duration::from_hours(24);

/// Result of a rate limit check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimitStatus {
    /// The call is within the provider's rate limit.
    Allowed,
    /// The rate limit has been exceeded; retry after the given duration.
    Exceeded {
        /// Duration until the current rate-limit window resets.
        retry_after: Duration,
    },
}

impl ProviderRateLimiter {
    /// Create a rate limiter pre-loaded with known provider rate limits.
    #[must_use]
    pub fn new() -> Self {
        let limits = DashMap::new();

        // Strava: 15k requests per day (use daily window for app-wide safety margin)
        limits.insert(
            "strava".to_owned(),
            (rate_limits::STRAVA_DEFAULT_DAILY_RATE_LIMIT, ONE_DAY),
        );
        // Fitbit: 1k requests per day
        limits.insert(
            "fitbit".to_owned(),
            (rate_limits::FITBIT_DEFAULT_DAILY_RATE_LIMIT, ONE_DAY),
        );
        // Garmin: 1k requests per day
        limits.insert(
            "garmin".to_owned(),
            (rate_limits::GARMIN_DEFAULT_DAILY_RATE_LIMIT, ONE_DAY),
        );
        // WHOOP: 1k requests per day
        limits.insert(
            "whoop".to_owned(),
            (rate_limits::WHOOP_DEFAULT_DAILY_RATE_LIMIT, ONE_DAY),
        );
        // Terra: 1k requests per day
        limits.insert(
            "terra".to_owned(),
            (rate_limits::TERRA_DEFAULT_DAILY_RATE_LIMIT, ONE_DAY),
        );

        info!(
            provider_count = limits.len(),
            "Provider rate limiter initialized"
        );

        Self {
            windows: Arc::new(DashMap::new()),
            limits: Arc::new(limits),
        }
    }

    /// Check whether a call to the given provider is within the rate limit.
    ///
    /// Returns `Allowed` if no limit is configured or the window has not been
    /// exhausted. Returns `Exceeded` with a `retry_after` duration when the
    /// provider's quota has been reached for the current window.
    #[must_use]
    pub fn check_rate_limit(&self, provider: &str) -> RateLimitStatus {
        let Some(limit_entry) = self.limits.get(provider) else {
            // No rate limit configured for this provider
            return RateLimitStatus::Allowed;
        };
        let (max_calls, window_duration) = *limit_entry;

        let now = Instant::now();

        // Check and possibly reset the window
        let mut entry = self.windows.entry(provider.to_owned()).or_insert((0, now));
        let (count, window_start) = entry.value_mut();

        // If the window has elapsed, reset it
        let elapsed = now.duration_since(*window_start);
        if elapsed >= window_duration {
            *count = 0;
            *window_start = now;
            return RateLimitStatus::Allowed;
        }

        if *count >= max_calls {
            let retry_after = window_duration.saturating_sub(elapsed);
            warn!(
                provider = provider,
                count = *count,
                max_calls = max_calls,
                retry_after_secs = retry_after.as_secs(),
                "Provider rate limit exceeded"
            );
            RateLimitStatus::Exceeded { retry_after }
        } else {
            RateLimitStatus::Allowed
        }
    }

    /// Record a successful API call to the given provider.
    ///
    /// Increments the call counter for the current window.
    pub fn record_call(&self, provider: &str) {
        let now = Instant::now();
        let mut entry = self.windows.entry(provider.to_owned()).or_insert((0, now));
        let (count, window_start) = entry.value_mut();

        // Reset window if expired
        if let Some(limit_entry) = self.limits.get(provider) {
            let (_, window_duration) = *limit_entry;
            if now.duration_since(*window_start) >= window_duration {
                *count = 0;
                *window_start = now;
            }
        }

        *count += 1;
    }

    /// Register a custom rate limit for a provider not in the default set.
    pub fn register_provider(&self, provider: &str, max_calls: u32, window: Duration) {
        self.limits.insert(provider.to_owned(), (max_calls, window));
        info!(
            provider = provider,
            max_calls = max_calls,
            window_secs = window.as_secs(),
            "Custom provider rate limit registered"
        );
    }
}

impl Default for ProviderRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}
