// ABOUTME: The athlete's recent training load, computed from cached activities
// ABOUTME: One source for the calibration baseline and the plan-save ramp check
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Recent-load figures derived from the activity cache.
//!
//! Two callers need the same number and must not disagree about it: the
//! calibration interview quotes the athlete's recent weekly hours back to them
//! for confirmation, and the plan-save ramp check compares the first planned
//! week against that same figure. If they were computed separately, an athlete
//! could confirm "yes, seven hours is right" and then have a nine-hour opening
//! week pass unwarned because the check measured something else.
//!
//! The source is the activity **cache**, not a live provider call: the figures
//! are wanted synchronously inside a command or a tool save, and the cache is
//! the copy the rest of the platform already reasons about. An athlete with no
//! connected provider — or one whose cache is empty — yields `None` rather than
//! a zeroed snapshot, so callers ask cold instead of quoting a fabricated
//! baseline.

use chrono::{Duration, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::{Activity, LoadSnapshot, SportFamily, TenantId};
use pierre_database::repositories::ActivityCacheRepository;
use uuid::Uuid;

/// How many weeks of history the snapshot averages over.
///
/// Six weeks is long enough to survive one disrupted week and short enough to
/// still describe current form; it is also the window the interview's
/// baseline-confirm question is worded around.
pub const SNAPSHOT_WEEKS: u32 = 6;

/// Upper bound on activities pulled for the window. Six weeks of even a very
/// high-frequency athlete (three sessions a day) stays well inside this, so the
/// cap bounds memory without truncating a real history.
const ACTIVITY_FETCH_LIMIT: i64 = 500;

/// Compute the athlete's recent-load snapshot over [`SNAPSHOT_WEEKS`].
///
/// Returns `None` when the window holds no cached activity — no connected
/// provider, a fresh account, or a genuine six-week layoff. All three mean the
/// same thing to a caller: there is no baseline to quote, and inventing one
/// would be worse than asking.
///
/// # Errors
/// Returns the repository error when the activity cache cannot be read.
pub async fn recent_load_snapshot(
    activities: &dyn ActivityCacheRepository,
    user_id: Uuid,
    tenant_id: &TenantId,
) -> AppResult<Option<LoadSnapshot>> {
    let end = Utc::now();
    let start = end - Duration::weeks(i64::from(SNAPSHOT_WEEKS));
    let window = activities
        .get_cached_activities(user_id, tenant_id, None, start, end, ACTIVITY_FETCH_LIMIT)
        .await?;
    let families = SportFamily::distinct(window.iter().map(Activity::sport_type));
    Ok(snapshot_from_durations(
        &window
            .iter()
            .map(Activity::duration_seconds)
            .collect::<Vec<_>>(),
        SNAPSHOT_WEEKS,
    )
    .map(|snapshot| LoadSnapshot {
        sport_families: families,
        ..snapshot
    }))
}

/// The snapshot for a window of session durations, in seconds.
///
/// Split out from the fetch so the arithmetic is testable without a database,
/// and so the ramp check can reuse it over planned durations.
#[must_use]
pub fn snapshot_from_durations(durations_seconds: &[u64], weeks: u32) -> Option<LoadSnapshot> {
    if durations_seconds.is_empty() || weeks == 0 {
        return None;
    }
    let total_seconds: u64 = durations_seconds.iter().sum();
    let weeks_f = f64::from(weeks);
    #[expect(
        clippy::cast_precision_loss,
        reason = "session-second totals are far below f64's exact-integer range"
    )]
    let total_hours = total_seconds as f64 / 3600.0;
    #[expect(
        clippy::cast_precision_loss,
        reason = "session counts are small; precision loss cannot occur at this magnitude"
    )]
    let session_count = durations_seconds.len() as f64;
    let longest = durations_seconds.iter().copied().max().unwrap_or(0);

    Some(LoadSnapshot {
        weekly_hours: total_hours / weeks_f,
        sessions_per_week: session_count / weeks_f,
        longest_session_min: u32::try_from(longest / 60).unwrap_or(u32::MAX),
        weeks,
        // Durations carry no sport; the fetching caller counts families.
        sport_families: 0,
    })
}
