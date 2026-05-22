// ABOUTME: Fragment-aware activity deduplication for overlapping GPS recordings
// ABOUTME: Detects Garmin auto-splits, Strava re-uploads, and dual-device captures
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Fragment-aware activity deduplication.
//!
//! Fitness providers routinely surface multiple recordings of the same physical
//! workout: a Garmin watch auto-splits a long ride when GPS drops, a user
//! re-uploads the same `.fit` file twice, or a watch + bike computer both push
//! the same ride to Strava. Without deduplication, downstream consumers count
//! these as distinct training sessions — inflating TSS, weekly volume, activity
//! counts, and confusing LLM coaches that report literal row counts.
//!
//! This module groups activities by **temporal overlap within the same sport
//! type**: two activities are considered fragments of the same workout when
//! their time windows intersect within a configurable tolerance (default 5
//! minutes). The strategy is **non-destructive**: all rows are preserved, and
//! a [`FragmentReport`] sidecar describes which rows belong together and which
//! row is canonical (longest duration, tie-broken by largest distance, then
//! lowest id for determinism).
//!
//! ```
//! use pierre_providers::deduplication::{detect_fragments, DedupConfig};
//! use pierre_core::models::Activity;
//!
//! # fn example(activities: &[Activity]) {
//! let report = detect_fragments(activities, &DedupConfig::default());
//! if report.has_fragments() {
//!     println!(
//!         "{} recordings → {} distinct sessions",
//!         report.raw_count, report.session_count
//!     );
//! }
//! # }
//! ```

mod config;
mod detection;

pub use config::DedupConfig;
pub use detection::{detect_fragments, FragmentGroup, FragmentReport};

use pierre_core::models::Activity;

/// Convenience helper for intelligence handlers: detect fragments, then
/// collapse the slice to canonical rows. Returns the canonical-only `Vec`
/// alongside the full report so the caller can log dedup metrics or surface
/// the grouping decision in its own response envelope.
///
/// Equivalent to:
/// ```text
/// let report = detect_fragments(activities, config);
/// let canonical = report.canonical_activities(activities);
/// (canonical, report)
/// ```
#[must_use]
pub fn dedupe_and_report(
    activities: &[Activity],
    config: &DedupConfig,
) -> (Vec<Activity>, FragmentReport) {
    let report = detect_fragments(activities, config);
    let canonical = report.canonical_activities(activities);
    (canonical, report)
}
