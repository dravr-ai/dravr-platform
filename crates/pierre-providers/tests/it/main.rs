// ABOUTME: One integration-test binary for pierre-providers: every suite is a module here
// ABOUTME: Suites that mutate the process environment stay separate binaries in tests/
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Integration tests for the providers crate, linked as one binary.
//!
//! A top-level file in `tests/` is a test binary of its own, linking its own
//! copy of the crate. Only a suite that writes the process environment stays
//! one, because the environment is shared by every test in a binary.

mod ai_scope_planned_workouts_test;
mod api_request_401_precedes_vendor_error_test;
mod coach_roster_capability_test;
mod debug_redaction_test;
mod deduplication_test;
#[cfg(feature = "provider-intervals-icu")]
mod intervals_icu_delegated_read_test;
#[cfg(feature = "provider-intervals-icu")]
mod intervals_icu_provider_test;
mod provider_401_requires_reauth_test;
mod request_budget_test;
#[cfg(feature = "provider-sciotte")]
mod sciotte_coros_provider_test;
#[cfg(feature = "provider-sciotte")]
mod sciotte_planned_workouts_test;
#[cfg(feature = "provider-sciotte")]
mod sciotte_route_time_series_test;
#[cfg(feature = "provider-sciotte")]
mod sciotte_session_expiry_test;
#[cfg(feature = "provider-sciotte")]
mod sciotte_session_reimport_test;
#[cfg(feature = "provider-sciotte")]
mod sciotte_trainingpeaks_conversion_test;
#[cfg(feature = "provider-strava")]
mod strava_detail_read_test;
#[cfg(feature = "provider-strava")]
mod strava_summary_polyline_test;
#[cfg(feature = "provider-sciotte")]
mod trainingpeaks_plan_conversion_test;
#[cfg(feature = "provider-wahoo")]
mod wahoo_provider_test;
