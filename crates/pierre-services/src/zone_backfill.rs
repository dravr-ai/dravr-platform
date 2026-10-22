// ABOUTME: Re-cuts a tenant's stored HR and power zone copies from each profile's own inputs
// ABOUTME: Operator-run, dry by default; turns rows cut by the old platform tables into cageux's one zone model
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! One-off re-cut of the zone copies stored on physiological profiles.
//!
//! `set_physiology` stores the athlete's zones beside the inputs they are cut
//! from (`hr_zones_json` from resting and maximum heart rate and `VO2max`,
//! `power_zones_json` from FTP). Rows written before the zones moved to
//! dravr-cageux hold boundaries the platform cut with its own tables; they
//! parse as [`pierre_core::models::zones::HrZoneModel::Legacy`] and would
//! keep reading that way until the athlete next saved a value.
//!
//! This walks one tenant's profiles and re-cuts each stored set with
//! [`UserPhysiologicalProfile::refresh_zones`], the same call `set_physiology`
//! makes, so a backfilled row is byte-for-byte what a fresh save would write.
//! No schema changes and no input is touched: a set whose inputs are gone
//! stays as stored, and the inputs being kept is what makes the rewrite
//! reversible by re-deriving.
//!
//! Re-running it is a no-op: a row already holding cageux's zones is counted
//! as current and not written again. It is **dry by default**, so an operator
//! sees the count before any row moves.

use pierre_core::errors::AppResult;
use pierre_core::models::zones::{HrZoneModel, PowerZoneModel};
use pierre_core::models::{TenantId, UserPhysiologicalProfile};
use pierre_database::repositories::UserPhysiologicalProfileRepository;
use serde::Serialize;
use tracing::info;

/// What the run did, or would have done.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ZoneBackfillStats {
    /// Profiles read.
    pub profiles_scanned: u64,
    /// Profiles whose stored zones differed from cageux's and were (or, on a
    /// dry run, would be) rewritten.
    pub rewritten: u64,
    /// Profiles whose stored zones already matched cageux's and hold no
    /// legacy set.
    pub already_current: u64,
    /// Profiles still holding a legacy set after the re-cut because its
    /// inputs are missing (no resting or maximum heart rate, or no FTP).
    pub legacy_without_inputs: u64,
    /// Whether nothing was written.
    pub dry_run: bool,
}

/// Re-cut the zones stored on every profile in `tenant_id`.
///
/// # Errors
///
/// Returns a database error from listing, reading or writing a profile.
pub async fn run_zone_backfill(
    profiles: &dyn UserPhysiologicalProfileRepository,
    tenant_id: TenantId,
    dry_run: bool,
) -> AppResult<ZoneBackfillStats> {
    let mut stats = ZoneBackfillStats {
        dry_run,
        ..ZoneBackfillStats::default()
    };
    for user_id in profiles.list_profile_user_ids(tenant_id).await? {
        let Some(stored) = profiles
            .get_user_physiological_profile(tenant_id, user_id)
            .await?
        else {
            continue;
        };
        stats.profiles_scanned += 1;
        let mut recut = stored.clone();
        recut.refresh_zones();
        let legacy_left = still_legacy(&recut);
        if legacy_left {
            stats.legacy_without_inputs += 1;
        }
        if recut.hr_zones == stored.hr_zones && recut.power_zones == stored.power_zones {
            // A legacy set the re-cut could not touch is unchanged but not
            // current: it is counted above, not here.
            if !legacy_left {
                stats.already_current += 1;
            }
            continue;
        }
        stats.rewritten += 1;
        if !dry_run {
            profiles
                .upsert_user_physiological_profile(tenant_id, user_id, &recut)
                .await?;
        }
    }
    info!(
        tenant_id = %tenant_id,
        profiles_scanned = stats.profiles_scanned,
        rewritten = stats.rewritten,
        already_current = stats.already_current,
        legacy_without_inputs = stats.legacy_without_inputs,
        dry_run,
        "zone_backfill_complete"
    );
    Ok(stats)
}

/// Whether a profile still holds a zone set the old platform tables cut.
fn still_legacy(profile: &UserPhysiologicalProfile) -> bool {
    profile
        .hr_zones
        .is_some_and(|zones| zones.model == HrZoneModel::Legacy)
        || profile
            .power_zones
            .is_some_and(|zones| zones.model == PowerZoneModel::Legacy)
}
