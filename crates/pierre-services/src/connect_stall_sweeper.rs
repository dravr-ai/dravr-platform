// ABOUTME: Background sweep that reports, once each, athletes stuck at the onboarding connect step for 30 minutes
// ABOUTME: profile_type answered, no connect_provider step, no real connection in that tenant: onboarding.stuck_at_connect

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Detection for an athlete who reaches connect and never gets past it.
//!
//! On 2026-11-24 an athlete could not connect Strava in web onboarding:
//! Strava turned the sign-in down and the window offered only a retry. Three
//! athletes hit it in six days, and the only one we heard about was the one
//! who told us. Each sign-in failure now reports `provider.connect_failed`,
//! but an athlete who cancels, closes the window or never tries reports
//! nothing at all, and that is exactly how the second athlete looked.
//!
//! This sweep closes that gap from the other side: it does not ask why, only
//! whether the athlete got through. An athlete who answered the `profile_type`
//! step 30 minutes ago and still has no `connect_provider` step (connected,
//! skipped or not applicable) and no real connection in that tenant is
//! reported as `onboarding.stuck_at_connect`, once.
//!
//! Once is the ledger's doing, not a table of its own: the sweep keeps its
//! watermark — the newest answer time it has reported up to — in the
//! [`WorkerRunRepository`] row named [`WATERMARK`], and each pass reports the
//! answers between that watermark and 30 minutes ago. [`spawn_periodic`]
//! already runs one pass at a time across instances, and a scale-to-zero
//! service that slept through several passes reports the whole gap on its
//! next one, late rather than never.
//!
//! Mirrors [`oauth_launch_sweeper`](crate::oauth_launch_sweeper), the sweep
//! that made the 2026-09-10 blank OAuth popup visible.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::AppResult;
use pierre_database::repositories::{
    StalledAtConnect, UserOnboardingRepository, UserRepository, WorkerRunRepository,
};
use tracing::{info, warn};
use uuid::Uuid;

use crate::periodic::spawn_periodic;

/// Sweep cadence: an athlete is reported within five minutes of crossing
/// [`STALL_AFTER`].
const SWEEP_INTERVAL: Duration = Duration::from_mins(5);

/// How long after answering `profile_type` an athlete with no connection
/// counts as stuck. Connecting takes a minute or two when it works.
const STALL_AFTER: Duration = Duration::from_mins(30);

/// How far back the first pass ever run looks, so the athletes stuck on the
/// day the sweep ships are reported rather than skipped.
const FIRST_LOOKBACK: Duration = Duration::from_hours(24);

/// The furthest back any pass looks, however long the service slept: an
/// athlete stuck for a week is a cohort question, not an alert.
const MAX_LOOKBACK: Duration = Duration::from_hours(24 * 7);

/// How many candidates one pass reads, newest answer first. The window is
/// always the newest answers, so this only bounds a pathological backlog.
const SCAN_LIMIT: i64 = 500;

/// The ledger row that holds the watermark: `last_run_at_ms` is the newest
/// answer time already reported up to. Never claimed, only read and written.
const WATERMARK: &str = "connect stall sweeper watermark";

/// Start the background connect-stall sweep.
///
/// Fire-and-forget and best-effort like every [`spawn_periodic`] worker: a
/// failed pass leaves the watermark where it was, so the next pass reports
/// what this one could not.
pub fn start_connect_stall_sweeper(
    onboarding: Arc<dyn UserOnboardingRepository>,
    users: Arc<dyn UserRepository>,
    ledger: Arc<dyn WorkerRunRepository>,
) {
    let watermarks = Arc::clone(&ledger);
    spawn_periodic("connect stall sweeper", SWEEP_INTERVAL, ledger, move || {
        let onboarding = Arc::clone(&onboarding);
        let users = Arc::clone(&users);
        let watermarks = Arc::clone(&watermarks);
        async move {
            sweep_stalled_connects(
                onboarding.as_ref(),
                users.as_ref(),
                watermarks.as_ref(),
                Timestamp::now(),
            )
            .await
            .map(|_| ())
        }
    });
}

/// One pass at `now`: report every athlete whose `profile_type` answer fell
/// between the watermark and `now - STALL_AFTER`, then move the watermark
/// there. Returns how many were reported.
///
/// Every user is read before anything is reported, so a failed read reports
/// nobody and the retry reports everyone once. A user who no longer exists
/// is not reported.
///
/// # Errors
///
/// Returns the repository error of the scan, the user read or the watermark
/// read or write.
async fn sweep_stalled_connects(
    onboarding: &dyn UserOnboardingRepository,
    users: &dyn UserRepository,
    ledger: &dyn WorkerRunRepository,
    now: Timestamp,
) -> AppResult<usize> {
    let cutoff_ms = now.epoch_millis().saturating_sub(millis(STALL_AFTER));
    let previous = ledger
        .get_worker_run(WATERMARK)
        .await?
        .map(|run| run.last_run_at_ms)
        .filter(|ms| *ms > 0);
    let after_ms = window_start(previous, cutoff_ms);

    let candidates = onboarding.list_stalled_at_connect(SCAN_LIMIT).await?;
    let due = due_in_window(&candidates, after_ms, cutoff_ms);
    let ids: Vec<Uuid> = due.iter().filter_map(|stall| stall.user).collect();
    let accounts = if ids.is_empty() {
        HashMap::default()
    } else {
        users.get_global_many(&ids).await?
    };

    let mut reported = 0;
    for stall in &due {
        let Some(account) = stall.user.and_then(|id| accounts.get(&id)) else {
            continue;
        };
        let minutes_since_profile_type =
            now.epoch_millis().saturating_sub(stall.answered_ms) / 60_000;
        let seconds_since_signup = now
            .signed_duration_since(account.created_at)
            .num_seconds()
            .max(0);
        info!(
            target: "notify",
            event = "onboarding.stuck_at_connect",
            user_id = %account.id,
            tenant_id = stall.row.tenant_id.as_str(),
            minutes_since_profile_type,
            seconds_since_signup,
            "athlete stuck at the onboarding connect step"
        );
        reported += 1;
    }
    if candidates.len() >= usize::try_from(SCAN_LIMIT).unwrap_or(usize::MAX) {
        warn!(
            scanned = candidates.len(),
            "connect stall sweep read its whole scan limit; older stalls may be unreported"
        );
    }
    ledger.finish_worker_run(WATERMARK, cutoff_ms, None).await?;
    Ok(reported)
}

/// One candidate inside the window, with what the report needs.
struct DueStall<'a> {
    row: &'a StalledAtConnect,
    /// The athlete's id, `None` when the stored id is not a UUID.
    user: Option<Uuid>,
    answered_ms: i64,
}

/// Where this pass's window opens: the watermark, or [`FIRST_LOOKBACK`]
/// before the cutoff when no pass has run, never more than [`MAX_LOOKBACK`].
fn window_start(previous_ms: Option<i64>, cutoff_ms: i64) -> i64 {
    let floor = cutoff_ms.saturating_sub(millis(MAX_LOOKBACK));
    previous_ms
        .unwrap_or_else(|| cutoff_ms.saturating_sub(millis(FIRST_LOOKBACK)))
        .max(floor)
}

/// The candidates whose answer fell in `(after_ms, cutoff_ms]`, one per
/// athlete and tenant. An answer whose time does not read is skipped: it
/// cannot be placed in any window.
fn due_in_window(
    candidates: &[StalledAtConnect],
    after_ms: i64,
    cutoff_ms: i64,
) -> Vec<DueStall<'_>> {
    let mut seen = HashSet::new();
    candidates
        .iter()
        .filter_map(|row| {
            let answered_ms = row.profile_type_at?.epoch_millis();
            (answered_ms > after_ms && answered_ms <= cutoff_ms).then_some((row, answered_ms))
        })
        .filter(|(row, _)| seen.insert((row.user_id.as_str(), row.tenant_id.as_str())))
        .map(|(row, answered_ms)| DueStall {
            row,
            user: Uuid::parse_str(&row.user_id).ok(),
            answered_ms,
        })
        .collect()
}

fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use pierre_test_support::db::create_test_db;
    use pierre_test_support::log_capture::{self, capture_notify};
    use pierre_test_support::server::create_test_user;

    use super::*;

    const MINUTE: i64 = 60_000;

    fn stall(user: &str, at_ms: Option<i64>) -> StalledAtConnect {
        StalledAtConnect {
            user_id: user.to_owned(),
            tenant_id: "tenant".to_owned(),
            profile_type_at: at_ms.and_then(|ms| Timestamp::from_epoch_millis(ms).ok()),
        }
    }

    #[test]
    fn only_answers_inside_the_window_are_due() {
        let cutoff = 1_000 * MINUTE;
        let after = cutoff - 5 * MINUTE;
        let candidates = [
            stall("too-recent", Some(cutoff + 1)),
            stall("at-cutoff", Some(cutoff)),
            stall("inside", Some(after + 1)),
            stall("at-watermark", Some(after)),
            stall("unreadable", None),
        ];
        let due: Vec<_> = due_in_window(&candidates, after, cutoff)
            .into_iter()
            .map(|d| d.row.user_id.as_str())
            .collect();
        assert_eq!(
            due,
            ["at-cutoff", "inside"],
            "bug: the window must be (watermark, cutoff], or an athlete is reported twice or never"
        );
    }

    #[test]
    fn an_athlete_is_due_once_per_tenant() {
        let candidates = [stall("a", Some(10 * MINUTE)), stall("a", Some(9 * MINUTE))];
        assert_eq!(
            due_in_window(&candidates, 0, 20 * MINUTE).len(),
            1,
            "bug: two answers by one athlete in one tenant reported them twice"
        );
    }

    #[test]
    fn the_window_opens_at_the_watermark_within_the_lookback() {
        let cutoff = 100 * 24 * 60 * MINUTE;
        assert_eq!(
            window_start(Some(cutoff - 5 * MINUTE), cutoff),
            cutoff - 5 * MINUTE,
            "a pass after a pass opens where the last one closed"
        );
        assert_eq!(
            window_start(None, cutoff),
            cutoff - millis(FIRST_LOOKBACK),
            "the first pass ever looks back a day"
        );
        assert_eq!(
            window_start(Some(1), cutoff),
            cutoff - millis(MAX_LOOKBACK),
            "a long sleep looks back a week at most"
        );
    }

    /// The `onboarding.stuck_at_connect` reports a pass made for `user_id`.
    fn reports_for(
        events: Vec<HashMap<String, String>>,
        user_id: &str,
    ) -> Vec<HashMap<String, String>> {
        events
            .into_iter()
            .filter(|e| {
                e.get("event").map(String::as_str) == Some("onboarding.stuck_at_connect")
                    && e.get("user_id").map(String::as_str) == Some(user_id)
            })
            .collect()
    }

    #[tokio::test]
    async fn a_stuck_athlete_is_reported_once() {
        let db = create_test_db().await.expect("test database");
        let repos = db.repositories();
        let user = create_test_user(&format!("{}@stall.test", Uuid::new_v4()), None);
        let user_id = repos
            .users
            .create(&user)
            .await
            .expect("user row")
            .to_string();
        let tenant_id = Uuid::new_v4().to_string();
        repos
            .user_onboarding
            .set_onboarding_step(&user_id, "profile_type", "complete", None, Some(&tenant_id))
            .await
            .expect("answer profile_type");

        // The answer was just written: 31 minutes from now it is past the stall.
        let now = Timestamp::now();
        let later = Timestamp::from_epoch_millis(now.epoch_millis() + 31 * MINUTE)
            .expect("an instant 31 minutes on");
        let (events, _guard) = capture_notify();

        sweep_stalled_connects(
            repos.user_onboarding.as_ref(),
            repos.users.as_ref(),
            repos.worker_runs.as_ref(),
            now,
        )
        .await
        .expect("sweep now");
        assert!(
            reports_for(log_capture::drain(&events), &user_id).is_empty(),
            "bug: an athlete is reported before 30 minutes have passed"
        );

        sweep_stalled_connects(
            repos.user_onboarding.as_ref(),
            repos.users.as_ref(),
            repos.worker_runs.as_ref(),
            later,
        )
        .await
        .expect("sweep later");
        let reported = reports_for(log_capture::drain(&events), &user_id);
        assert_eq!(reported.len(), 1, "bug: a stuck athlete was not reported");
        assert_eq!(
            reported[0].get("tenant_id"),
            Some(&tenant_id),
            "bug: the report names another tenant than the answer's"
        );
        let minutes: i64 = reported[0]
            .get("minutes_since_profile_type")
            .expect("the report carries the minutes since the answer")
            .parse()
            .expect("the minutes are a number");
        assert!(
            (30..=32).contains(&minutes),
            "bug: minutes since the answer read {minutes}, not about 31"
        );

        sweep_stalled_connects(
            repos.user_onboarding.as_ref(),
            repos.users.as_ref(),
            repos.worker_runs.as_ref(),
            later,
        )
        .await
        .expect("sweep again");
        assert!(
            reports_for(log_capture::drain(&events), &user_id).is_empty(),
            "bug: the next pass reported the same athlete again"
        );
    }
}
