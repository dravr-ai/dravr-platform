// ABOUTME: Garmin consent notices — a deregistration withdraws the connection, a permission change purges what stopped
// ABOUTME: The grants Garmin last reported are kept on the connection and gate every later push

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Consent notices.
//!
//! Garmin tells Dravr when an athlete leaves (`deregistrations`: they removed
//! Dravr in Garmin Connect, or Dravr deregistered them) and when they stop
//! or resume sharing a kind of data while staying connected
//! (`userPermissionsChange`, which lists every permission they grant now).
//! A deregistration withdraws the connection through the host, which runs
//! the platform's disconnect chokepoint. A permission change is recorded on
//! the connection; a kind the athlete stopped sharing is purged at once, with
//! its attestation row, and further pushes of it are refused until Garmin
//! reports it granted again.
//!
//! Every purge here reaches only what the Garmin API wrote: its activities
//! (the usr/pwd connection's are stored under another provider key) and the
//! wellness filed under its own data source. The `garmin` wellness the
//! usr/pwd connection's health sync collected stays.

use pierre_core::constants::oauth_providers::GARMIN;
use pierre_core::errors::AppResult;
use pierre_database::repositories::PurgeReason;
use pierre_database::RepositoryRegistry;
use serde_json::Value;
use tracing::{info, warn};

use super::wire::PermissionsRecord;
use super::{
    owner_of, records_as, BatchOutcome, GarminOwner, OwnerBatch, SummaryType, API_SOURCE_MARKER,
};

/// The permission that lets Dravr receive the athlete's activities.
pub(crate) const GRANT_ACTIVITY_EXPORT: &str = "ACTIVITY_EXPORT";
/// The permission that lets Dravr receive the athlete's health summaries.
pub(crate) const GRANT_HEALTH_EXPORT: &str = "HEALTH_EXPORT";

/// Whether `grants` (as last recorded; `None` when Garmin reported none since
/// the connection was made) include `grant`.
fn grants_include(grants: Option<&[String]>, grant: &str) -> bool {
    grants.is_none_or(|grants| grants.iter().any(|g| g.eq_ignore_ascii_case(grant)))
}

/// Whether `grants` (as last recorded, `None` read as everything granted)
/// let the API push the athlete's wellness. When they do not, nothing but
/// the usr/pwd connection serves that athlete's wellness, so its health pull
/// runs for them.
#[cfg(feature = "health-sync")]
pub(crate) fn shares_wellness(grants: Option<&[String]>) -> bool {
    grants_include(grants, GRANT_HEALTH_EXPORT)
}

/// Whether a permission notice reporting `now` withdraws `grant`, given the
/// grants recorded `before` it.
///
/// No recorded grants read as everything granted, as they do for gating
/// pushes: pushes of every kind were accepted until this notice, so a kind it
/// leaves out may have API data to purge. The purge reaches only what the API
/// wrote, so a first notice purging a kind the API never delivered deletes
/// nothing.
fn withdrawn(before: Option<&[String]>, now: &[String], grant: &str) -> bool {
    grants_include(before, grant) && !grants_include(Some(now), grant)
}

/// Whether the owner still shares the data a `kind` push carries.
pub(super) async fn shares(
    repos: &RepositoryRegistry,
    owner: GarminOwner,
    kind: SummaryType,
) -> AppResult<bool> {
    let Some(grant) = kind.required_grant() else {
        return Ok(true);
    };
    let grants = repos
        .provider_connections
        .provider_grants(owner.user_id, owner.tenant_id, GARMIN)
        .await?;
    Ok(grants_include(grants.as_deref(), grant))
}

/// Whether `owner` still holds the Garmin connection that captured
/// `garmin_user` and still shares `kind`, read after a batch of it was
/// written. When either is gone the withdrawal raced the write, so the kind
/// of data the batch wrote is purged again: a withdrawal that committed
/// before this read did not see those rows.
pub(super) async fn still_shares(
    repos: &RepositoryRegistry,
    owner: GarminOwner,
    garmin_user: &str,
    kind: SummaryType,
) -> AppResult<bool> {
    let Some(grant) = kind.required_grant() else {
        return Ok(true);
    };
    let reason = if owner_of(repos, garmin_user).await? == Some(owner) {
        if shares(repos, owner, kind).await? {
            return Ok(true);
        }
        PurgeReason::PermissionWithdrawn
    } else {
        PurgeReason::ProviderRevoked
    };
    let purge = purge_kind(repos, owner, grant, reason).await?;
    info!(
        user_id = %owner.user_id,
        summary_type = kind.key(),
        rows_removed = purge,
        "Garmin push landed while the athlete withdrew it; what it wrote is purged"
    );
    Ok(false)
}

/// Purge the owner's Garmin API data that `grant` covers: its activities,
/// or the wellness filed under its API data source. Returns the rows
/// removed.
async fn purge_kind(
    repos: &RepositoryRegistry,
    owner: GarminOwner,
    grant: &str,
    reason: PurgeReason,
) -> AppResult<u64> {
    let purge = if grant == GRANT_HEALTH_EXPORT {
        repos
            .provider_data
            .purge_user_provider_source_wellness(
                owner.user_id,
                &owner.tenant_id,
                GARMIN,
                API_SOURCE_MARKER,
                reason,
            )
            .await?
    } else {
        repos
            .provider_data
            .purge_user_provider_activities(owner.user_id, &owner.tenant_id, GARMIN, reason)
            .await?
    };
    Ok(purge.total())
}

/// `deregistrations`: the owner's Garmin connection is withdrawn, its data
/// purged with the attestation row the chokepoint writes.
pub(super) async fn deregister(batch: &OwnerBatch<'_>, records: usize) -> AppResult<BatchOutcome> {
    batch.host.deregister(batch.owner).await?;
    Ok(BatchOutcome {
        stored: records,
        ..BatchOutcome::default()
    })
}

/// `userPermissionsChange`: the latest record's grants are recorded, and
/// each kind of data the athlete stopped sharing is purged.
pub(super) async fn permissions_changed(
    batch: &OwnerBatch<'_>,
    records: Vec<Value>,
) -> AppResult<BatchOutcome> {
    let mut outcome = BatchOutcome::default();
    let changes: Vec<PermissionsRecord> = records_as(records, &mut outcome);
    let Some(latest) = changes.last() else {
        return Ok(outcome);
    };
    let repos = batch.repos;
    let owner = batch.owner;
    let before = repos
        .provider_connections
        .provider_grants(owner.user_id, owner.tenant_id, GARMIN)
        .await?;
    let now: Vec<String> = latest
        .permissions
        .iter()
        .map(|grant| grant.trim().to_ascii_uppercase())
        .collect();
    let recorded = repos
        .provider_connections
        .set_provider_grants(owner.user_id, owner.tenant_id, GARMIN, &now)
        .await?;
    if !recorded {
        warn!(
            user_id = %owner.user_id,
            "Garmin permission change for an athlete with no Garmin connection row; grants not recorded"
        );
    }
    for grant in [GRANT_HEALTH_EXPORT, GRANT_ACTIVITY_EXPORT] {
        if !withdrawn(before.as_deref(), &now, grant) {
            continue;
        }
        let rows_removed =
            purge_kind(repos, owner, grant, PurgeReason::PermissionWithdrawn).await?;
        info!(
            user_id = %owner.user_id,
            permission = grant,
            rows_removed,
            "Garmin permission withdrawn: the athlete's Garmin API data of that kind is purged"
        );
    }
    outcome.stored = changes.len();
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notice_withdraws_what_it_leaves_out_of_what_was_granted() {
        let only_activities = vec![GRANT_ACTIVITY_EXPORT.to_owned()];
        assert!(
            withdrawn(None, &only_activities, GRANT_HEALTH_EXPORT),
            "health pushes were accepted until the first notice left it out"
        );
        assert!(
            !withdrawn(None, &only_activities, GRANT_ACTIVITY_EXPORT),
            "a kind the notice still lists is not withdrawn"
        );
        assert!(
            !withdrawn(
                Some(&only_activities),
                &only_activities,
                GRANT_HEALTH_EXPORT
            ),
            "a kind already withdrawn is not withdrawn again"
        );
    }

    #[test]
    fn no_recorded_grants_read_as_everything_granted() {
        assert!(
            grants_include(None, GRANT_HEALTH_EXPORT),
            "no recorded grants read as everything granted"
        );
        let only_activities = vec![GRANT_ACTIVITY_EXPORT.to_owned()];
        assert!(
            grants_include(Some(&only_activities), GRANT_ACTIVITY_EXPORT),
            "a recorded grant is included"
        );
        assert!(
            !grants_include(Some(&only_activities), GRANT_HEALTH_EXPORT),
            "a grant left out of the record is not included"
        );
    }
}
