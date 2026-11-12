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
//!   walks the athlete's activities once, newest first, and stores the bests
//!   they set without telling anything. It lists nothing at the provider: it
//!   waits until a history walk (carnet#1482) copied the whole history into
//!   the activity store, then pages the store by `before` under every key the
//!   provider's history is held under ([`history_keys`]), so a Strava history
//!   walked through the usr/pwd session is read under `sciotte` as well as
//!   `strava`, each workout once. Streams are not in the store, so each past
//!   run still costs one samples request at the provider. The walk runs
//!   [`in_background`], so each of those requests is admitted against only
//!   the background share of the signing app's budget; it stops when that
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
//! request — the next run's samples or, when the failed run is the oldest the
//! store holds, one read of the athlete's profile, the only request left to
//! ask. When it answers, the failure was the run's own and the run is
//! recorded unmeasured, so it is never asked for again; when it refuses that
//! one too, the provider is refusing (a spent quota, an open circuit, a lost
//! grant) and the batch stops with the run unrecorded, so it is asked for
//! again later.
//!
//! A run is one workout whichever key holds it: the scan is keyed by the
//! service that recorded it ([`activity_origin`]), so the session's copy of a
//! Strava run and the grant's are scanned once, and the walk and the sync
//! scan read the same record.
//!
//! **Where the samples come from.** [`seed_stream_source`] picks, for each
//! provider, the backend of its history order ([`history_sources_for`]) whose
//! field dictionary ([`field_support`](pierre_providers::field_support))
//! declares the channels a best effort is read from: time and cumulative
//! distance ([`BEST_EFFORT_CHANNELS`]). A session declares only its route, so
//! a Strava history read through it is measured through the grant, by the
//! activity id both share.
//!
//! Only runs are measured, as cageux's own record detection does: `Run`,
//! `VirtualRun` and `TrailRunning`.

use pierre_core::ai_policy::first_party_only;
use pierre_core::transport::TransportPolicy;
use pierre_providers::backend_resolver::{activity_origin, history_keys};
use pierre_providers::field_support::StreamChannels;
use pierre_providers::registry::{global_registry, ProviderRegistry};
use pierre_providers::ProviderDescriptor;
use std::cmp::Reverse;
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};
use dravr_cageux::best_efforts::{best_efforts, STANDARD_RUNNING_DISTANCES_METERS};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{Activity, SportType, TenantId, TimeSeriesData};
use pierre_database::repositories::{
    ActivityBackfillJobRepository, ActivityCacheRepository, BackfillJobKind, BackfillJobState,
    PersonalBest, PersonalBestRepository, PersonalBestSeed, WorkerRunRepository,
};
use pierre_database::RepositoryRegistry;
use pierre_notifications::triggers::trigger_personal_record;
use pierre_notifications::{NotificationService, TenantId as CommTenantId};
use pierre_providers::core::FitnessProvider;
use pierre_providers::request_budget::in_background;
use tokio::time::sleep;
use tracing::{info, warn};
use uuid::Uuid;

use crate::notification_text::{
    PR_DISTANCE_10K, PR_DISTANCE_5K, PR_DISTANCE_HALF_MARATHON, PR_DISTANCE_MARATHON,
};
use crate::training_history_read::history_sources_for;

/// The catalogue code of each standard distance, index-aligned with
/// [`STANDARD_RUNNING_DISTANCES_METERS`]; the array length ties the two.
const DISTANCE_CODES: [&str; STANDARD_RUNNING_DISTANCES_METERS.len()] = [
    PR_DISTANCE_5K,
    PR_DISTANCE_10K,
    PR_DISTANCE_HALF_MARATHON,
    PR_DISTANCE_MARATHON,
];

/// The activities one page of the walk reads from the store, under each key.
///
/// The walk ends on an empty page rather than a short one.
pub const SEED_PAGE_SIZE: usize = 200;

/// The sample channels a best effort is read from: cageux's sweep reads the
/// cumulative distance against the time, and nothing else
/// ([`best_efforts`]).
pub const BEST_EFFORT_CHANNELS: StreamChannels =
    StreamChannels::TIME.union(StreamChannels::DISTANCE);

/// The backend whose samples the seed of `provider`'s history is measured
/// on: the first of [`history_sources_for`] `provider` whose descriptor
/// declares every one of [`BEST_EFFORT_CHANNELS`] in its field dictionary.
///
/// A backend that declares fewer is skipped: a usr/pwd session declares
/// only its route, so Strava's history is measured through the grant. `None`
/// when no backend of the provider serves them (COROS, TrainingPeaks,
/// Intervals.icu, which sends no distance channel): its history is never
/// seeded, rather than recorded scanned without a best.
#[must_use]
pub fn seed_stream_source(registry: &ProviderRegistry, provider: &str) -> Option<&'static str> {
    history_sources_for(provider)
        .into_iter()
        .filter_map(|(backend, _)| registry.get_descriptor(backend))
        .find(|descriptor| {
            descriptor
                .activity_fields()
                .streams
                .contains(BEST_EFFORT_CHANNELS)
        })
        .map(ProviderDescriptor::name)
}

/// Every backend [`seed_stream_source`] names for a provider `registry`
/// serves, each once, in the registry's order.
#[must_use]
pub fn seed_stream_sources(registry: &ProviderRegistry) -> Vec<&'static str> {
    let mut sources = Vec::new();
    for provider in registry.supported_providers() {
        if let Some(source) = seed_stream_source(registry, provider) {
            if !sources.contains(&source) {
                sources.push(source);
            }
        }
    }
    sources
}

/// The key a scan of `activity` is recorded under: the service that recorded
/// it, so the session's copy of a run and the grant's are one scan.
fn scan_key(activity: &Activity) -> &str {
    activity_origin(activity.provider(), activity.source())
}

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
    /// No history walk has copied the athlete's history into the store yet;
    /// the seed waits for it, with no request made and the cursor untouched.
    AwaitingHistory,
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
    /// The activity store the seed pages the athlete's history from.
    store: Arc<dyn ActivityCacheRepository>,
    /// The job ledger that says whether a history walk filled the store.
    walks: Arc<dyn ActivityBackfillJobRepository>,
}

impl fmt::Debug for PersonalBests {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PersonalBests")
            .field("service", &self.service)
            .finish_non_exhaustive()
    }
}

impl PersonalBests {
    /// Personal bests over `repos`, announced through `service`: the bests
    /// in its personal-best tables, each athlete's measuring leased in its
    /// worker ledger, and the seed read from its activity store once its job
    /// ledger says a history walk ended.
    #[must_use]
    pub fn new(repos: &RepositoryRegistry, service: Option<Arc<NotificationService>>) -> Self {
        Self {
            repo: Arc::clone(&repos.personal_bests),
            service,
            leases: Arc::clone(&repos.worker_runs),
            store: Arc::clone(&repos.activity_cache),
            walks: Arc::clone(&repos.activity_backfill_jobs),
        }
    }

    /// Take the lease on measuring the athlete's runs, asking again every
    /// [`LEASE_RETRY`] for up to `patience` while another batch holds it.
    /// The lease's token, or `None` when it was still held at the end.
    async fn hold_athlete(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        patience: Duration,
    ) -> AppResult<Option<String>> {
        let name = athlete_lease_name(user_id, tenant_id);
        let deadline = Instant::now() + patience;
        loop {
            let now_ms = Utc::now().timestamp_millis();
            if let Some(token) = self
                .leases
                .claim_worker_run(&name, 0, now_ms, millis(ATHLETE_LEASE))
                .await?
            {
                return Ok(Some(token));
            }
            if Instant::now() + LEASE_RETRY > deadline {
                return Ok(None);
            }
            sleep(LEASE_RETRY).await;
        }
    }

    /// Give the athlete's lease back. A release that fails is logged: the
    /// lease then runs out on its own.
    async fn release_athlete(&self, user_id: Uuid, tenant_id: TenantId, lease: &str) {
        let name = athlete_lease_name(user_id, tenant_id);
        if let Err(e) = self
            .leases
            .finish_worker_run(&name, Utc::now().timestamp_millis(), Some(lease))
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
                .is_activity_scanned(user_id, tenant_id, scan_key(activity), activity.id())
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
        let Some(lease) = self.hold_athlete(user_id, tenant_id, SCAN_PATIENCE).await? else {
            info!(%user_id, "athlete's runs held by another batch past the wait; they wait for the next sync");
            return Ok(0);
        };
        let measured = self
            .scan_runs_held(provider, user_id, tenant_id, activities)
            .await;
        self.release_athlete(user_id, tenant_id, &lease).await;
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

    /// Walk the athlete's history of `provider` from where the last call
    /// stopped, newest activity first, measuring every run no sync or earlier
    /// walk measured on `provider`'s samples, and storing the bests they set
    /// without telling anything.
    ///
    /// `provider` is the backend [`seed_stream_source`] names. Nothing is
    /// listed at it: until a history walk under one of its history keys
    /// ([`history_keys`]) ended, the seed waits
    /// ([`SeedProgress::AwaitingHistory`]); after, each page is read from the
    /// store under every one of those keys. One samples request per run, each
    /// admitted [`in_background`] against the background share of the signing
    /// app's budget. The walk stops when that share is spent or the provider
    /// refuses, saving the cursor it reached so the next call reads from
    /// there; it completes when the store holds nothing older, and from then
    /// on records are told.
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
        let Some(lease) = self
            .hold_athlete(user_id, tenant_id, Duration::ZERO)
            .await?
        else {
            return Ok(SeedProgress::Held);
        };
        let progress = in_background(self.walk_history_held(provider, user_id, tenant_id)).await;
        self.release_athlete(user_id, tenant_id, &lease).await;
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
        let keys = history_keys(provider_name);
        if !self.history_walked(user_id, tenant_id, &keys).await? {
            info!(%user_id, provider = provider_name, "best-effort seed waits for the history walk");
            return Ok(SeedProgress::AwaitingHistory);
        }

        let mut batch = Batch::new(self, provider, user_id, tenant_id, false);
        // `walk` bounds the next listing and follows every activity handled;
        // `seed.cursor_before` is what a restart lists from, so it stays put
        // while a failed run is held unresolved.
        let mut walk = seed.cursor_before;
        let progress = loop {
            match self.next_page(&mut batch, &keys, &mut seed, walk).await? {
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

    /// Whether a history walk under one of `keys` reached the end of the
    /// athlete's history: the store then holds all of it.
    async fn history_walked(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        keys: &[&str],
    ) -> AppResult<bool> {
        for key in keys {
            let state = self
                .walks
                .backfill_job_state(user_id, tenant_id, key, BackfillJobKind::HistoryWalk)
                .await?;
            if state == Some(BackfillJobState::Walked) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// The page of the athlete's history just before `walk` (the newest page
    /// when `None`), read from the store under every one of `keys`; when the
    /// store holds nothing older, the walk has reached the first activity.
    ///
    /// A run held from the page before is resolved here when the store is
    /// spent: no samples request is left to tell whose its failure was, so
    /// one read of the athlete's profile asks the provider instead. When it
    /// answers, the saved cursor moves to `walk`, since every activity from
    /// there on is handled.
    async fn next_page(
        &self,
        batch: &mut Batch<'_>,
        keys: &[&str],
        seed: &mut PersonalBestSeed,
        walk: Option<i64>,
    ) -> AppResult<PageOutcome> {
        let page = self
            .store_page(batch.user_id, batch.tenant_id, keys, walk)
            .await?;
        if !page.is_empty() {
            return Ok(PageOutcome::Page(page));
        }
        if batch.suspect.is_some() {
            if let Some(pause) = batch.probe().await? {
                return Ok(PageOutcome::Stop(pause));
            }
            seed.cursor_before = walk;
        }
        Ok(PageOutcome::End)
    }

    /// Up to [`SEED_PAGE_SIZE`] stored activities that started before `walk`
    /// (now when `None`), newest first, under any of `keys`.
    ///
    /// A workout stored under two keys (the session's copy and the grant's,
    /// under the one id the service gave it) is one activity, the first key's
    /// copy kept. Twins are matched by that id rather than merged into a
    /// session the way a read is: two recordings the athlete split a run into
    /// each carry their own samples, so each is measured.
    async fn store_page(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        keys: &[&str],
        walk: Option<i64>,
    ) -> AppResult<Vec<Activity>> {
        let end = walk
            .and_then(|ts| Utc.timestamp_opt(ts, 0).single())
            .unwrap_or_else(Utc::now);
        let limit = i64::try_from(SEED_PAGE_SIZE).unwrap_or(i64::MAX);
        let mut seen = HashSet::new();
        let mut page = Vec::new();
        for key in keys {
            let rows = self
                .store
                .get_cached_activities(
                    user_id,
                    &tenant_id,
                    Some(key),
                    DateTime::UNIX_EPOCH,
                    end,
                    limit,
                )
                .await?;
            for activity in rows {
                if seen.insert((scan_key(&activity).to_owned(), activity.id().to_owned())) {
                    page.push(activity);
                }
            }
        }
        // The store's bound is inclusive; the walk has handled `walk` itself.
        page.retain(|activity| walk.is_none_or(|bound| activity.start_date().timestamp() < bound));
        page.sort_by_key(|activity| Reverse(activity.start_date()));
        page.truncate(SEED_PAGE_SIZE);
        Ok(page)
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
                        scan_key(activity),
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
                    run,
                    result.distance,
                    result.elapsed_seconds,
                );
            }
        }
        self.repo
            .record_activity_scan(user_id, tenant_id, scan_key(run), run.id(), Utc::now())
            .await?;
        Ok(results)
    }

    /// Send the one "new personal record" notice a beaten best owes.
    fn announce(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        run: &Activity,
        distance: &str,
        elapsed_seconds: f64,
    ) {
        if let Some(service) = &self.service {
            // The record is the run's own data: it is served where the run's
            // terms let it be (carnet#769).
            let transport_policy = TransportPolicy::from_first_party_only(first_party_only(
                global_registry().as_ref(),
                run.provider(),
                run.source(),
            ));
            trigger_personal_record(
                service,
                user_id,
                CommTenantId(tenant_id.as_uuid()),
                run.id(),
                distance,
                &format_effort_time(elapsed_seconds),
                transport_policy,
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
    /// Nothing older, and the provider did not answer the request that was
    /// to resolve the run held from the last page: the budget is spent or
    /// the provider refused.
    Stop(SeedPause),
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

    /// Ask the provider for the athlete's profile, the one request left when
    /// a run is held and the store holds nothing older: an answer resolves
    /// the held run as its own failure. Returns why the walk stops when the
    /// provider does not answer.
    async fn probe(&mut self) -> AppResult<Option<SeedPause>> {
        match self.provider.get_athlete().await {
            Ok(_) => {
                self.provider_answered().await?;
                Ok(None)
            }
            Err(e) if budget_spent(&e) => Ok(Some(SeedPause::Budget)),
            Err(e) => {
                warn!(user_id = %self.user_id, provider = self.provider.name(), error = %e, "profile read failed while a run is held; the walk resumes on a later pass");
                Ok(Some(SeedPause::Refused))
            }
        }
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
