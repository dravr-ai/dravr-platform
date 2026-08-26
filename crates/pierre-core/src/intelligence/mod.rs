// ABOUTME: Intelligence module re-exports from dravr-cageux
// ABOUTME: Fitness profiles and max-heart-rate algorithms from the standalone engine
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Maximum heart rate estimation algorithms (from dravr-cageux)
pub mod algorithms {
    /// Maximum heart rate algorithms
    pub mod maxhr {
        pub use dravr_cageux::models::maxhr::*;
    }
}

// Fitness profile types from dravr-cageux
pub use dravr_cageux::models::fitness_profile::{
    FitnessLevel, TimeAvailability, UserFitnessProfile, UserPreferences,
};
