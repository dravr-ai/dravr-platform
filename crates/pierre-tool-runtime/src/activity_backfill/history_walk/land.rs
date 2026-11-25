// ABOUTME: What a history walk step does with what it read: lands the rows, says where the next step starts
// ABOUTME: A capture short of its window lands below the head and reads on below its oldest row; the head refuses it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Landing a step (carnet#1698).
//!
//! A session read that covered some months of a step's window and then failed on one
//! returns what it read, short of its window (`head_complete` false), and
//! never skips the month it failed on. A read that covers the list head (the
//! first step's) refuses such a capture as every write-through does
//! ([`capture_with_head`]). A read below the head lands it: its write is no
//! sync ([`sync_verdict::covers_list_head`]), so the head's freshness is untouched, and the
//! next step reads below the oldest row it landed, never below its window.
//!
//! A step below the head that read nothing below its cursor short of its
//! window — no row at all, or only rows at or above the cursor, as a month
//! walk that reads the cursor's month whole returns — failed on the first
//! month below its cursor. When the walk's own key holds rows below that
//! cursor, the failure sits inside stored history: the step failed. When it
//! holds none, the source may simply hold nothing older (a month before the
//! account answers an error): the step falls short ([`Step::FellShort`]) and
//! is tried again on each attempt, a lease apart, and only once its attempts
//! at that cursor are spent has the walk reached the start of the history:
//! the session phase ends there, handing off to a grant beside it as any
//! empty step does, or ends the walk walked (carnet#1780).

use std::time::Instant;

use dravr_tronc::dates::Timestamp;

use pierre_core::models::Activity;
use pierre_database::repositories::{JobLease, JobProgressStep};
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::CoreFitnessProvider;
use tracing::{info, warn};

use super::{months_between, no_step, oldest_stored, Cursor, Step};
use crate::activity_backfill::{
    capture_with_head, owed_write, warm_training_history, ActivityBackfillJob, BackfillFetch,
};
use crate::activity_fetch::sync_verdict;
use crate::activity_fetch::write_through::WriteThrough;
use crate::protocol::UniversalExecutor;

/// The span one step read.
#[derive(Debug, Clone, Copy)]
pub(super) struct StepSpan<'step> {
    /// The request that returned the rows.
    pub(super) read: &'step ActivityQueryParams,
    /// The newest instant the step covers (unix seconds).
    pub(super) top: i64,
    /// The oldest instant the request covered.
    pub(super) bottom: i64,
    /// Where the step started.
    pub(super) cursor: Cursor,
    /// When the step started, for its seconds per month.
    pub(super) started: Instant,
    /// The lease the step holds its row under; none for a walk the ledger
    /// could not record.
    pub(super) lease: Option<&'step JobLease>,
}

/// A session step that read nothing between `bottom` and the cursor: the end
/// of the history when the capture covered its window, else a failure on the
/// head, a failure inside stored history, or a step that fell short at the
/// bottom of what the walk stored ([`Step::FellShort`]).
pub(super) async fn read_nothing(
    job: &ActivityBackfillJob,
    provider: &dyn CoreFitnessProvider,
    cursor: Cursor,
    bottom: i64,
) -> Step {
    if provider.head_complete() {
        let mut step = no_step(bottom);
        step.units_done = months_between(cursor.top, bottom);
        return Step::Ended(step);
    }
    if cursor.first {
        warn!(user_id = %job.user_id, provider = %job.provider_name, "History walk: the first step read nothing and fell short of its head; it is no end of the history");
        return Step::Failed;
    }
    fell_short(job, cursor).await
}

/// A step below the head that read nothing below its cursor short of its
/// window: failed when the walk's key holds rows below its cursor, else
/// fallen short at the bottom of what the walk stored.
async fn fell_short(job: &ActivityBackfillJob, cursor: Cursor) -> Step {
    let key = job.provider_name.as_str();
    match oldest_stored(&job.resources, job.user_id, job.tenant_id, &[key]).await {
        Ok(Some(oldest)) if oldest.epoch_seconds() < cursor.top => {
            warn!(user_id = %job.user_id, provider = key, before = cursor.top, "History walk: a step fell short of its window above rows already stored; it failed");
            Step::Failed
        }
        Ok(_) => {
            info!(user_id = %job.user_id, provider = key, before = cursor.top, "History walk: a step read nothing below its cursor and fell short of its window at the oldest stored row");
            Step::FellShort(no_step(cursor.top))
        }
        Err(()) => Step::Failed,
    }
}

/// Write a step's `activities` through the store while its row is still its
/// own ([`owed_write::land_owed`]), and say where the next step starts: below
/// the oldest activity when the request filled its limit (more lies in its
/// window) or fell short of it below the head, else below the window.
pub(super) async fn land(
    job: &ActivityBackfillJob,
    executor: &UniversalExecutor,
    provider: &dyn CoreFitnessProvider,
    activities: Vec<Activity>,
    span: StepSpan<'_>,
) -> Step {
    let fetched = activities.len();
    let short = !provider.head_complete()
        && !sync_verdict::covers_list_head(span.read, Timestamp::now().epoch_seconds());
    let activities = if short {
        activities
    } else {
        match capture_with_head(job, provider, activities) {
            BackfillFetch::Activities(activities) => activities,
            BackfillFetch::AuthRequired => return Step::AuthRequired,
            BackfillFetch::Failed => return Step::Failed,
            BackfillFetch::Paused(until_ms) => return Step::Paused(until_ms),
        }
    };
    let oldest = activities
        .iter()
        .map(|activity| activity.start_date().epoch_seconds())
        .min()
        .unwrap_or(span.bottom);
    if short {
        // The next step must read strictly below this one, or it reads the
        // same rows again on every pass: a capture holding nothing below its
        // cursor read nothing new, and falls short as an empty one does.
        if oldest >= span.cursor.top {
            info!(user_id = %job.user_id, provider = %job.provider_name, fetched, "History walk: a capture short of its window held nothing below its cursor");
            return fell_short(job, span.cursor).await;
        }
        info!(user_id = %job.user_id, provider = %job.provider_name, fetched, oldest, "History walk: a step's capture fell short of its window; its rows land and the next step reads below the oldest");
    }
    let write = WriteThrough {
        read: span.read,
        // A walk copies the past: it lands no new session.
        announces_sessions: false,
    };
    let landed = match owed_write::land_owed(job, executor, &activities, write, span.lease).await {
        owed_write::Landed::Rows(landed) => landed,
        owed_write::Landed::Kept => {
            warm_training_history(job).await;
            return Step::Kept;
        }
        owed_write::Landed::Stopped => return Step::Stopped,
        owed_write::Landed::Failed => return Step::Failed,
    };
    warm_training_history(job).await;
    let next = if short || (fetched >= span.cursor.limit && oldest < span.top) {
        oldest
    } else {
        span.bottom
    };
    let months = months_between(span.top, next);
    let step = JobProgressStep {
        before_ts: next,
        units_done: months,
        rows_landed: i64::try_from(landed).unwrap_or(i64::MAX),
        secs_per_unit: secs_per_month(span.started, months),
        floor: None,
    };
    if next <= span.cursor.floor {
        Step::Ended(step)
    } else {
        Step::Walked(step)
    }
}

/// The seconds one month of a step took, or `None` for a step that covered
/// less than a month, which measures nothing.
fn secs_per_month(started: Instant, months: i64) -> Option<f64> {
    let months = i32::try_from(months).ok().filter(|months| *months > 0)?;
    Some(started.elapsed().as_secs_f64() / f64::from(months))
}
