// ABOUTME: One integration-test binary for pierre-fitness-compute: every suite is a module here
// ABOUTME: Suites that mutate the process environment stay separate binaries in tests/
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Integration tests for the fitness-compute crate, linked as one binary.
//!
//! A top-level file in `tests/` is a test binary of its own, linking its own
//! copy of the crate. Only a suite that writes the process environment stays
//! one, because the environment is shared by every test in a binary.

mod intervals_routes_test;
mod latest_snapshot_test;
mod polyline_test;
mod route_privacy_test;
mod route_track_test;
mod threshold_estimation_test;
mod training_history_compute_test;
