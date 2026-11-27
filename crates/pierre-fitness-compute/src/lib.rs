// ABOUTME: Platform-side fitness computations over dravr-cageux primitives + provider/context data
// ABOUTME: Endurance (snapshot/history/intervals/thresholds) and weather
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![deny(unsafe_code)]

//! # Pierre Fitness Compute
//!
//! Platform-side fitness computation built on the `dravr-cageux` sports-science
//! engine plus provider and context data.
//!
//! It owns the Endurance API computations (latest snapshot, training history,
//! intervals, LT1/LT2 thresholds) and weather context. Route and place
//! geometry lives in `dravr-arpenteur`; place parsing and daylight in
//! `dravr-meteo`.

/// Endurance Phase 1 latest-snapshot computation (`export_latest_snapshot`).
pub mod latest_snapshot;
pub use latest_snapshot::{
    build_latest_snapshot, ActivitySection11Metrics, LatestSnapshot, LatestSnapshotActivityRow,
    DEFAULT_WINDOW_DAYS, MAX_WINDOW_DAYS,
};

/// Endurance Phase 2 daily training-history compute (`get_training_history`).
pub mod training_history_compute;
pub use training_history_compute::{
    compute_training_history, warmup_days, AthleteInputs, ATL_WINDOW_DAYS as TH_ATL_WINDOW_DAYS,
    CTL_WARMUP_MARGIN_DAYS, CTL_WINDOW_DAYS as TH_CTL_WINDOW_DAYS,
    MAX_BACKFILL_DAYS as TH_MAX_BACKFILL_DAYS,
};

/// Endurance Phase 3 intervals.json builder (`export_intervals`).
pub mod intervals;
pub use intervals::{build_intervals, IntervalRow, IntervalsExport};

/// Weather impact analysis + provider factory (delegates vendor logic to dravr-meteo).
pub mod weather;
// Outer doc intentionally omitted, as for `weather_cache_adapter` below.
pub mod weather_clients;
// Outer doc intentionally omitted — `weather_cache_adapter.rs`'s
// inner `//!` header is authoritative. When both an outer `///` on the
// mod declaration and an inner `//!` exist, rustdoc concatenates them
// into one virtual doc block whose first paragraph trips
// `clippy::too_long_first_doc_paragraph`.
pub mod weather_cache_adapter;
