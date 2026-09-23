// ABOUTME: Tunable thresholds for recognizing two recordings of one workout
// ABOUTME: Fragment tolerance plus the cross-provider start window and distance tolerance
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Configuration knobs for session merging.

use std::env;
use std::str::FromStr;

/// Default temporal tolerance (in seconds) when matching overlapping activities.
///
/// Picked so that:
/// - Garmin auto-split fragments (typically 1-3 sec gap) are caught.
/// - True back-to-back sessions of the same sport (e.g., two trail runs ~10
///   minutes apart) are NOT grouped together.
/// - Dual-device recordings (watch + bike computer) starting 30-60 sec apart
///   are caught.
const DEFAULT_OVERLAP_TOLERANCE_SECS: u64 = 300;

/// Default window (minutes) between two providers' start times for the same
/// sport to count as one workout. Wider than the overlap tolerance because a
/// second provider's clock, or its rounding of the start, can sit minutes off.
const DEFAULT_TIME_WINDOW_MINUTES: i64 = 15;

/// Default distance tolerance (percent of the longer distance) for two
/// providers' records of the same sport to count as one workout.
const DEFAULT_DISTANCE_TOLERANCE_PCT: f64 = 10.0;

/// Environment variable to override [`DEFAULT_OVERLAP_TOLERANCE_SECS`].
const ENV_OVERLAP_TOLERANCE_SECS: &str = "FRAGMENT_OVERLAP_TOLERANCE_SECS";

/// Environment variable to override [`DEFAULT_TIME_WINDOW_MINUTES`].
const ENV_TIME_WINDOW_MINUTES: &str = "ACTIVITY_DEDUP_TIME_WINDOW_MINUTES";

/// Environment variable to override [`DEFAULT_DISTANCE_TOLERANCE_PCT`].
const ENV_DISTANCE_TOLERANCE_PCT: &str = "ACTIVITY_DEDUP_DISTANCE_TOLERANCE_PCT";

/// Configuration for session merging.
#[derive(Debug, Clone, Copy)]
pub struct DedupConfig {
    /// Maximum gap (in seconds) between the end of one activity and the start
    /// of another of the same sport for them to be pieces of one workout.
    /// Also applies in reverse: B counts as overlapping A if B.start lies
    /// between `A.start - tolerance` and `A.end + tolerance`.
    pub overlap_tolerance_secs: u64,
    /// Maximum minutes between two providers' start times for records of the
    /// same sport to be one workout.
    pub time_window_minutes: i64,
    /// Maximum distance difference, as a percentage of the longer distance,
    /// for two providers' records of the same sport to be one workout.
    pub distance_tolerance_pct: f64,
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

impl DedupConfig {
    /// Build a config from `FRAGMENT_OVERLAP_TOLERANCE_SECS`,
    /// `ACTIVITY_DEDUP_TIME_WINDOW_MINUTES` and
    /// `ACTIVITY_DEDUP_DISTANCE_TOLERANCE_PCT`, each falling back to its
    /// default when absent or unparseable.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            overlap_tolerance_secs: env_or(
                ENV_OVERLAP_TOLERANCE_SECS,
                DEFAULT_OVERLAP_TOLERANCE_SECS,
            ),
            time_window_minutes: env_or(ENV_TIME_WINDOW_MINUTES, DEFAULT_TIME_WINDOW_MINUTES),
            distance_tolerance_pct: env_or(
                ENV_DISTANCE_TOLERANCE_PCT,
                DEFAULT_DISTANCE_TOLERANCE_PCT,
            ),
        }
    }

    /// Construct a config with an explicit fragment tolerance and the default
    /// cross-provider thresholds — useful in tests.
    #[must_use]
    pub const fn with_tolerance(overlap_tolerance_secs: u64) -> Self {
        Self {
            overlap_tolerance_secs,
            time_window_minutes: DEFAULT_TIME_WINDOW_MINUTES,
            distance_tolerance_pct: DEFAULT_DISTANCE_TOLERANCE_PCT,
        }
    }
}

fn env_or<T: FromStr>(name: &str, default: T) -> T {
    env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
