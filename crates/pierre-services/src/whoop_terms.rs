// ABOUTME: What health sync may keep of a WHOOP record: measurements, never WHOOP's own scores
// ABOUTME: Drops WHOOP scores at ingestion, computes sleep efficiency in-house, and keeps nothing without owner authorization

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! WHOOP's proprietary scores stop at the storage boundary.
//!
//! WHOOP's API Terms of Use (effective 2026-10-06, §4) forbid permanent
//! copies and derivative works of WHOOP Data unless the data's owner
//! authorizes them. WHOOP's own calculations — recovery %, day and workout
//! strain, sleep performance and WHOOP's sleep efficiency — are WHOOP's to
//! authorize, not the athlete's, so there is no consent that could let Dravr
//! keep them (tier 4 of the 2026-09-23 legal read). They are dropped here,
//! before anything is written; what nothing stores, no prompt and no tool
//! reply can quote.
//!
//! Every WHOOP record health sync writes passes through
//! [`crate::health_sync::PierreSyncStorage`], which applies these functions to
//! each record before its upsert. The measurements the record carries (sleep
//! and stage durations, awake time, resting heart rate, HRV, SpO2, skin
//! temperature, weight) pass through unchanged. Records from other providers
//! pass through untouched.
//!
//! WHOOP workouts reach the activity cache through the platform's own WHOOP
//! provider, which never maps a workout's strain to begin with.
//!
//! The measurements themselves are kept only under the owner's authorization:
//! the WHOOP notice ([`provider_terms_version`]) an account accepts before the
//! WHOOP OAuth flow begins. WHOOP's terms bind every account, so the notice is
//! asked of every account whatever its feature flags; for one that has not
//! accepted the current version, [`owner_authorization_outstanding`] answers
//! true and health sync keeps none of that account's WHOOP records — the same
//! rule, read through the same [`outstanding_notice`], as the connect that
//! asks for it.
//!
//! [`provider_terms_version`]: pierre_core::constants::oauth_providers::provider_terms_version

use std::borrow::Cow;

use chrono::{DateTime, Utc};
use pierre_core::constants::oauth_providers;
use pierre_core::errors::AppResult;
use pierre_core::models::{StoredRecoveryMetrics, StoredSleepSession};
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

use crate::provider_notice::outstanding_notice;

/// Whether a synced record came from WHOOP, by the source name the sync
/// adapter stamps on it.
#[must_use]
pub fn is_whoop(source_name: &str) -> bool {
    source_name.eq_ignore_ascii_case(oauth_providers::WHOOP)
}

/// Sleep efficiency as Dravr computes it, in percent: the share of the time
/// in bed not spent awake.
///
/// ```text
/// time_in_bed = end - start                     (whole seconds)
/// efficiency  = (time_in_bed - awake) / time_in_bed * 100
/// ```
///
/// Both inputs are measurements: the session's recorded bounds and the awake
/// time within them. `None` when the awake time is unknown, the session is
/// empty (or longer than `u32` seconds), or the awake time exceeds the
/// session, since none of those yields a figure worth storing.
#[must_use]
pub fn in_house_sleep_efficiency(
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    awake_seconds: Option<u32>,
) -> Option<f64> {
    let awake = awake_seconds?;
    let time_in_bed = u32::try_from((end - start).num_seconds())
        .ok()
        .filter(|seconds| *seconds > 0)?;
    let asleep = time_in_bed.checked_sub(awake)?;
    Some(f64::from(asleep) * 100.0 / f64::from(time_in_bed))
}

/// The sleep session health sync may store.
///
/// For WHOOP: without WHOOP's sleep performance score, and with WHOOP's sleep
/// efficiency replaced by [`in_house_sleep_efficiency`]. Any other source's
/// session is returned as it came.
#[must_use]
pub fn sleep_session_to_store(session: &StoredSleepSession) -> Cow<'_, StoredSleepSession> {
    if !is_whoop(&session.source_name) {
        return Cow::Borrowed(session);
    }
    let mut kept = session.clone(); // Safe: owned copy for the withheld fields
    kept.sleep_score = None;
    kept.sleep_efficiency = in_house_sleep_efficiency(
        session.start_datetime,
        session.end_datetime,
        session.awake_seconds,
    );
    Cow::Owned(kept)
}

/// The recovery metrics health sync may store.
///
/// For WHOOP: without any provider-scored composite — recovery %, readiness,
/// stress, energy and day strain. Any other source's metrics are returned as
/// they came.
#[must_use]
pub fn recovery_metrics_to_store(
    metrics: &StoredRecoveryMetrics,
) -> Cow<'_, StoredRecoveryMetrics> {
    if !is_whoop(&metrics.source_name) {
        return Cow::Borrowed(metrics);
    }
    let mut kept = metrics.clone(); // Safe: owned copy for the withheld fields
    kept.recovery_score = None;
    kept.readiness_score = None;
    kept.stress_score = None;
    kept.body_battery = None;
    kept.daily_strain = None;
    Cow::Owned(kept)
}

/// Whether `user_id` still owes WHOOP's owner authorization in this tenant.
///
/// True while the WHOOP notice is in force for the account and its current
/// version is not the one the account accepted; health sync then keeps none
/// of the account's WHOOP records.
///
/// # Errors
/// Returns the store's error when the acceptance cannot be read; the caller
/// keeps nothing it could not clear.
pub async fn owner_authorization_outstanding(
    repos: &RepositoryRegistry,
    tenant_id: Uuid,
    user_id: Uuid,
) -> AppResult<bool> {
    Ok(
        outstanding_notice(repos, tenant_id, user_id, oauth_providers::WHOOP)
            .await?
            .is_some(),
    )
}
