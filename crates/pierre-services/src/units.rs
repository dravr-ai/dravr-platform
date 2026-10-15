// ABOUTME: One reader for "does this athlete read kilometres or miles" — the REST surface, notifications and the agent's prompt ask it
// ABOUTME: Metric unless the athlete chose imperial in Settings; an unreadable choice reads metric
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete units (carnet#835, carnet#1044).
//!
//! The athlete's Settings choice is the only input: no provider setting and
//! no device locale is consulted, so an athlete who never chose reads metric.

use pierre_core::models::UnitSystem;
use pierre_database::repositories::UnitPreferencesRepository;
use tracing::warn;
use uuid::Uuid;

/// The unit system `user_id` reads.
///
/// Never fails: an unreadable choice is logged and reads as metric, so the
/// athlete gets the default rather than an error on every page that prints a
/// distance.
pub async fn resolve_user_units(
    unit_preferences: &dyn UnitPreferencesRepository,
    user_id: Uuid,
) -> UnitSystem {
    match unit_preferences.get_units(user_id).await {
        Ok(units) => units,
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "Unit preference unreadable; reading metric");
            UnitSystem::default()
        }
    }
}
