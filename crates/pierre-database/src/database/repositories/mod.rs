// ABOUTME: Re-exports repository traits from pierre-database crate
// ABOUTME: Provides direct implementations bridging domain-manager traits to SQLite Database
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

pub use crate::repositories::*;

/// Agent version history — snapshot, list, revert
mod agent_translations;
mod agents_assignments;
/// Direct `AgentsRepository` impl on `Database` (SQLite agents catalogue)
mod agents_impl;
mod agents_versions;
/// Direct `MobilityRepository` impl on `Database` (SQLite stretching + yoga)
mod mobility_impl;
/// Direct `RecipeRepository` impl on `Database` (SQLite recipe persistence)
mod recipes_impl;
/// Direct `StoreListingsRepository` impl on `Database` (SQLite marketplace listings)
mod store_listings_impl;
