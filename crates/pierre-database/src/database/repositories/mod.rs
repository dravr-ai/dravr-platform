// ABOUTME: Re-exports repository traits from pierre-database crate
// ABOUTME: Provides direct implementations bridging domain-manager traits to SQLite Database
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

pub use crate::repositories::*;

/// Coach version history — snapshot, list, revert
mod coaches_assignments;
/// Direct `CoachesRepository` impl on `Database` (SQLite coaches catalogue)
mod coaches_impl;
mod coaches_versions;
/// Direct `MobilityRepository` impl on `Database` (SQLite stretching + yoga)
mod mobility_impl;
/// Direct `RecipeRepository` impl on `Database` (SQLite recipe persistence)
mod recipes_impl;
/// Direct `SocialRepository` impl on `Database` (SQLite social graph + insights)
mod social_impl;
/// Direct `StoreListingsRepository` impl on `Database` (SQLite marketplace listings)
mod store_listings_impl;
