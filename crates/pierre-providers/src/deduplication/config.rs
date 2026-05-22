// ABOUTME: Tunable thresholds for activity-fragment overlap detection
// ABOUTME: Default 5-min temporal tolerance balances Garmin auto-split + manual brick spacing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Configuration knobs for fragment detection.

use std::env;

/// Default temporal tolerance (in seconds) when matching overlapping activities.
///
/// Picked so that:
/// - Garmin auto-split fragments (typically 1-3 sec gap) are caught.
/// - True back-to-back sessions of the same sport (e.g., two trail runs ~10
///   minutes apart) are NOT grouped together.
/// - Dual-device recordings (watch + bike computer) starting 30-60 sec apart
///   are caught.
const DEFAULT_OVERLAP_TOLERANCE_SECS: u64 = 300;

/// Environment variable to override [`DEFAULT_OVERLAP_TOLERANCE_SECS`].
const ENV_OVERLAP_TOLERANCE_SECS: &str = "FRAGMENT_OVERLAP_TOLERANCE_SECS";

/// Configuration for fragment-group detection.
#[derive(Debug, Clone, Copy)]
pub struct DedupConfig {
    /// Maximum gap (in seconds) between the end of one activity and the start
    /// of another for them to be considered the same workout. Also applies in
    /// reverse: B counts as overlapping A if B.start lies between
    /// `A.start - tolerance` and `A.end + tolerance`.
    pub overlap_tolerance_secs: u64,
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self::from_env()
    }
}

impl DedupConfig {
    /// Build a config from the `FRAGMENT_OVERLAP_TOLERANCE_SECS` environment
    /// variable, falling back to [`DEFAULT_OVERLAP_TOLERANCE_SECS`] when it is
    /// absent or unparseable.
    #[must_use]
    pub fn from_env() -> Self {
        let overlap_tolerance_secs = env::var(ENV_OVERLAP_TOLERANCE_SECS)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_OVERLAP_TOLERANCE_SECS);
        Self {
            overlap_tolerance_secs,
        }
    }

    /// Construct a config with an explicit tolerance — useful in tests.
    #[must_use]
    pub const fn with_tolerance(overlap_tolerance_secs: u64) -> Self {
        Self {
            overlap_tolerance_secs,
        }
    }
}
