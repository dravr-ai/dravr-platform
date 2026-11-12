// ABOUTME: The wellness a history walk of a usr/pwd session beside a push-fed grant copies once its activities are in
// ABOUTME: The year of nights and readings before the grant, read in the walk's scope so no day the push holds is written

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The wellness half of a walk (carnet#1386).
//!
//! A Garmin athlete whose grant is pushed every summary from the day they
//! connected still has the nights and readings before it on the account.
//! Their usr/pwd session serves their reads, but while the push grant behind
//! it is live the push is their wellness source, so no pull reads the
//! session (carnet#1494). A walk of that session copies the
//! [`WELLNESS_HISTORY_DAYS`] before the grant once its activities are in,
//! inside [`history_walk_scope`], the only sync that reads a session beside
//! a live push-fed grant.

use super::super::ActivityBackfillJob;
#[cfg(feature = "health-sync")]
use chrono::{Days, NaiveDate};
#[cfg(feature = "health-sync")]
use dravr_enforme::error::EnformeResult;
#[cfg(feature = "health-sync")]
use pierre_core::models::ConnectionType;
#[cfg(feature = "health-sync")]
use pierre_providers::backend_resolver::{fallback_grant, pushed_backend_for};
#[cfg(feature = "health-sync")]
use pierre_services::garmin_push;
#[cfg(feature = "health-sync")]
use pierre_services::health_sync::history_walk_scope;
#[cfg(feature = "health-sync")]
use tracing::{info, warn};

/// Copy the wellness before the grant when `job` walks a session whose read
/// plan stands a live push-fed grant behind it ([`fallback_grant`]). Returns whether nothing of it is left to copy: `true`
/// once copied, and for every walk that has none to copy; `false` when a
/// read or a write failed, so a retry of the walk's last step copies again.
#[cfg(feature = "health-sync")]
pub(super) async fn copy_before_grant(job: &ActivityBackfillJob) -> bool {
    use pierre_providers::backend_resolver::user_facing_name;

    let behind = fallback_grant(
        job.resources.repos(),
        job.user_id,
        job.tenant_id,
        &job.provider_name,
    )
    .await;
    if behind.is_none_or(|grant| pushed_backend_for(&grant) != Some(grant.as_str())) {
        return true;
    }
    let Some(orchestrator) = job.resources.sync_orchestrator() else {
        return true;
    };
    let provider = user_facing_name(&job.provider_name);
    if !orchestrator.provider_names().contains(&provider) {
        return true;
    }
    // Without the grant's first day no day can be told the push's: copy none.
    let Some(pushed_from) = pushed_from(job, provider).await else {
        return false;
    };
    // A grant made on the oldest representable day has no day before it.
    let Some((start, end)) = wellness_window(pushed_from) else {
        return true;
    };
    let user_id = job.user_id.to_string();
    let read = orchestrator.backfill_between(&user_id, provider, start, end);
    let copy = wellness_copy_of(
        history_walk_scope(job.user_id, pushed_from, read)
            .await
            .map(|landed| {
                (
                    landed
                        .records_created
                        .saturating_add(landed.records_updated),
                    landed.records_errored,
                )
            }),
    );
    match &copy {
        Ok((written, errored)) => info!(
            user_id = %job.user_id,
            provider,
            records = written,
            errored,
            "History walk: copied the session's wellness"
        ),
        Err(why) => warn!(
            user_id = %job.user_id,
            provider,
            error = %why,
            "History walk: the session's wellness could not be copied"
        ),
    }
    copy.is_ok()
}

/// No health sync in this build: no wellness to copy.
#[cfg(not(feature = "health-sync"))]
pub(super) async fn copy_before_grant(_job: &ActivityBackfillJob) -> bool {
    true
}

/// What the windowed backfill's `result` — the records it wrote and the
/// days it could not read — copied, or why the copy is not complete. Any
/// unread day fails the copy, whatever else landed: it is history the
/// session still holds and only a retry can bring in. A read that wrote
/// nothing cleanly found no reading on any day, or held no session the
/// provider reads.
#[cfg(feature = "health-sync")]
fn wellness_copy_of(result: EnformeResult<(u32, u32)>) -> Result<(u32, u32), String> {
    let (written, errored) = result.map_err(|e| e.to_string())?;
    if errored > 0 {
        return Err(format!(
            "{errored} day(s) could not be read ({written} records landed)"
        ));
    }
    Ok((written, errored))
}

/// How many days of wellness before the grant a walk copies: a year, as
/// `ChefFamille` decided (carnet#1386). It is the depth a season's baselines
/// (resting heart rate, HRV, sleep) are read against, and it stays inside
/// the windowed backfill's bound
/// ([`dravr_enforme::orchestrator::backfill::MAX_BACKFILL_WINDOW_DAYS`]).
/// Older days are left on the account.
#[cfg(feature = "health-sync")]
const WELLNESS_HISTORY_DAYS: u64 = 365;

/// The inclusive day window a walk reads wellness over: the
/// [`WELLNESS_HISTORY_DAYS`] ending the day before `pushed_from`, the grant's
/// first day, which the push holds on. `None` when no day precedes it.
#[cfg(feature = "health-sync")]
fn wellness_window(pushed_from: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    let end = pushed_from.checked_sub_days(Days::new(1))?;
    let start = end
        .checked_sub_days(Days::new(WELLNESS_HISTORY_DAYS - 1))
        .unwrap_or(NaiveDate::MIN);
    Some((start, end))
}

/// The first day the grant's pushes hold `provider`'s wellness: the earlier
/// of the day its connection was made and the first day a push actually
/// stored ([`garmin_push::first_pushed_wellness_day`]).
///
/// A reconnect restamps the connection's `connected_at`, so it alone could
/// move the first day later than what the push already stored, and the walk
/// would then overwrite those days; the stored rows keep it at or before
/// them. `None` when the connection or the stored rows cannot be read, or
/// the connection is not there, so no day can be told apart from the push's.
#[cfg(feature = "health-sync")]
async fn pushed_from(job: &ActivityBackfillJob, provider: &str) -> Option<NaiveDate> {
    let repos = job.resources.repos();
    let connected = match repos
        .provider_connections
        .get_for_user(job.user_id, Some(job.tenant_id))
        .await
    {
        Ok(connections) => connections
            .iter()
            .filter(|c| c.provider == provider && c.connection_type != ConnectionType::Delegated)
            .map(|c| c.connected_at.date_naive())
            .min(),
        Err(e) => {
            warn!(user_id = %job.user_id, error = %e, "History walk: the grant's connection could not be read; copying no wellness");
            return None;
        }
    };
    let Some(connected) = connected else {
        warn!(user_id = %job.user_id, "History walk: the grant has no connection row; copying no wellness");
        return None;
    };
    match garmin_push::first_pushed_wellness_day(repos, job.user_id, job.tenant_id).await {
        Ok(stored) => Some(stored.map_or(connected, |stored| stored.min(connected))),
        Err(e) => {
            warn!(user_id = %job.user_id, error = %e, "History walk: the days the push holds could not be read; copying no wellness");
            None
        }
    }
}

#[cfg(feature = "health-sync")]
#[cfg(test)]
mod tests {
    use dravr_enforme::error::EnformeError;
    use dravr_enforme::orchestrator::backfill::MAX_BACKFILL_WINDOW_DAYS;

    use super::*;

    #[test]
    fn the_wellness_window_is_the_year_ending_the_day_before_the_grant() {
        let grant = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
        let (start, end) = wellness_window(grant).unwrap();
        assert_eq!(
            end,
            NaiveDate::from_ymd_opt(2026, 11, 4).unwrap(),
            "the grant's own day is the push's"
        );
        assert_eq!(
            start,
            NaiveDate::from_ymd_opt(2025, 11, 5).unwrap(),
            "bug: the window starts a year before the grant"
        );
        let days = (end - start).num_days() + 1;
        assert_eq!(days, 365, "bug: the window is WELLNESS_HISTORY_DAYS long");
        assert!(
            days <= MAX_BACKFILL_WINDOW_DAYS,
            "the window fits the backfill's bound"
        );
    }

    #[test]
    fn no_day_precedes_a_grant_on_the_oldest_day() {
        assert_eq!(
            wellness_window(NaiveDate::MIN),
            None,
            "bug: no day precedes the oldest representable day"
        );
    }

    #[test]
    fn an_unread_day_fails_the_copy_even_when_records_landed() {
        assert_eq!(
            wellness_copy_of(Ok((120, 0))),
            Ok((120, 0)),
            "bug: a clean read copies what it wrote"
        );
        assert_eq!(
            wellness_copy_of(Ok((0, 0))),
            Ok((0, 0)),
            "bug: a clean read of no reading is a complete copy"
        );
        assert!(
            wellness_copy_of(Ok((120, 3))).is_err(),
            "bug: an unread day fails the copy though records landed"
        );
        assert!(
            wellness_copy_of(Ok((0, 365))).is_err(),
            "bug: a read of no day fails the copy"
        );
        assert!(
            wellness_copy_of(Err(EnformeError::config("inverted"))).is_err(),
            "bug: a failed read fails the copy"
        );
    }
}
