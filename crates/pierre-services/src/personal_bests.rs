// ABOUTME: An athlete's all-time best efforts at 5 km, 10 km, half and full marathon, and the push a new one sends
// ABOUTME: A one-time paced walk of their history seeds the bests silently; only then is a faster synced run told

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Personal records at the standard running distances.
//!
//! A best effort is cageux's: the shortest elapsed time of any contiguous
//! stretch of a run covering exactly the distance, read from the run's
//! cumulative distance channel and interpolated at both edges
//! ([`best_efforts`]). Each run is measured once — the scan is recorded
//! whatever it found — and every best it sets is stored per athlete, tenant
//! and distance.
//!
//! What the athlete hears depends on how much of their history is known:
//!
//! - **The whole history is measured first.** [`PersonalBests::advance_seed`]
//!   walks the athlete's activities at the provider once, newest first, one
//!   listing per page and one samples request per past run, and stores the
//!   bests they set without telling anything. The walk runs
//!   [`in_background`], so each of its provider requests is admitted against
//!   only the background share of the signing app's budget; it stops when that
//!   share is spent, and resumes on a later pass from the cursor it saved,
//!   skipping every run `best_effort_scans` already holds.
//! - **Nothing is told against a partial history.** Until the walk reached
//!   the athlete's first activity, a synced run is measured and its bests
//!   stored like the walk's, and nothing is announced: a time faster than the
//!   bests known so far may still be slower than one the walk has not reached.
//! - **A faster time is told.** Once the walk is complete, an effort strictly
//!   faster, in whole seconds, than the stored best at its distance replaces
//!   it and sends one [`trigger_personal_record`] naming the distance and the
//!   new time.
//! - **A first effort at a distance is the baseline.** Once the walk is
//!   complete, a distance with no stored best is one the athlete's whole
//!   history never covered, so the effort becomes the baseline for it rather
//!   than a record.
//!
//! A run is measured at most once: whoever measures it records the scan, and
//! the walk and a sync scanning the same athlete take the athlete's lease in
//! turn, so neither asks for a run's samples while the other holds it. The
//! lease is a row in the worker ledger — the one `spawn_periodic` leases its
//! ticks from — so it holds across every instance of the backend, and it
//! carries an expiry, so an instance that dies holding it frees the athlete
//! when [`ATHLETE_LEASE`] runs out.
//!
//! A samples request that fails is held until the provider answers the next
//! request — the next run's samples, or the next page of the walk. When it
//! answers, the failure was the run's own and the run is recorded unmeasured,
//! so it is never asked for again; when it refuses that one too, the provider
//! is refusing (a spent quota, an open circuit, a lost grant) and the batch
//! stops with neither run recorded, so both are asked for again later.
//!
//! Only runs are measured, as cageux's own record detection does: `Run`,
//! `VirtualRun` and `TrailRunning`.

use std::cmp::Reverse;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use dravr_cageux::best_efforts::{best_efforts, STANDARD_RUNNING_DISTANCES_METERS};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{Activity, SportType, TenantId, TimeSeriesData};
use pierre_database::repositories::{
    PersonalBest, PersonalBestRepository, PersonalBestSeed, WorkerRunRepository,
};
use pierre_notifications::triggers::trigger_personal_record;
use pierre_notifications::{NotificationService, TenantId as CommTenantId};
use pierre_providers::core::{ActivityQueryParams, FitnessProvider};
use pierre_providers::request_budget::in_background;
use tokio::time::sleep;
use tracing::{info, warn};
use uuid::Uuid;

use crate::notification_text::{
    PR_DISTANCE_10K, PR_DISTANCE_5K, PR_DISTANCE_HALF_MARATHON, PR_DISTANCE_MARATHON,
};

/// The catalogue code of each standard distance, index-aligned with
/// [`STANDARD_RUNNING_DISTANCES_METERS`]; the array length ties the two.
const DISTANCE_CODES: [&str; STANDARD_RUNNING_DISTANCES_METERS.len()] = [
    PR_DISTANCE_5K,
    PR_DISTANCE_10K,
    PR_DISTANCE_HALF_MARATHON,
    PR_DISTANCE_MARATHON,
];

/// The activities one page of the history walk asks for: Strava's page cap.
///
/// The walk ends on an empty page rather than a short one, so a provider that
/// caps its pages lower is walked just as completely.
pub const SEED_PAGE_SIZE: usize = 200;

/// How long a batch holds an athlete before another instance may take them
/// over.
///
/// A walk's batch makes at most the background share of one Strava window
/// (25 requests) and a sync's scan one request per new run, so either ends
/// well inside it; a batch that dies with its instance frees the athlete when
/// it runs out. The same length as a worker tick's lease.
pub const ATHLETE_LEASE: Duration = Duration::from_mins(15);

/// How long a sync's scan waits for an athlete the walk holds before leaving
/// its runs to the next sync: a walk's batch is its share of one window, a
/// few seconds of requests.
const SCAN_PATIENCE: Duration = Duration::from_mins(1);

/// How often a waiting scan asks for the athlete's lease again.
const LEASE_RETRY: Duration = Duration::from_secs(2);

/// The worker-ledger row that leases the measuring of the athlete's runs in
/// `tenant_id`, across every instance.
#[must_use]
pub fn athlete_lease_name(user_id: Uuid, tenant_id: TenantId) -> String {
    format!("personal best athlete {user_id} {tenant_id}")
}

/// A duration in milliseconds, saturating.
fn millis(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

/// What one scanned run did to the athlete's best at one distance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BestOutcome {
    /// No best was stored at the distance; this effort is now the baseline.
    Seeded,
    /// The effort beat the stored best, which it replaced.
    Improved {
        /// The best it replaced, in whole seconds.
        previous_seconds: f64,
    },
    /// The effort was not faster than the stored best.
    Slower,
}

/// One distance a scanned run covered, and what that did to the best.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DistanceResult {
    /// The distance's catalogue code.
    pub distance: &'static str,
    /// The run's effort at the distance, in whole seconds.
    pub elapsed_seconds: f64,
    /// What the effort did to the stored best.
    pub outcome: BestOutcome,
}

/// Why a walk or a scan stopped before its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedPause {
    /// The shared request budget said stop; the rest waits for its next window.
    Budget,
    /// The provider refused a listing, or two samples requests in a row.
    Refused,
}

/// Where a call to [`PersonalBests::advance_seed`] left the athlete's walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedProgress {
    /// Every past run was measured; records are announced from now on.
    Complete,
    /// The walk stopped part-way; the next call resumes where it stopped.
    Paused(SeedPause),
    /// Another batch — a walk on another instance, or a sync's scan — holds
    /// the athlete; the walk was not advanced.
    Held,
}

/// Whether best efforts are measured on `sport`.
#[must_use]
pub const fn is_measured_sport(sport: &SportType) -> bool {
    matches!(
        sport,
        SportType::Run | SportType::VirtualRun | SportType::TrailRunning
    )
}

/// An elapsed time as an athlete reads it: `h:mm:ss` from an hour up,
/// `m:ss` below, rounded to the second.
#[must_use]
pub fn format_effort_time(seconds: f64) -> String {
    let total = whole_seconds(seconds);
    let (hours, minutes, secs) = (total / 3600, (total % 3600) / 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

/// Seconds rounded to the nearest whole one; negative or non-finite is 0.
fn whole_seconds(seconds: f64) -> u64 {
    if seconds.is_finite() && seconds > 0.0 {
        // Finite and positive, and a best effort is hours at most, so the
        // rounded value fits a u64 exactly.
        seconds.round() as u64
    } else {
        0
    }
}

/// Stores the athlete's best efforts and tells them about each new one.
#[derive(Clone)]
pub struct PersonalBests {
    /// Where the bests, the scanned runs and the walk's progress are kept.
    repo: Arc<dyn PersonalBestRepository>,
    /// Where a new best is announced. `None` when the deployment runs without
    /// the notification backend: bests are then stored and nothing is sent.
    service: Option<Arc<NotificationService>>,
    /// The worker ledger, where the lease on each athlete's measuring lives.
    leases: Arc<dyn WorkerRunRepository>,
}

impl PersonalBests {
    /// Personal bests over `repo`, announced through `service`, with each
    /// athlete's measuring leased in `leases`.
    #[must_use]
    pub fn new(
        repo: Arc<dyn PersonalBestRepository>,
        service: Option<Arc<NotificationService>>,
        leases: Arc<dyn WorkerRunRepository>,
    ) -> Self {
        Self {
            repo,
            service,
            leases,
        }
    }

    /// Take the lease on measuring the athlete's runs, asking again every
    /// [`LEASE_RETRY`] for up to `patience` while another batch holds it.
    /// `false` when it was still held at the end.
    async fn hold_athlete(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        patience: Duration,
    ) -> AppResult<bool> {
        let name = athlete_lease_name(user_id, tenant_id);
        let deadline = Instant::now() + patience;
        loop {
            let now_ms = Utc::now().timestamp_millis();
            if self
                .leases
                .claim_worker_run(&name, 0, now_ms, millis(ATHLETE_LEASE))
                .await?
            {
                return Ok(true);
            }
            if Instant::now() + LEASE_RETRY > deadline {
                return Ok(false);
            }
            sleep(LEASE_RETRY).await;
        }
    }

    /// Give the athlete's lease back. A release that fails is logged: the
    /// lease then runs out on its own.
    async fn release_athlete(&self, user_id: Uuid, tenant_id: TenantId) {
        let name = athlete_lease_name(user_id, tenant_id);
        if let Err(e) = self
            .leases
            .finish_worker_run(&name, Utc::now().timestamp_millis())
            .await
        {
            warn!(%user_id, error = %e, "athlete lease could not be released; it runs out on its own");
        }
    }

    /// Whether the walk of the athlete's history at `provider` reached their
    /// first activity. Until it did, a scan announces nothing.
    ///
    /// # Errors
    /// Returns a database error when the walk's progress cannot be read.
    pub async fn is_seed_complete(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        provider: &str,
    ) -> AppResult<bool> {
        Ok(self
            .repo
            .personal_best_seed(user_id, tenant_id, provider)
            .await?
            .is_some_and(|seed| seed.completed_at.is_some()))
    }

    /// The runs among `activities` that were never scanned, oldest first, so
    /// a best one of them sets is the baseline the next is measured against.
    ///
    /// # Errors
    /// Returns a database error when the scans cannot be read.
    pub async fn unscanned_runs<'a>(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        activities: &'a [Activity],
    ) -> AppResult<Vec<&'a Activity>> {
        let mut runs = Vec::new();
        for activity in activities
            .iter()
            .filter(|a| is_measured_sport(a.sport_type()))
        {
            if !self
                .repo
                .is_activity_scanned(user_id, tenant_id, activity.provider(), activity.id())
                .await?
            {
                runs.push(activity);
            }
        }
        runs.sort_by_key(|run| run.start_date());
        Ok(runs)
    }

    /// Measure the runs among `activities` that no earlier sync or walk
    /// measured, reading each one's samples from `provider`, and — once the
    /// walk of the athlete's history is complete — tell them about each
    /// all-time best one sets. Returns how many runs were measured.
    ///
    /// Each run costs one samples request, admitted against the signing app's
    /// budget as a request the athlete is waiting on; when it is spent, or the
    /// provider refuses two requests in a row, the runs left wait for the next
    /// sync, whose window lists them again.
    ///
    /// # Errors
    /// Returns a database error when the scans or bests cannot be read or
    /// written; a provider failure is logged and handled as described above.
    pub async fn scan_new_runs(
        &self,
        provider: &dyn FitnessProvider,
        user_id: Uuid,
        tenant_id: TenantId,
        activities: &[Activity],
    ) -> AppResult<usize> {
        if !self.hold_athlete(user_id, tenant_id, SCAN_PATIENCE).await? {
            info!(%user_id, "athlete's runs held by another batch past the wait; they wait for the next sync");
            return Ok(0);
        }
        let measured = self
            .scan_runs_held(provider, user_id, tenant_id, activities)
            .await;
        self.release_athlete(user_id, tenant_id).await;
        measured
    }

    /// [`Self::scan_new_runs`]'s work, run while the athlete's lock is held.
    async fn scan_runs_held(
        &self,
        provider: &dyn FitnessProvider,
        user_id: Uuid,
        tenant_id: TenantId,
        activities: &[Activity],
    ) -> AppResult<usize> {
        let announce = self
            .is_seed_complete(user_id, tenant_id, provider.name())
            .await?;
        let runs = self.unscanned_runs(user_id, tenant_id, activities).await?;
        let mut batch = Batch::new(self, provider, user_id, tenant_id, announce);
        for run in runs {
            if let Step::Stop(pause) = batch.measure(run).await? {
                info!(%user_id, ?pause, "run scan stopped; the runs left wait for the next sync");
                break;
            }
        }
        Ok(batch.measured)
    }

    /// Walk the athlete's history at `provider` from where the last call
    /// stopped, newest activity first, measuring every run no sync or earlier
    /// walk measured, and storing the bests they set without telling anything.
    ///
    /// One listing per page and one samples request per run, each admitted
    /// [`in_background`] against the background share of the signing app's
    /// budget. The walk stops when that share is spent or the provider
    /// refuses, saving the cursor it reached so the next call lists from
    /// there; it completes when a listing past the oldest activity comes back
    /// empty, and from then on records are told.
    ///
    /// # Errors
    /// Returns a database error when the walk's progress, the scans or the
    /// bests cannot be read or written.
    pub async fn advance_seed(
        &self,
        provider: &dyn FitnessProvider,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<SeedProgress> {
        if !self
            .hold_athlete(user_id, tenant_id, Duration::ZERO)
            .await?
        {
            return Ok(SeedProgress::Held);
        }
        let progress = in_background(self.walk_history_held(provider, user_id, tenant_id)).await;
        self.release_athlete(user_id, tenant_id).await;
        progress
    }

    /// [`Self::advance_seed`]'s work, run while the athlete's lock is held.
    async fn walk_history_held(
        &self,
        provider: &dyn FitnessProvider,
        user_id: Uuid,
        tenant_id: TenantId,
    ) -> AppResult<SeedProgress> {
        let provider_name = provider.name();
        let mut seed = self
            .repo
            .personal_best_seed(user_id, tenant_id, provider_name)
            .await?
            .unwrap_or_default();
        if seed.completed_at.is_some() {
            return Ok(SeedProgress::Complete);
        }

        let mut batch = Batch::new(self, provider, user_id, tenant_id, false);
        // `walk` bounds the next listing and follows every activity handled;
        // `seed.cursor_before` is what a restart lists from, so it stays put
        // while a failed run is held unresolved.
        let mut walk = seed.cursor_before;
        let progress = loop {
            match next_page(&mut batch, &mut seed, walk).await? {
                PageOutcome::Stop(pause) => break SeedProgress::Paused(pause),
                PageOutcome::End => {
                    seed.completed_at = Some(Utc::now());
                    break SeedProgress::Complete;
                }
                PageOutcome::Page(page) => {
                    if let Some(pause) = self
                        .walk_page(&mut batch, &page, &mut seed, &mut walk)
                        .await?
                    {
                        break SeedProgress::Paused(pause);
                    }
                    self.repo
                        .save_personal_best_seed(user_id, tenant_id, provider_name, &seed)
                        .await?;
                }
            }
        };
        self.repo
            .save_personal_best_seed(user_id, tenant_id, provider_name, &seed)
            .await?;
        info!(
            %user_id,
            provider = provider_name,
            measured = batch.measured,
            ?progress,
            cursor_before = ?seed.cursor_before,
            "history walk for best efforts advanced"
        );
        Ok(progress)
    }

    /// Handle one page of the walk, newest first: measure each run not yet
    /// scanned, and move the walk and — while no failed run is held — the
    /// saved cursor past each activity handled. Returns why the walk stopped
    /// inside the page, or `None` when the whole page was handled.
    async fn walk_page(
        &self,
        batch: &mut Batch<'_>,
        page: &[Activity],
        seed: &mut PersonalBestSeed,
        walk: &mut Option<i64>,
    ) -> AppResult<Option<SeedPause>> {
        for activity in page {
            if is_measured_sport(activity.sport_type())
                && !self
                    .repo
                    .is_activity_scanned(
                        batch.user_id,
                        batch.tenant_id,
                        activity.provider(),
                        activity.id(),
                    )
                    .await?
            {
                if let Step::Stop(pause) = batch.measure(activity).await? {
                    return Ok(Some(pause));
                }
            }
            let start = activity.start_date().timestamp();
            *walk = Some(start);
            if batch.suspect.is_none() {
                seed.cursor_before = Some(start);
            }
        }
        Ok(None)
    }

    /// Measure `run` on `series`, store each best it sets, record the run as
    /// scanned, and — when `announce` — tell the athlete about each stored
    /// best it beat.
    ///
    /// A run without a series (no samples, or a request that failed on the
    /// run itself) is still recorded, so it is never fetched again, and
    /// changes no best.
    ///
    /// # Errors
    /// Returns a database error when a best or the scan cannot be written.
    pub async fn record_run(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        run: &Activity,
        series: Option<&TimeSeriesData>,
        announce: bool,
    ) -> AppResult<Vec<DistanceResult>> {
        let results = match series {
            Some(series) => {
                let stored = self.repo.personal_bests(user_id, tenant_id).await?;
                measure(series, &stored)
            }
            None => Vec::new(),
        };
        for result in &results {
            if result.outcome == BestOutcome::Slower {
                continue;
            }
            let written = self
                .repo
                .record_personal_best(
                    user_id,
                    tenant_id,
                    &PersonalBest {
                        distance: result.distance.to_owned(),
                        elapsed_seconds: result.elapsed_seconds,
                        provider: run.provider().to_owned(),
                        activity_id: run.id().to_owned(),
                        achieved_at: run.start_date(),
                    },
                )
                .await?;
            if written && announce && matches!(result.outcome, BestOutcome::Improved { .. }) {
                self.announce(
                    user_id,
                    tenant_id,
                    run.id(),
                    result.distance,
                    result.elapsed_seconds,
                );
            }
        }
        self.repo
            .record_activity_scan(user_id, tenant_id, run.provider(), run.id(), Utc::now())
            .await?;
        Ok(results)
    }

    /// Send the one "new personal record" notice a beaten best owes.
    fn announce(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        activity_id: &str,
        distance: &str,
        elapsed_seconds: f64,
    ) {
        if let Some(service) = &self.service {
            trigger_personal_record(
                service,
                user_id,
                CommTenantId(tenant_id.as_uuid()),
                activity_id,
                distance,
                &format_effort_time(elapsed_seconds),
            );
        }
    }
}

/// What asking for the walk's next page came to.
enum PageOutcome {
    /// Activities older than the walk's position, newest first.
    Page(Vec<Activity>),
    /// Nothing older: the walk has reached the athlete's first activity.
    End,
    /// No page: the budget is spent or the provider refused.
    Stop(SeedPause),
}

/// Ask for the page of the athlete's history just before `walk` (the newest
/// page when `None`); a spent budget stops the walk.
///
/// The provider answering resolves a run held from the page before as the
/// run's own failure; the saved cursor then moves to `walk`, since every
/// activity from there on is handled.
async fn next_page(
    batch: &mut Batch<'_>,
    seed: &mut PersonalBestSeed,
    walk: Option<i64>,
) -> AppResult<PageOutcome> {
    let params = ActivityQueryParams {
        limit: Some(SEED_PAGE_SIZE),
        offset: None,
        before: walk,
        after: None,
    };
    let mut page = match batch.provider.get_activities_with_params(&params).await {
        Ok(page) => page,
        Err(e) if budget_spent(&e) => return Ok(PageOutcome::Stop(SeedPause::Budget)),
        Err(e) => {
            warn!(user_id = %batch.user_id, provider = batch.provider.name(), error = %e, "history walk listing failed; the walk resumes on a later pass");
            return Ok(PageOutcome::Stop(SeedPause::Refused));
        }
    };
    if batch.provider_answered().await? {
        seed.cursor_before = walk;
    }
    // Newest first, and nothing the walk already went past: a provider
    // honouring `before` returns exactly that, and one that does not cannot
    // hold the walk on a page it has handled.
    page.retain(|activity| walk.is_none_or(|bound| activity.start_date().timestamp() < bound));
    page.sort_by_key(|activity| Reverse(activity.start_date()));
    Ok(if page.is_empty() {
        PageOutcome::End
    } else {
        PageOutcome::Page(page)
    })
}

/// Whether `error` says a request budget is spent: the signing app's, which
/// refused the request before it was sent, or the provider's own, which
/// answered `429` once retries ran out. Neither is the run's fault.
fn budget_spent(error: &AppError) -> bool {
    error.code == ErrorCode::ExternalRateLimited
}

/// What measuring one run decided about the rest of the batch.
enum Step {
    /// Go on to the next run.
    Continue,
    /// Stop the batch here.
    Stop(SeedPause),
}

/// Runs measured one after another for one athlete, holding a run whose
/// samples request failed until the provider's answer to the next request
/// says whose failure it was.
struct Batch<'a> {
    bests: &'a PersonalBests,
    provider: &'a dyn FitnessProvider,
    user_id: Uuid,
    tenant_id: TenantId,
    /// Whether a beaten best is told: the walk of the history is complete.
    announce: bool,
    /// The run whose samples request failed, until the provider answers or
    /// refuses the next request.
    suspect: Option<Activity>,
    /// Runs measured, not counting a held run recorded unmeasured.
    measured: usize,
}

impl<'a> Batch<'a> {
    const fn new(
        bests: &'a PersonalBests,
        provider: &'a dyn FitnessProvider,
        user_id: Uuid,
        tenant_id: TenantId,
        announce: bool,
    ) -> Self {
        Self {
            bests,
            provider,
            user_id,
            tenant_id,
            announce,
            suspect: None,
            measured: 0,
        }
    }

    /// The provider answered a request, so a held run's failure was its own:
    /// record it unmeasured, so it is never asked for again. Returns whether
    /// a run was held.
    async fn provider_answered(&mut self) -> AppResult<bool> {
        let Some(suspect) = self.suspect.take() else {
            return Ok(false);
        };
        warn!(
            user_id = %self.user_id,
            activity_id = %suspect.id(),
            "run samples request failed while the provider answered the next request; the run is recorded unmeasured"
        );
        self.bests
            .record_run(self.user_id, self.tenant_id, &suspect, None, false)
            .await?;
        Ok(true)
    }

    /// Ask for `run`'s samples and record what they hold, or hold the run
    /// when the request fails; stop when the budget is spent, or when the
    /// provider refuses a second run in a row.
    async fn measure(&mut self, run: &Activity) -> AppResult<Step> {
        match self.provider.get_activity_streams(run.id()).await {
            Ok(series) => {
                self.provider_answered().await?;
                let results = self
                    .bests
                    .record_run(
                        self.user_id,
                        self.tenant_id,
                        run,
                        series.as_ref(),
                        self.announce,
                    )
                    .await?;
                self.measured += 1;
                info!(user_id = %self.user_id, activity_id = %run.id(), distances = results.len(), announce = self.announce, "run scanned for best efforts");
                Ok(Step::Continue)
            }
            // A spent budget is no fault of the run's: it is not suspect.
            Err(e) if budget_spent(&e) => Ok(Step::Stop(SeedPause::Budget)),
            Err(e) => {
                warn!(user_id = %self.user_id, activity_id = %run.id(), error = %e, "run samples request failed");
                if self.suspect.is_some() {
                    return Ok(Step::Stop(SeedPause::Refused));
                }
                self.suspect = Some(run.clone());
                Ok(Step::Continue)
            }
        }
    }
}

/// The best efforts `series` holds at the standard distances, each judged
/// against the athlete's `stored` best at that distance.
fn measure(series: &TimeSeriesData, stored: &[PersonalBest]) -> Vec<DistanceResult> {
    best_efforts(series, &STANDARD_RUNNING_DISTANCES_METERS)
        .into_iter()
        .filter_map(|effort| {
            let distance = distance_code(effort.distance_meters)?;
            let elapsed_seconds = whole_seconds(effort.elapsed_seconds) as f64;
            let previous = stored
                .iter()
                .find(|best| best.distance == distance)
                .map(|best| best.elapsed_seconds);
            let outcome = match previous {
                None => BestOutcome::Seeded,
                Some(previous_seconds) if elapsed_seconds < previous_seconds => {
                    BestOutcome::Improved { previous_seconds }
                }
                Some(_) => BestOutcome::Slower,
            };
            Some(DistanceResult {
                distance,
                elapsed_seconds,
                outcome,
            })
        })
        .collect()
}

/// The catalogue code of the standard distance `meters` is, if it is one.
fn distance_code(meters: f64) -> Option<&'static str> {
    STANDARD_RUNNING_DISTANCES_METERS
        .iter()
        .position(|standard| (standard - meters).abs() < f64::EPSILON)
        .and_then(|index| DISTANCE_CODES.get(index).copied())
}
