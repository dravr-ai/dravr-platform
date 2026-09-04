// ABOUTME: Platform-side fitness computations over dravr-cageux primitives + provider/context data
// ABOUTME: Endurance (snapshot/history/intervals/routes/thresholds), geo, weather
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
//! intervals, route terrain, LT1/LT2 thresholds), geocoding and OSM route
//! discovery, and weather context.

/// Jack Daniels' oxygen-cost curve, inverted: `VO2max` to velocity at `VO2max`.
pub mod daniels;
pub use daniels::velocity_at_vo2max;

/// Endurance Phase 1 latest-snapshot computation (`GET /api/v1/endurance/latest`).
pub mod latest_snapshot;
pub use latest_snapshot::{
    build_latest_snapshot, ActivitySection11Metrics, LatestSnapshot, LatestSnapshotActivityRow,
    DEFAULT_WINDOW_DAYS, MAX_WINDOW_DAYS,
};

/// Endurance Phase 2 daily training-history compute (`GET /api/v1/endurance/history`).
pub mod training_history_compute;
pub use training_history_compute::{
    compute_training_history, AthleteInputs, ACWR_ACUTE_DAYS, ACWR_CHRONIC_DAYS,
    ATL_WINDOW_DAYS as TH_ATL_WINDOW_DAYS, CTL_WINDOW_DAYS as TH_CTL_WINDOW_DAYS,
    FOSTER_WINDOW_DAYS, MAX_BACKFILL_DAYS as TH_MAX_BACKFILL_DAYS, RAMP_RATE_LOOKBACK_DAYS,
};

/// Endurance Phase 3 intervals.json builder (`GET /api/v1/endurance/intervals/{activity_id}`).
pub mod intervals;
pub use intervals::{build_intervals, IntervalRow, IntervalsExport};

/// Endurance Phase 3 GPX terrain analysis (`GET /api/v1/endurance/routes/{activity_id}`).
pub mod routes;
pub use routes::{
    build_route_summary, build_route_summary_from_streams, haversine_meters_between, Climb,
    ClimbCategory, RouteSummary, TerrainSummary,
};

/// Endurance Phase 3 LT1 / LT2 threshold estimators.
pub mod threshold_estimation;
pub use threshold_estimation::{ThresholdEstimate, ThresholdInputs};

/// Location and geographic context (geocoding, elevation, address parsing via OSM Nominatim).
pub mod location;
/// Route and trail discovery via OpenStreetMap (Overpass API + OSM piste data).
pub mod osm_routes;
/// Weather impact analysis + provider factory (delegates vendor logic to dravr-meteo).
pub mod weather;
// Outer doc intentionally omitted — `weather_cache_adapter.rs`'s
// inner `//!` header is authoritative. When both an outer `///` on the
// mod declaration and an inner `//!` exist, rustdoc concatenates them
// into one virtual doc block whose first paragraph trips
// `clippy::too_long_first_doc_paragraph`.
pub mod weather_cache_adapter;

pub use osm_routes::{
    build_overpass_query, routes_from_overpass_json, DiscoveredRoute, RouteDiscoveryService,
    RouteSource, RouteType,
};
