// ABOUTME: How a pooling provider's app windows are sized from the athletes who granted it
// ABOUTME: Intervals.icu's terms: a daily pool per grant between clamps, and a bucketed short window

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The pool an OAuth app's windows are sized from.

use std::time::Duration;

use pierre_core::constants::rate_limits;

use super::{Budget, BUCKETS_A_ROLLING_WINDOW_SPANS, FIFTEEN_MINUTES, ONE_DAY};

/// An app's windows at a provider that sizes them by the athletes who
/// granted the app: a daily pool of `per_grant` requests for each of them,
/// between `min_daily` and `max_daily`, and a rolling `short_window` of the
/// day divided by `short_divisor`, never below `min_short`.
///
/// The limiter counts fixed buckets aligned to the epoch, while the
/// provider's short window rolls. A rolling window as long as a bucket
/// overlaps two adjacent buckets, so each bucket holds half the rolling
/// limit ([`BUCKETS_A_ROLLING_WINDOW_SPANS`]): together the two can never
/// pass it, wherever the provider's window falls. A full-size bucket would
/// let up to twice the limit through across a boundary, a burst the provider
/// refuses with its own 429, and `usage_counters` keeps no per-request
/// timestamps from which a sliding window could be counted. The pool is
/// counted per UTC day, as every daily window here is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GrantPool {
    per_grant: u32,
    min_daily: u32,
    max_daily: u32,
    short_window: Duration,
    short_divisor: u32,
    min_short: u32,
}

impl GrantPool {
    /// Intervals.icu's terms for an OAuth app (forum.intervals.icu/t/609).
    pub(super) const fn intervals_icu() -> Self {
        Self {
            per_grant: rate_limits::INTERVALS_ICU_OAUTH_DAILY_PER_ATHLETE,
            min_daily: rate_limits::INTERVALS_ICU_OAUTH_DAILY_MIN,
            max_daily: rate_limits::INTERVALS_ICU_OAUTH_DAILY_MAX,
            short_window: FIFTEEN_MINUTES,
            short_divisor: rate_limits::INTERVALS_ICU_OAUTH_15MIN_DAILY_DIVISOR,
            min_short: rate_limits::INTERVALS_ICU_OAUTH_15MIN_MIN,
        }
    }

    /// The daily pool of an app `grants` athletes granted.
    pub(super) fn daily(self, grants: u32) -> u32 {
        self.per_grant
            .saturating_mul(grants)
            .max(self.min_daily)
            .min(self.max_daily)
    }

    /// The windows of an app whose day holds `daily` requests: the short
    /// bucket at its share of the rolling limit that day sets, then the day.
    pub(super) fn windows(self, daily: u32) -> Vec<Budget> {
        let rolling = (daily / self.short_divisor.max(1)).max(self.min_short);
        vec![
            Budget {
                max_calls: rolling / BUCKETS_A_ROLLING_WINDOW_SPANS,
                window: self.short_window,
            },
            Budget {
                max_calls: daily,
                window: ONE_DAY,
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Intervals.icu's daily pool is 100 requests for each athlete who
    /// granted the app, held to at least 5,000 and at most 50,000: the 50th
    /// athlete still leaves it at the minimum and the 51st widens it, the
    /// 500th fills it and the 501st adds nothing.
    #[test]
    fn the_intervals_pool_grows_by_a_hundred_an_athlete_between_its_clamps() {
        let pool = GrantPool::intervals_icu();
        assert_eq!(pool.daily(0), 5_000);
        assert_eq!(pool.daily(50), 5_000);
        assert_eq!(pool.daily(51), 5_100);
        assert_eq!(pool.daily(499), 49_900);
        assert_eq!(pool.daily(500), 50_000);
        assert_eq!(pool.daily(501), 50_000);
        assert_eq!(pool.daily(u32::MAX), 50_000, "the product saturates");
    }

    /// The rolling 15-minute limit is an eighth of the day, at least 2,500,
    /// and each fixed 15-minute bucket holds half of it, so two adjacent
    /// buckets never pass it: 1,250 a bucket until the day passes 20,000,
    /// 3,125 at the day's 50,000.
    #[test]
    fn each_fifteen_minute_bucket_holds_half_the_rolling_limit() {
        let pool = GrantPool::intervals_icu();
        let short = |daily| {
            let windows = pool.windows(daily);
            assert_eq!(windows[1].max_calls, daily, "the day is the pool");
            assert_eq!(windows[1].window, ONE_DAY);
            assert_eq!(windows[0].window, FIFTEEN_MINUTES);
            windows[0].max_calls
        };
        assert_eq!(short(5_000), 1_250);
        assert_eq!(short(20_000), 1_250);
        assert_eq!(short(24_000), 1_500);
        assert_eq!(short(50_000), 3_125);
    }
}
