// ABOUTME: App-wide rate limiter for external fitness provider APIs, counted in the database so every instance shares it
// ABOUTME: Strava's 15-minute and daily windows are usage_counters buckets taken by an atomic increment-if-under

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! App-wide provider request budgets.
//!
//! A provider limits the requests an application makes, whichever tenant or
//! athlete they are for: Strava counts every call against one 15-minute
//! window, reset at :00, :15, :30 and :45, and one daily window, reset at
//! midnight UTC, per app. A request has to fit every budget its provider has.
//!
//! The backend runs as several instances (Cloud Run scales it from zero to
//! three), so a count held in one process lets each instance spend the whole
//! budget. The counts live in the database instead, in `usage_counters` — the
//! table every quota counts in — one bucket per window under the platform
//! scope ([`PLATFORM_SCOPE`], a provider's budget belongs to no tenant or
//! user). The buckets are fixed and aligned to the epoch, as Strava's own
//! windows are, and each bucket's period label starts with its start time,
//! so the usage-counter pruning ages them out with the rest.
//!
//! A request is taken with one conditional increment per window, which lands
//! only while the bucket holds fewer than its allowance: instances racing for
//! the last slot get it once between them, and no window ever passes its
//! budget. When a later window refuses, the slots already taken for the
//! request are given back.
//!
//! Work nobody is waiting on — the walk of an athlete's history — asks with
//! [`ProviderRateLimiter::acquire_background`], which stops at
//! [`BACKGROUND_SHARE_PERCENT`] of each window, so the rest stays for the
//! requests an athlete is waiting on.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use pierre_core::constants::rate_limits;
use pierre_core::errors::AppResult;
use pierre_database::repositories::UsageCounterRepository;
use tracing::{debug, info, warn};

/// One day as a `Duration`.
pub const ONE_DAY: Duration = Duration::from_hours(24);

/// Strava's short window.
pub const FIFTEEN_MINUTES: Duration = Duration::from_mins(15);

/// The share of every window, in percent, that work nobody is waiting on may
/// spend: the walk of an athlete's whole Strava history for their personal
/// bests.
///
/// A quarter. Against Strava's standard read limits of 100 requests per 15
/// minutes and 1,000 per day, the walk gets 25 requests a window and 250 a
/// day, one streams request per past run, so a 1,000-run history is seeded in
/// about four days, while the requests an athlete is waiting on keep three
/// quarters of every window.
pub const BACKGROUND_SHARE_PERCENT: u32 = 25;

/// The `tenant_id` and `user_id` a provider's budget buckets are counted
/// under: the budget is the application's, owed to no tenant or user.
pub const PLATFORM_SCOPE: &str = "platform";

/// The `counter_key` of `provider`'s budget buckets.
#[must_use]
pub fn budget_counter_key(provider: &str) -> String {
    format!("provider_requests:{provider}")
}

/// The period label of the `window` bucket that holds `now`.
///
/// The bucket's start, to the millisecond, then the window's length in
/// milliseconds, so two windows starting at the same instant keep separate
/// buckets and the label sorts by date for the pruning.
#[must_use]
pub fn budget_period(now: DateTime<Utc>, window: Duration) -> String {
    let (start_ms, _) = bucket_bounds(now, window);
    let start = DateTime::from_timestamp_millis(start_ms).unwrap_or(now);
    format!(
        "{}/{}",
        start.format("%Y-%m-%dT%H:%M:%S%.3fZ"),
        window_ms(window)
    )
}

/// A window's length in milliseconds, never below one.
fn window_ms(window: Duration) -> i64 {
    i64::try_from(window.as_millis()).unwrap_or(i64::MAX).max(1)
}

/// The start and the end, in unix milliseconds, of the epoch-aligned
/// `window` bucket that holds `now`.
fn bucket_bounds(now: DateTime<Utc>, window: Duration) -> (i64, i64) {
    let length = window_ms(window);
    let now_ms = now.timestamp_millis();
    let start = now_ms - now_ms.rem_euclid(length);
    (start, start.saturating_add(length))
}

/// One budget: at most `max_calls` requests in each `window`.
#[derive(Debug, Clone, Copy)]
struct Budget {
    max_calls: u32,
    window: Duration,
}

impl Budget {
    /// The requests a caller asking for `share_percent` of the window may
    /// have counted in it.
    fn allowance(self, share_percent: u32) -> i64 {
        i64::from(self.max_calls) * i64::from(share_percent) / 100
    }
}

/// Holds each provider's budgets and takes requests from them in the
/// database, so every instance counts into the same windows.
pub struct ProviderRateLimiter {
    /// Where the windows are counted: `usage_counters`, shared by every
    /// instance of the backend.
    counters: Arc<dyn UsageCounterRepository>,
    /// Map of `provider_name` -> every budget a request to it must fit.
    /// Configuration only, the same on every instance; read and cloned out
    /// before any database call.
    budgets: DashMap<String, Vec<Budget>>,
}

/// Result of a rate limit check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimitStatus {
    /// The request was taken from every window of the provider's budget.
    Allowed,
    /// A window is spent; retry after the given duration.
    Exceeded {
        /// Duration until the spent window resets.
        retry_after: Duration,
    },
}

impl ProviderRateLimiter {
    /// A rate limiter counting in `counters`, pre-loaded with known provider
    /// rate limits.
    #[must_use]
    pub fn new(counters: Arc<dyn UsageCounterRepository>) -> Self {
        let daily = |max_calls| {
            vec![Budget {
                max_calls,
                window: ONE_DAY,
            }]
        };
        let budgets = DashMap::new();
        // Strava: the standard read limits, per app.
        budgets.insert(
            "strava".to_owned(),
            vec![
                Budget {
                    max_calls: rate_limits::STRAVA_RATE_LIMIT_15MIN,
                    window: FIFTEEN_MINUTES,
                },
                Budget {
                    max_calls: rate_limits::STRAVA_RATE_LIMIT_DAILY,
                    window: ONE_DAY,
                },
            ],
        );
        budgets.insert(
            "garmin".to_owned(),
            daily(rate_limits::GARMIN_DEFAULT_DAILY_RATE_LIMIT),
        );
        budgets.insert(
            "whoop".to_owned(),
            daily(rate_limits::WHOOP_DEFAULT_DAILY_RATE_LIMIT),
        );
        budgets.insert(
            "terra".to_owned(),
            daily(rate_limits::TERRA_DEFAULT_DAILY_RATE_LIMIT),
        );

        info!(
            provider_count = budgets.len(),
            "Provider rate limiter initialized"
        );

        Self { counters, budgets }
    }

    /// Take one request an athlete is waiting on from `provider`'s budget:
    /// the whole of every window is open to it.
    ///
    /// # Errors
    /// Returns a database error when a window cannot be counted.
    pub async fn acquire(&self, provider: &str) -> AppResult<RateLimitStatus> {
        let status = self.acquire_share(provider, 100).await?;
        if let RateLimitStatus::Exceeded { retry_after } = &status {
            warn!(
                provider = provider,
                retry_after_secs = retry_after.as_secs(),
                "Provider rate limit exceeded"
            );
        }
        Ok(status)
    }

    /// Take one request nobody is waiting on from `provider`'s budget: taken
    /// only while every window holds fewer requests than
    /// [`BACKGROUND_SHARE_PERCENT`] of its budget.
    ///
    /// # Errors
    /// Returns a database error when a window cannot be counted.
    pub async fn acquire_background(&self, provider: &str) -> AppResult<RateLimitStatus> {
        let status = self
            .acquire_share(provider, BACKGROUND_SHARE_PERCENT)
            .await?;
        if let RateLimitStatus::Exceeded { retry_after } = &status {
            debug!(
                provider = provider,
                retry_after_secs = retry_after.as_secs(),
                "Background share of the provider's rate limit spent"
            );
        }
        Ok(status)
    }

    /// Take one request from every window of `provider` that holds fewer
    /// than `share_percent` of its budget; when one does not, give back the
    /// windows already taken and say how long until that one resets.
    async fn acquire_share(
        &self,
        provider: &str,
        share_percent: u32,
    ) -> AppResult<RateLimitStatus> {
        let budgets: Vec<Budget> = match self.budgets.get(provider) {
            Some(budgets) => budgets.clone(),
            // No rate limit configured for this provider
            None => return Ok(RateLimitStatus::Allowed),
        };
        let key = budget_counter_key(provider);
        let now = Utc::now();
        let mut taken: Vec<String> = Vec::with_capacity(budgets.len());
        for budget in budgets {
            let period = budget_period(now, budget.window);
            let landed = self
                .counters
                .increment_counter_below(
                    PLATFORM_SCOPE,
                    PLATFORM_SCOPE,
                    &key,
                    &period,
                    budget.allowance(share_percent),
                )
                .await?;
            if landed {
                taken.push(period);
                continue;
            }
            for period in &taken {
                self.counters
                    .increment_counter(PLATFORM_SCOPE, PLATFORM_SCOPE, &key, period, -1)
                    .await?;
            }
            let (_, end_ms) = bucket_bounds(now, budget.window);
            let retry_ms = u64::try_from(end_ms - now.timestamp_millis()).unwrap_or(0);
            return Ok(RateLimitStatus::Exceeded {
                retry_after: Duration::from_millis(retry_ms),
            });
        }
        Ok(RateLimitStatus::Allowed)
    }

    /// Replace `provider`'s budgets with `budgets`, each `(max_calls,
    /// window)`.
    pub fn set_budgets(&self, provider: &str, budgets: &[(u32, Duration)]) {
        self.budgets.insert(
            provider.to_owned(),
            budgets
                .iter()
                .map(|(max_calls, window)| Budget {
                    max_calls: *max_calls,
                    window: *window,
                })
                .collect(),
        );
        info!(
            provider = provider,
            budgets = ?budgets,
            "Provider rate limit budgets set"
        );
    }
}
