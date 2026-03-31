// ABOUTME: Data source and device tracking types re-exported from dravr-equilibre
// ABOUTME: Provides device identity, type classification, and deduplication priority
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Data source tracking for device and provider identity management.
//!
//! These types enable tracking which device and provider recorded each data point,
//! supporting deduplication when multiple devices report the same metric.

pub use dravr_equilibre::{DataSource, DevicePriority, DeviceType, ProviderPriority};
// trigger CI
