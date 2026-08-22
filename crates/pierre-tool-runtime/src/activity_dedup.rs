// ABOUTME: Cross-provider activity deduplication — same session recorded twice collapses to one row
// ABOUTME: Ungated: every consumer of multi-provider activity lists needs it, groups or not
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Cross-provider activity deduplication.
//!
//! An athlete with two sources (Strava + WHOOP, Garmin + Strava) records one
//! physical session as two rows. Same-sport rows dedup on time+distance
//! proximity; a wrist tracker that misclassified the sport dedups on
//! substantial wall-clock overlap, and [`TimeWindowDeduplicator`] keeps the
//! richer row (`pick_best` prefers the one carrying distance). Lived inside
//! the `tools-groups`-gated snapshot module until 2026-08-22, when the
//! single-user `get_activities` merge needed it too — dedup is provider-data
//! logic, not group logic.

use std::env;

use chrono::{DateTime, Duration, Utc};
use pierre_core::models::Activity;
use tracing::debug;

/// Default time window (minutes) for considering two activities as duplicates
const DEFAULT_DEDUP_TIME_WINDOW_MINUTES: i64 = 15;

/// Default distance tolerance (percentage) for duplicate detection
const DEFAULT_DEDUP_DISTANCE_TOLERANCE_PCT: f64 = 10.0;

/// Minimum fraction of the shorter activity's duration that must overlap
/// with the longer one for the cross-sport dedup path to collapse them.
///
/// Wrist-based trackers (WHOOP, Apple Watch) frequently misclassify
/// activities — a long bike ride shows up as a `Run` because the wrist
/// moves enough to trigger the wrong heuristic. The GPS provider
/// (Strava) reports the same physical session as `Ride`. Same-sport
/// dedup rejects this pair because of the sport mismatch, so both rows
/// survive and double-count toward weekly volume and the `Recent:`
/// block. The cross-sport check below relaxes the sport requirement
/// when the two activities' time windows overlap substantially (≥60%
/// of the shorter session), interpreting that overlap as strong
/// evidence of the same physical workout regardless of the declared
/// sport label.
const CROSS_SPORT_OVERLAP_FRACTION: f64 = 0.6;

/// Returns `true` when a timestamp's time-of-day component is exactly
/// `00:00:00` UTC, indicating the provider only resolved the workout day
/// (e.g. the Strava-mirror scraper). Used by the deduplicator to widen
/// the comparison window from minute-precision to calendar-day precision
/// for such pairs so they collapse against their precise-timestamped
/// counterparts on the same day.
fn is_date_only_timestamp(ts: DateTime<Utc>) -> bool {
    ts.time() == chrono::NaiveTime::MIN
}

/// Compute the wall-clock overlap in seconds between two activities, using
/// each one's start time and reported duration. Returns `None` when either
/// duration is unrepresentable as `i64` (effectively never under realistic
/// activity lengths) or when the intervals don't overlap.
fn time_overlap_seconds(a: &Activity, b: &Activity) -> Option<u64> {
    let a_dur_secs = i64::try_from(a.duration_seconds()).ok()?;
    let b_dur_secs = i64::try_from(b.duration_seconds()).ok()?;
    let a_end = a.start_date() + Duration::seconds(a_dur_secs);
    let b_end = b.start_date() + Duration::seconds(b_dur_secs);
    let overlap_start = a.start_date().max(b.start_date());
    let overlap_end = a_end.min(b_end);
    let overlap = (overlap_end - overlap_start).num_seconds();
    if overlap > 0 {
        u64::try_from(overlap).ok()
    } else {
        None
    }
}

// ══════════════════════════════════════════════════════════════
// Activity Deduplication
// ══════════════════════════════════════════════════════════════

/// Pluggable strategy for deduplicating activities from multiple providers.
///
/// When a user connects multiple providers (e.g., Strava + Garmin), the same
/// workout may appear from both sources. Implementations decide how to detect
/// and resolve these overlaps.
pub trait ActivityDeduplicator: Send + Sync {
    /// Remove duplicate activities, keeping the best version of each.
    fn deduplicate(&self, activities: Vec<Activity>) -> Vec<Activity>;
}

/// Deduplicates activities using time proximity, sport type, and distance similarity.
///
/// Configuration loaded from environment:
/// - `ACTIVITY_DEDUP_TIME_WINDOW_MINUTES` — max minutes between start times (default: 15)
/// - `ACTIVITY_DEDUP_DISTANCE_TOLERANCE_PCT` — max distance difference percentage (default: 10)
pub struct TimeWindowDeduplicator {
    time_window_minutes: i64,
    distance_tolerance_pct: f64,
}

impl TimeWindowDeduplicator {
    /// Create from environment variables with defaults.
    #[must_use]
    pub fn from_env() -> Self {
        let time_window_minutes = env::var("ACTIVITY_DEDUP_TIME_WINDOW_MINUTES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_DEDUP_TIME_WINDOW_MINUTES);

        let distance_tolerance_pct = env::var("ACTIVITY_DEDUP_DISTANCE_TOLERANCE_PCT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_DEDUP_DISTANCE_TOLERANCE_PCT);

        Self {
            time_window_minutes,
            distance_tolerance_pct,
        }
    }

    /// Check if two activities from different providers are likely the same workout.
    ///
    /// Two-mode time check:
    /// - Strict `time_window_minutes` when both timestamps look precise
    ///   (anything more than ~1 minute past midnight UTC).
    /// - Calendar-date match when either side is anchored at midnight UTC.
    ///   Scrape-based mirrors (e.g. the Strava-mirror sciotte provider)
    ///   only resolve to the workout day, not its actual start time, so
    ///   minute-window comparisons against a real Strava timestamp 12h
    ///   later let cross-provider duplicates slip through and double-count.
    fn is_likely_duplicate(&self, a: &Activity, b: &Activity) -> bool {
        // Same provider can't produce cross-provider duplicates
        if a.provider() == b.provider() {
            return false;
        }

        if a.sport_type() == b.sport_type() {
            self.is_same_sport_duplicate(a, b)
        } else {
            Self::is_cross_sport_duplicate(a, b)
        }
    }

    /// Same-sport cross-provider dedup: time proximity + distance similarity.
    /// Distance check is skipped when either source lacks GPS data, in which
    /// case the date+sport match alone is sufficient.
    fn is_same_sport_duplicate(&self, a: &Activity, b: &Activity) -> bool {
        // Time proximity: prefer minute-window when both look precise, fall
        // back to calendar-date match when at least one side is date-only.
        let a_is_date_only = is_date_only_timestamp(a.start_date());
        let b_is_date_only = is_date_only_timestamp(b.start_date());
        if a_is_date_only || b_is_date_only {
            if a.start_date().date_naive() != b.start_date().date_naive() {
                return false;
            }
        } else {
            let time_diff = (a.start_date() - b.start_date()).num_minutes().abs();
            if time_diff > self.time_window_minutes {
                return false;
            }
        }

        // Distance proximity (when both have distance data)
        match (a.distance_meters(), b.distance_meters()) {
            (Some(da), Some(db)) => {
                let max = da.max(db);
                max > 0.0 && (da - db).abs() / max * 100.0 < self.distance_tolerance_pct
            }
            // Both missing distance — trust the date+sport match
            _ => true,
        }
    }

    /// Cross-sport cross-provider dedup: time-overlap fraction only.
    ///
    /// Distance and sport-label are intentionally ignored — the wrist-tracker
    /// (WHOOP, Apple Watch) doesn't have GPS for the GPS provider's sport
    /// (cycling on a watch reports as `Run`; the same workout shows up as
    /// `Ride` on Strava with full distance). When both sessions overlap in
    /// wall-clock time by at least `CROSS_SPORT_OVERLAP_FRACTION` of the
    /// shorter one's duration, they are the same physical workout from
    /// different sources. `pick_best` then keeps the GPS row.
    ///
    /// Conservative: returns false when either side is anchored at midnight
    /// UTC (date-only timestamps from scrape-mirror providers like sciotte),
    /// since the overlap math is unreliable without a real start time.
    /// Returns false when either side has zero duration.
    fn is_cross_sport_duplicate(a: &Activity, b: &Activity) -> bool {
        if is_date_only_timestamp(a.start_date()) || is_date_only_timestamp(b.start_date()) {
            return false;
        }
        let a_dur = a.duration_seconds();
        let b_dur = b.duration_seconds();
        if a_dur == 0 || b_dur == 0 {
            return false;
        }
        let Some(overlap_secs) = time_overlap_seconds(a, b) else {
            return false;
        };
        let shorter_secs = a_dur.min(b_dur);
        if shorter_secs == 0 {
            return false;
        }
        #[allow(clippy::cast_precision_loss)]
        // bounded by single-activity duration; far below f64 limits
        let overlap_fraction = overlap_secs as f64 / shorter_secs as f64;
        overlap_fraction >= CROSS_SPORT_OVERLAP_FRACTION
    }

    /// Pick the better version of two duplicate activities.
    /// Prefers the one with more data (distance present, longer duration).
    fn pick_best(a: &Activity, b: &Activity) -> usize {
        let a_has_distance = a.distance_meters().is_some_and(|d| d > 0.0);
        let b_has_distance = b.distance_meters().is_some_and(|d| d > 0.0);

        if a_has_distance && !b_has_distance {
            return 0;
        }
        if b_has_distance && !a_has_distance {
            return 1;
        }

        // Both have or both lack distance — prefer longer duration
        usize::from(a.duration_seconds() < b.duration_seconds())
    }
}

impl ActivityDeduplicator for TimeWindowDeduplicator {
    fn deduplicate(&self, mut activities: Vec<Activity>) -> Vec<Activity> {
        if activities.len() < 2 {
            return activities;
        }

        // Sort by start time for efficient comparison
        activities.sort_by_key(Activity::start_date);

        let mut keep = vec![true; activities.len()];
        for i in 0..activities.len() {
            if !keep[i] {
                continue;
            }
            for j in (i + 1)..activities.len() {
                if !keep[j] {
                    continue;
                }
                // Early exit: once activities[j] falls on a strictly later
                // calendar day than activities[i], no further pair on this
                // outer index can be a same-workout duplicate. Date-bucketed
                // rather than minute-bucketed so scrape-mirror providers
                // (sciotte: date-only timestamps) still pair against the
                // matching Strava workout 12+ hours later in the same day.
                if activities[j].start_date().date_naive() > activities[i].start_date().date_naive()
                {
                    break;
                }
                if self.is_likely_duplicate(&activities[i], &activities[j]) {
                    let loser = if Self::pick_best(&activities[i], &activities[j]) == 0 {
                        j
                    } else {
                        i
                    };
                    keep[loser] = false;
                }
            }
        }

        let total = activities.len();
        let result: Vec<Activity> = activities
            .into_iter()
            .zip(keep.iter())
            .filter(|(_, &k)| k)
            .map(|(a, _)| a)
            .collect();

        let removed = total - result.len();
        if removed > 0 {
            debug!(
                removed,
                total, "Deduplication removed cross-provider duplicates"
            );
        }

        result
    }
}
