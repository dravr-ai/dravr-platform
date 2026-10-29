// ABOUTME: Merges every recording of one physical workout into a single session
// ABOUTME: Auto-splits, re-uploads, dual devices and two providers' copies; fields combined
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Session merging — the one place the platform decides that two activity
//! rows are the same workout.
//!
//! Fitness sources routinely surface several recordings of one physical
//! workout: a Garmin watch auto-splits a long ride when GPS drops, a user
//! re-uploads the same `.fit` file, a watch and a bike computer both push the
//! ride, or an athlete with Strava and WHOOP connected gets the session from
//! each. Counted separately they inflate TSS, weekly volume and activity
//! counts, and confuse agents that report literal row counts.
//!
//! [`merge_duplicates`] groups those recordings and keeps one canonical row per
//! workout. The row takes the volume of every piece of the session it did not
//! record — a ride the watch split into 70 km and 30 km is one 100 km session —
//! while a second recording of the same time adds none, so nothing is counted
//! twice. It then fills its missing fields from the other recordings of the
//! whole session — so a Strava ride keeps its GPS and power while gaining the
//! WHOOP heart rate and the Intervals.icu RPE recorded for the same session. A
//! [`FragmentReport`] names every group and every field that crossed over.
//!
//! ```
//! use pierre_providers::deduplication::{merge_duplicates, DedupConfig};
//! use pierre_core::models::Activity;
//!
//! # fn example(activities: Vec<Activity>) {
//! let (sessions, report) = merge_duplicates(activities, &DedupConfig::default());
//! if report.has_fragments() {
//!     println!(
//!         "{} recordings → {} distinct sessions",
//!         report.raw_count,
//!         sessions.len()
//!     );
//! }
//! # }
//! ```

mod config;
mod detection;

pub use config::DedupConfig;
pub use detection::{
    carries_gps_trace, is_canonical_copy, merge_duplicates, FilledField, FragmentGroup,
    FragmentReport,
};
