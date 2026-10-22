// ABOUTME: The athlete's saved thresholds and zones that training-load and intensity readings use
// ABOUTME: One reader for every tool and snapshot that scores sessions or reads heart rate against zones

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The one reader of the thresholds training stress is scored against.
//!
//! Training stress per activity depends on the athlete's thresholds: power
//! against a saved FTP, heart rate against a saved threshold heart rate. Those
//! numbers live in one place, the physiological profile `set_physiology`
//! writes, and every computation that turns activities into CTL, ATL and TSB
//! reads them from there — `analyze_training_load`, the recovery tools,
//! `calculate_fitness_score`, `generate_recommendations`, `predict_performance`
//! and the group snapshot through this module, `get_training_history` and the
//! athlete snapshot through the same [`AthleteInputs::from_profile`] — so two
//! surfaces asked about the same day report the same form.
//!
//! There is no override layer. The user configuration that
//! `update_user_configuration` writes carries no thresholds: it refuses them
//! and names `set_physiology`.

use pierre_providers::ai_scope;
use std::sync::Arc;

use dravr_cageux::zones::HrZones;
use pierre_core::errors::AppResult;
use pierre_core::models::zones::{athlete_hr_zones, HrZonesUnavailable};
use pierre_core::models::{TenantId, UserPhysiologicalProfile};
use pierre_fitness_compute::AthleteInputs;
use uuid::Uuid;

use crate::runtime::ToolRuntime;

/// The thresholds stored on the athlete's physiological profile in this
/// tenant, each absent where the profile is silent.
///
/// A request with no tenant, or one that is not a tenant id, has no profile
/// to read — profiles are tenant-scoped — and gets [`AthleteInputs::default`],
/// under which the engine scores stress from pace or duration and says so.
///
/// # Errors
/// Returns the repository's error when the profile cannot be read, rather than
/// scoring the athlete's sessions as though they had saved nothing.
pub async fn stored_athlete_inputs(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: Option<&str>,
    user_id: Uuid,
) -> AppResult<AthleteInputs> {
    let profile = stored_profile(runtime, tenant_id, user_id).await?;
    Ok(AthleteInputs::from_profile(profile.as_ref()))
}

/// The athlete's heart-rate zones, cut by cageux from the physiological
/// profile saved in this tenant ([`athlete_hr_zones`]).
///
/// The inner `Err` says why there are none — no profile (or no tenant to
/// read one under), or a resting or maximum heart rate missing — so the
/// reading that needed them is reported absent with that reason rather than
/// computed against a default physiology.
///
/// # Errors
/// Returns the repository's error when the profile cannot be read.
pub async fn stored_hr_zones(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: Option<&str>,
    user_id: Uuid,
) -> AppResult<Result<HrZones, HrZonesUnavailable>> {
    let profile = stored_profile(runtime, tenant_id, user_id).await?;
    Ok(athlete_hr_zones(profile.as_ref()))
}

/// The athlete's profile in this tenant as an AI-facing reader may see it.
///
/// # Errors
/// Returns the repository's error when the profile cannot be read.
pub async fn stored_profile(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: Option<&str>,
    user_id: Uuid,
) -> AppResult<Option<UserPhysiologicalProfile>> {
    let Some(tenant) = tenant_id.and_then(|t| TenantId::parse_str(t).ok()) else {
        return Ok(None);
    };
    Ok(runtime
        .repos()
        .user_physiological_profile
        .get_user_physiological_profile(tenant, user_id)
        .await?
        // A profile written from first-party-only data is withheld from an
        // external caller (carnet#769).
        .filter(|profile| ai_scope::admit_derived(profile.transport_policy)))
}

/// The thresholds of a group member, read from the first of `tenants` whose
/// profile of theirs exists.
///
/// A member's sessions are fetched under each tenant they hold a provider
/// connection in — the tenant their own one-to-one chat runs in, where
/// `set_physiology` saved their numbers — so the profile is looked for in the
/// same tenants, in the same order. No profile in any of them gives
/// [`AthleteInputs::default`].
///
/// # Errors
/// Returns the repository's error when a profile cannot be read.
pub async fn member_athlete_inputs(
    runtime: &Arc<dyn ToolRuntime>,
    tenants: &[TenantId],
    user_id: Uuid,
) -> AppResult<AthleteInputs> {
    for tenant in tenants {
        let profile = runtime
            .repos()
            .user_physiological_profile
            .get_user_physiological_profile(*tenant, user_id)
            .await?
            // A profile written from first-party-only data is withheld from an
            // external caller (carnet#769).
            .filter(|profile| ai_scope::admit_derived(profile.transport_policy));
        if profile.is_some() {
            return Ok(AthleteInputs::from_profile(profile.as_ref()));
        }
    }
    Ok(AthleteInputs::default())
}
