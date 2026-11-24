// ABOUTME: Wakes the server when a routine is due: a Cloud Tasks task per due slot, or an in-process timer locally
// ABOUTME: A wake is named by what it fires and when, so queuing the same wake twice queues it once

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The routine engine's waker (ADR-036, carnet#1790).
//!
//! The live environment scales to zero, so nothing in the process can wait
//! for a routine's slot: a timer only ticks while an instance is up, and the
//! 24-hour and 168-hour `spawn_periodic` workers were found never to fire
//! (carnet#459). Instead every due firing is queued as a Cloud Tasks HTTP task
//! scheduled for its time. Cloud Tasks wakes the service when it is due, and
//! the delivery runs the firing inside the request, so Cloud Run keeps the
//! instance up for exactly as long as the firing runs.
//!
//! Slots are on the hour, so routines due in the same hour arrive together
//! and share one cold start. A wake carries what it fires and when; its task
//! name is built from both ([`Wake::task_id`]), so queuing a wake that is
//! already queued is a no-op — the reconcile re-queues every pending wake
//! without duplicating any.
//!
//! Cloud Tasks delivers at least once. The firing guards itself: it acts only
//! while the routine's next firing is still the one the wake names, and the
//! post it makes is claimed before any model call.
//!
//! Locally, and wherever turns run in-process, an [`InProcessWaker`] holds
//! the same wakes as timers in the running process.

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::gcp_token::TokenProvider;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;

use crate::services::cloud_tasks::{create_http_task, HttpTask, TaskCreated};
use crate::services::turn_runner::CloudTasksRunner;

/// The path a wake is delivered to.
pub const WAKE_PATH: &str = "/internal/routines/wake";

/// A job the platform itself runs on a schedule, with no owner: it fires for
/// everyone it concerns at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SystemJob {
    /// The persona digest: returns the notifications an armed persona policy
    /// held back from push, on each athlete's daily or weekly cadence.
    PersonaDigest,
}

impl SystemJob {
    /// Every system job.
    pub const ALL: [Self; 1] = [Self::PersonaDigest];

    /// The job's name in task names and logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PersonaDigest => "persona_digest",
        }
    }

    /// The UTC hour the job runs at, every day. 11:00 UTC is 06:00–07:00 in
    /// Québec, so the pilot's athletes are never woken by it.
    const fn utc_hour(self) -> u32 {
        match self {
            Self::PersonaDigest => 11,
        }
    }

    /// How long after its hour a missed run still runs. The 3-hourly
    /// reconcile catches a lost wake inside it; the digest returns every
    /// held notification exactly once, so a second run sends nothing.
    pub const WINDOW: Elapsed = Elapsed::hours(3);

    /// The run a wake at `now` belongs to: today's, while still inside
    /// [`Self::WINDOW`], else the next.
    #[must_use]
    pub fn current_or_next(self, now: Timestamp) -> Timestamp {
        let today = now.utc_date();
        let at = |offset: i64| {
            today
                .checked_add_days(offset)
                .and_then(|date| date.and_hms(self.utc_hour(), 0, 0).ok())
                .and_then(|civil| civil.and_utc().ok())
        };
        match at(0) {
            Some(run) if run <= now && now < run + Self::WINDOW => run,
            Some(run) if run > now => run,
            _ => at(1).unwrap_or(now),
        }
    }

    /// The run after the one at `run`.
    #[must_use]
    pub fn next_after(self, run: Timestamp) -> Timestamp {
        self.current_or_next(run + Self::WINDOW)
    }
}

/// What a wake fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "wake", rename_all = "snake_case")]
pub enum Wake {
    /// One routine's pending firing.
    Routine {
        /// The routine's tenant.
        tenant_id: Uuid,
        /// The routine.
        routine_id: Uuid,
        /// The firing it is for, as epoch milliseconds: the routine acts only
        /// while this is still its next firing.
        fire_at_ms: i64,
    },
    /// A system job's run.
    System {
        /// The job.
        job: SystemJob,
        /// The run it is for, as epoch milliseconds.
        fire_at_ms: i64,
    },
}

impl Wake {
    /// A wake for `routine_id`'s firing at `fire_at`.
    #[must_use]
    pub fn routine(tenant_id: Uuid, routine_id: Uuid, fire_at: Timestamp) -> Self {
        Self::Routine {
            tenant_id,
            routine_id,
            fire_at_ms: fire_at.epoch_millis(),
        }
    }

    /// A wake for `job`'s run at `fire_at`.
    #[must_use]
    pub fn system(job: SystemJob, fire_at: Timestamp) -> Self {
        Self::System {
            job,
            fire_at_ms: fire_at.epoch_millis(),
        }
    }

    /// When the wake is due, as epoch milliseconds.
    #[must_use]
    pub const fn fire_at_ms(self) -> i64 {
        match self {
            Self::Routine { fire_at_ms, .. } | Self::System { fire_at_ms, .. } => fire_at_ms,
        }
    }

    /// When the wake is due; `None` for milliseconds no instant has (a body
    /// that did not come from [`Self::routine`] or [`Self::system`]).
    #[must_use]
    pub fn fire_at(self) -> Option<Timestamp> {
        Timestamp::from_epoch_millis(self.fire_at_ms()).ok()
    }

    /// The wake's task id: what it fires and when, so the same wake always
    /// has the same name. Cloud Tasks ids take letters, digits, `-` and `_`.
    #[must_use]
    pub fn task_id(self) -> String {
        let at = self.fire_at_ms().div_euclid(1000);
        match self {
            Self::Routine { routine_id, .. } => format!("routine-{routine_id}-{at}"),
            Self::System { job, .. } => format!("system-{}-{at}", job.as_str()),
        }
    }
}

/// Queues wakes for their time.
#[async_trait]
pub trait RoutineWaker: Send + Sync {
    /// Queue `wake` for [`Wake::fire_at`]. Queuing a wake that is already
    /// queued queues nothing new and is not an error.
    ///
    /// # Errors
    ///
    /// Returns an error when the wake could not be queued; the 3-hourly
    /// reconcile queues it again.
    async fn queue(&self, wake: Wake) -> AppResult<()>;
}

/// Production [`RoutineWaker`]: one Cloud Tasks HTTP task per wake, on the
/// queue turns use, delivered to [`WAKE_PATH`] with the same OIDC identity.
pub struct CloudTasksWaker {
    queue: String,
    target_url: String,
    service_account: String,
    api_base: String,
    /// The turn runner's own token source, shared with it: wakes and turns
    /// are queued under one identity.
    tokens: Arc<dyn TokenProvider>,
    dispatch_deadline: Duration,
}

impl fmt::Debug for CloudTasksWaker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloudTasksWaker")
            .field("queue", &self.queue)
            .field("target_url", &self.target_url)
            .finish_non_exhaustive()
    }
}

impl CloudTasksWaker {
    /// A waker on the turn runner's queue, target and identity. A firing runs
    /// one turn, so its delivery gets a turn's dispatch deadline.
    #[must_use]
    pub fn over(runner: &CloudTasksRunner) -> Self {
        let config = runner.config();
        Self {
            queue: config.queue.clone(),
            target_url: config.target_url.clone(),
            service_account: config.service_account.clone(),
            api_base: config.api_base.clone(),
            tokens: runner.token_provider(),
            dispatch_deadline: runner.dispatch_deadline(),
        }
    }

    /// The task's full name under the queue.
    fn task_name(&self, wake: Wake) -> String {
        format!("{}/tasks/{}", self.queue, wake.task_id())
    }
}

#[async_trait]
impl RoutineWaker for CloudTasksWaker {
    async fn queue(&self, wake: Wake) -> AppResult<()> {
        let body = serde_json::to_value(wake)
            .map_err(|e| AppError::internal(format!("a wake did not serialize: {e}")))?;
        let task = HttpTask {
            queue: &self.queue,
            name: self.task_name(wake),
            url: format!("{}{WAKE_PATH}", self.target_url),
            body,
            service_account: &self.service_account,
            audience: &self.target_url,
            dispatch_deadline: self.dispatch_deadline,
            schedule_time: wake.fire_at(),
        };
        match create_http_task(self.tokens.as_ref(), &self.api_base, &task).await {
            Ok(TaskCreated::Created) => {
                info!(task = %wake.task_id(), "routine wake queued");
                Ok(())
            }
            Ok(TaskCreated::AlreadyQueued) => Ok(()),
            Err(e) => {
                warn!(task = %wake.task_id(), error = %e, "Cloud Tasks refused a routine wake");
                Err(e)
            }
        }
    }
}

/// Local [`RoutineWaker`]: hands each wake to the in-process loop the engine
/// runs ([`crate::services::routine_engine::run_in_process_wakes`]).
#[derive(Debug, Clone)]
pub struct InProcessWaker {
    tx: mpsc::UnboundedSender<Wake>,
}

impl InProcessWaker {
    /// A waker and the receiving end the in-process loop drains.
    #[must_use]
    pub fn channel() -> (Self, mpsc::UnboundedReceiver<Wake>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self { tx }, rx)
    }
}

#[async_trait]
impl RoutineWaker for InProcessWaker {
    async fn queue(&self, wake: Wake) -> AppResult<()> {
        self.tx
            .send(wake)
            .map_err(|_| AppError::internal("the in-process routine loop has stopped"))
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    type TestResult = Result<(), Box<dyn Error>>;

    fn at(rfc3339: &str) -> Result<Timestamp, Box<dyn Error>> {
        Ok(Timestamp::parse_from_rfc3339(rfc3339)?)
    }

    #[test]
    fn a_wake_is_delivered_to_the_path_the_messaging_router_mounts() {
        assert_eq!(
            WAKE_PATH, "/internal/routines/wake",
            "bug: the waker posts to a path routes/messaging/mod.rs does not mount"
        );
    }

    #[test]
    fn the_same_wake_always_has_the_same_task_id() -> TestResult {
        let routine = Uuid::nil();
        let a = Wake::routine(Uuid::nil(), routine, at("2026-11-25T13:00:00Z")?);
        let b = Wake::routine(Uuid::nil(), routine, at("2026-11-25T13:00:00Z")?);
        let later = Wake::routine(Uuid::nil(), routine, at("2026-11-25T13:30:00Z")?);
        assert_eq!(
            a.task_id(),
            b.task_id(),
            "bug: one wake has two task names, so a re-queue duplicates it"
        );
        assert_ne!(
            a.task_id(),
            later.task_id(),
            "bug: a deferral shares its slot's task name and is refused as a duplicate"
        );
        assert!(
            a.task_id()
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "bug: a task id holds a character Cloud Tasks refuses"
        );
        Ok(())
    }

    #[test]
    fn a_wake_round_trips_through_its_body() -> TestResult {
        let wake = Wake::system(SystemJob::PersonaDigest, at("2026-11-25T11:00:00Z")?);
        let back: Wake = serde_json::from_value(serde_json::to_value(wake)?)?;
        assert_eq!(
            back, wake,
            "bug: a wake's body does not read back as the wake"
        );
        Ok(())
    }

    #[test]
    fn the_digest_runs_once_a_day_and_a_missed_run_is_caught_inside_its_window() -> TestResult {
        let job = SystemJob::PersonaDigest;
        assert_eq!(
            job.current_or_next(at("2026-11-25T09:00:00Z")?),
            at("2026-11-25T11:00:00Z")?,
            "bug: before 11:00 UTC the day's run is not the next one"
        );
        assert_eq!(
            job.current_or_next(at("2026-11-25T12:37:00Z")?),
            at("2026-11-25T11:00:00Z")?,
            "a reconcile inside the window re-queues the day's run"
        );
        assert_eq!(
            job.current_or_next(at("2026-11-25T15:00:00Z")?),
            at("2026-11-26T11:00:00Z")?,
            "bug: a run past its window is still the current one"
        );
        assert_eq!(
            job.next_after(at("2026-11-25T11:00:00Z")?),
            at("2026-11-26T11:00:00Z")?,
            "bug: the run after today's is not tomorrow's"
        );
        Ok(())
    }
}
