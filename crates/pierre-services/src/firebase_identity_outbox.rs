// ABOUTME: Drains the Firebase identity deletion outbox — settles each attempt and retries the rest with backoff
// ABOUTME: One settle path for the post-commit attempt and the sweep; an identity stuck past N attempts logs at ERROR

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Firebase identity deletion, made durable (carnet#798).
//!
//! The account delete queues the account's Firebase uid in
//! `firebase_identity_deletions` inside its own transaction. Right after the
//! commit, `user_removal::remove_user` tries the deletion once and settles the
//! row through [`settle`]: confirmed (or already gone at Google) removes it, a
//! failure counts the attempt and holds the row for a backoff. The sweep here
//! retries every due row on the worker ledger, through the same [`settle`].
//!
//! A row that has failed [`ALERT_AFTER_ATTEMPTS`] times is logged once at
//! ERROR with `event = "firebase_identity.delete_stalled"`, which the platform
//! forwards to the operators' Slack channel like every ERROR; the sweep keeps
//! retrying it after that, at the capped backoff.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use pierre_auth::firebase_identity::{FirebaseIdentityDeleter, FirebaseIdentityRemoval};
use pierre_core::errors::AppResult;
use pierre_database::repositories::{FirebaseIdentityDeletionRepository, WorkerRunRepository};
use tracing::{error, info, warn};

use crate::periodic::spawn_periodic;

/// The ledger name the sweep runs under.
const WORKER_NAME: &str = "firebase identity deletion sweeper";

/// Sweep cadence: a queued identity is retried within a quarter hour of its
/// backoff running out.
const SWEEP_INTERVAL: Duration = Duration::from_mins(15);

/// Rows retried per pass; the rest wait for the next one.
const SWEEP_BATCH: i64 = 50;

/// Wait after the first failure; doubled after each further one.
const FIRST_RETRY: TimeDelta = TimeDelta::minutes(15);

/// The longest wait between two attempts.
const MAX_RETRY: TimeDelta = TimeDelta::hours(24);

/// Failed attempts after which a stuck identity is raised to the operators.
pub const ALERT_AFTER_ATTEMPTS: i64 = 5;

/// When to try again after `attempts` failed attempts (`attempts >= 1`):
/// [`FIRST_RETRY`] doubled per earlier failure, capped at [`MAX_RETRY`].
#[must_use]
fn next_attempt_after(attempts: i64, now: DateTime<Utc>) -> DateTime<Utc> {
    let doublings = u32::try_from(attempts.saturating_sub(1).clamp(0, 16)).unwrap_or(16);
    let wait = FIRST_RETRY
        .checked_mul(1_i32 << doublings)
        .map_or(MAX_RETRY, |wait| wait.min(MAX_RETRY));
    now.checked_add_signed(wait).unwrap_or(now)
}

/// Record what one deletion attempt of `firebase_uid` came to.
///
/// A settled identity leaves the outbox; a failure is counted (on top of
/// `prior_attempts`) and held for the backoff, and raised at ERROR the time
/// it reaches [`ALERT_AFTER_ATTEMPTS`].
///
/// # Errors
///
/// Returns the database error when the outbox row cannot be updated; the row
/// then stays due and the sweep retries it.
pub(crate) async fn settle(
    outbox: &dyn FirebaseIdentityDeletionRepository,
    firebase_project: &str,
    firebase_uid: &str,
    removal: &FirebaseIdentityRemoval,
    prior_attempts: i64,
    now: DateTime<Utc>,
) -> AppResult<()> {
    let FirebaseIdentityRemoval::Failed { error: cause } = removal else {
        if removal.is_settled() {
            outbox.complete(firebase_uid).await?;
        }
        return Ok(());
    };
    let attempts = prior_attempts.saturating_add(1);
    let retry_at = next_attempt_after(attempts, now);
    let recorded = outbox
        .record_failure(firebase_uid, cause, retry_at)
        .await?
        .unwrap_or(attempts);
    if recorded == ALERT_AFTER_ATTEMPTS {
        error!(
            event = "firebase_identity.delete_stalled",
            firebase_uid = %firebase_uid,
            firebase_project = %firebase_project,
            attempts = recorded,
            error = %cause,
            "A deleted account's Firebase identity still cannot be deleted at Google; the sweep keeps retrying"
        );
    } else {
        warn!(
            firebase_uid = %firebase_uid,
            firebase_project = %firebase_project,
            attempts = recorded,
            retry_at = %retry_at,
            error = %cause,
            "Firebase identity deletion failed; queued for retry"
        );
    }
    Ok(())
}

/// Retry every Firebase identity deletion due at `now`, up to one batch, and
/// settle each. Returns how many identities left the outbox.
///
/// # Errors
///
/// Returns the first database error; rows settled before it stay settled.
pub async fn sweep_firebase_identity_deletions(
    outbox: &dyn FirebaseIdentityDeletionRepository,
    deleter: &FirebaseIdentityDeleter,
    now: DateTime<Utc>,
) -> AppResult<usize> {
    let pending = outbox.due(now, SWEEP_BATCH).await?;
    let mut gone = 0;
    for row in &pending {
        let removal = deleter
            .delete_user(&row.firebase_project, &row.firebase_uid)
            .await;
        if removal.is_settled() {
            gone += 1;
        }
        settle(
            outbox,
            &row.firebase_project,
            &row.firebase_uid,
            &removal,
            row.attempts,
            now,
        )
        .await?;
    }
    if !pending.is_empty() {
        info!(
            due = pending.len(),
            deleted = gone,
            "Retried pending Firebase identity deletions"
        );
    }
    Ok(gone)
}

/// Start the sweep on [`spawn_periodic`]. A server with no Firebase project
/// configured queues nothing and does not start it.
pub fn start_firebase_identity_sweeper(
    outbox: Arc<dyn FirebaseIdentityDeletionRepository>,
    deleter: Option<Arc<FirebaseIdentityDeleter>>,
    ledger: Arc<dyn WorkerRunRepository>,
) {
    let Some(deleter) = deleter else {
        info!("Firebase is not configured; the Firebase identity deletion sweeper is not started");
        return;
    };
    spawn_periodic(WORKER_NAME, SWEEP_INTERVAL, ledger, move || {
        let outbox = Arc::clone(&outbox);
        let deleter = Arc::clone(&deleter);
        async move {
            sweep_firebase_identity_deletions(outbox.as_ref(), &deleter, Utc::now())
                .await
                .map(|_| ())
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_backoff_doubles_from_a_quarter_hour_and_caps_at_a_day() {
        let now = Utc::now();
        assert_eq!(next_attempt_after(1, now) - now, TimeDelta::minutes(15));
        assert_eq!(next_attempt_after(2, now) - now, TimeDelta::minutes(30));
        assert_eq!(next_attempt_after(4, now) - now, TimeDelta::hours(2));
        assert_eq!(next_attempt_after(8, now) - now, MAX_RETRY);
        assert_eq!(next_attempt_after(i64::MAX, now) - now, MAX_RETRY);
    }
}
