// ABOUTME: Sleep tool operational parameters: how many recent activities a sleep tool fetches
// ABOUTME: The sleep science (trend window and thresholds, sleep target, wind-down) is dravr-cageux's
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::constants::sleep_recovery;
use serde::{Deserialize, Serialize};
use std::env;

/// Sleep tool operational parameters
///
/// Only how much data a sleep tool fetches. The trend's window and thresholds,
/// the sleep target and the wind-down are cageux's
/// (`SleepRecoveryConfig::sleep_trend` and `::sleep_schedule`, carnet#1803).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SleepToolParamsConfig {
    /// Number of recent activities to fetch for analysis
    pub activity_limit: u32,
}

impl Default for SleepToolParamsConfig {
    fn default() -> Self {
        Self {
            activity_limit: sleep_recovery::ACTIVITY_LIMIT,
        }
    }
}

impl SleepToolParamsConfig {
    /// Load sleep tool parameters from environment
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            activity_limit: env::var("SLEEP_RECOVERY_ACTIVITY_LIMIT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(sleep_recovery::ACTIVITY_LIMIT),
        }
    }
}
