// ABOUTME: The anchors dravr-cageux resolves an athlete's workout-family targets on, read from their stored physiology
// ABOUTME: The profile row joined with the per-sport anchors beside it, under the profile's transport policy (carnet#1496)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The one reader of an athlete's [`AthleteAnchors`].
//!
//! A family's targets are domain coordinates that dravr-cageux places on each
//! sport's own anchors; with none it uses the I8 default ratios (MAP ≈ 1.30 ×
//! CP, `vVO2max` ≈ 1.16 × CS). The save path, `prescribe_workout` and their
//! envelope checks read the athlete's anchors here, so the same session is
//! measured against the same numbers wherever it is written.

use pierre_core::errors::AppResult;
use pierre_core::models::periodization::AthleteAnchors;
use pierre_core::models::{sport_anchors, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use uuid::Uuid;

/// The anchors of `user_id`'s workout families in `tenant`.
///
/// `None` when the athlete has no profile, or one this caller may not see (a
/// profile written from first-party-only data, read by an external caller,
/// carnet#769): the families then resolve on the default ratios, as they do
/// for an athlete who has stated nothing. The per-sport anchors are read
/// only beside a profile the caller may see.
///
/// # Errors
///
/// Returns the repository's error when the profile or its anchors cannot be
/// read, rather than measuring the athlete's sessions against defaults.
pub async fn stored_athlete_anchors(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
) -> AppResult<Option<AthleteAnchors>> {
    let profiles = &repos.user_physiological_profile;
    let Some(profile) = profiles
        .get_user_physiological_profile(tenant, user_id)
        .await?
        .filter(|profile| ai_scope::admit_derived(profile.transport_policy))
    else {
        return Ok(None);
    };
    let stored = profiles.list_sport_anchors(tenant, user_id).await?;
    Ok(Some(sport_anchors::athlete_anchors(&profile, &stored)))
}
