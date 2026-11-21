// ABOUTME: The per-athlete, per-backend windows every read of a signed-in sciotte session is counted in
// ABOUTME: Asked and background caps by tier, resolved through a bound SessionReadLimits, and the refusal they give

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The session-read caps (carnet#1661).
//!
//! Every read of an athlete's signed-in session at a scraped provider (the
//! four [`SESSION_READ_BACKENDS`]) is one request to that provider made as
//! the athlete, through their own account. Each is admitted against the
//! athlete's own windows at that backend, sized by their tier
//! ([`SessionReadCaps`]): a read the athlete is waiting on in the `asked`
//! windows, work nobody is waiting on ([`in_background`]) in the
//! `background` ones, each a 15-minute burst and a UTC day. The burst is
//! taken first, so a burst refusal takes nothing from the day.
//!
//! A Garmin or COROS athlete's first wellness copy runs inside
//! [`in_wellness_first_fill`]: its days are counted in a day of their own
//! ([`SessionReadCaps::wellness_first_fill_daily`]) in place of the
//! background day, and still in the background burst every other background
//! read shares.
//!
//! The caps are admin parameters (`sciotte_reads.<cap>.<tier>`), resolved for
//! each athlete by the [`SessionReadLimits`] the server binds
//! ([`ProviderRateLimiter::bind_session_read_limits`]) and kept for the TTL it
//! binds with, so `pierre-cli config set` reaches every instance within that
//! TTL, with no restart. An unbound limiter applies the Starter caps.
//!
//! These windows nest under the others a read is counted in, never instead of
//! them: a context read also keeps its own windows ([`super::context_read`]),
//! and a walk read its walk windows. Those are admitted before the read
//! reaches its session, so a session-read refusal after them leaves one
//! walk or context slot spent; the walk then pauses until the window ends,
//! so that is at most one slot per pause.
//!
//! A refusal is [`ErrorCode::ExternalRateLimited`], [`super::refused_before_send`],
//! with the wait until the refusing window ends, and names its kind under
//! `session_read_cap` ([`session_read_cap`]). It is logged once, on the
//! `usage` target as `usage.limit_reached`, naming the parameter that
//! refused: the one `pierre-cli config set` changes.

use std::fmt;
use std::future::Future;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_core::errors::AppError;
use pierre_core::models::{SessionReadCap, SessionReadCaps, UserTier};
use serde_json::Value;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::{
    budget_period, budget_refusal, in_background, is_background, Budget, ProviderRateLimiter,
    Window, FIFTEEN_MINUTES, ONE_DAY, PLATFORM_SCOPE,
};

/// The signed-in sessions whose every read is counted: the scraper's
/// Strava, Garmin, COROS and `TrainingPeaks` sessions.
pub(crate) const SESSION_READ_BACKENDS: [&str; 4] = [
    oauth_providers::SCIOTTE,
    oauth_providers::SCIOTTE_GARMIN,
    oauth_providers::SCIOTTE_COROS,
    oauth_providers::SCIOTTE_TRAININGPEAKS,
];

/// The `details` key a session-read refusal names its kind under.
const SESSION_READ_CAP_DETAIL: &str = "session_read_cap";

/// The refusal's detail saying a window it was counted in allows no read at
/// all: an admin set its cap to `0`.
const SESSION_READ_DISABLED_DETAIL: &str = "session_read_disabled";

/// The `tracing` target of the refusal's line, shared with every usage limit.
const USAGE_TARGET: &str = "usage";

/// The `event` of the refusal's line, shared with every usage limit.
const LIMIT_REACHED_EVENT: &str = "usage.limit_reached";

tokio::task_local! {
    /// Set while the running task makes an athlete's first wellness copy.
    static WELLNESS_FIRST_FILL: ();
}

/// Whose wait a session read is: the athlete's, or nobody's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionReadKind {
    /// A read the athlete is waiting on.
    Asked,
    /// Work nobody is waiting on ([`in_background`]).
    Background,
}

impl SessionReadKind {
    /// The kind as a refusal names it: `asked` or `background`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Asked => "asked",
            Self::Background => "background",
        }
    }

    /// The kind of the running task's reads.
    fn of_task() -> Self {
        if is_background() {
            Self::Background
        } else {
            Self::Asked
        }
    }
}

/// The caps an athlete's session reads are held to, and the tier they were
/// resolved for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AthleteReadCaps {
    /// The athlete's tier.
    pub tier: UserTier,
    /// Their caps, the tier's with any admin override applied.
    pub caps: SessionReadCaps,
}

impl AthleteReadCaps {
    /// The compiled caps of `tier`, with no override.
    #[must_use]
    pub const fn compiled(tier: UserTier) -> Self {
        Self {
            caps: SessionReadCaps::for_tier(&tier),
            tier,
        }
    }
}

/// Resolves an athlete's session-read caps: their tier, then each cap's
/// admin parameter for it. The server implements it over its admin config;
/// this crate cannot name that service.
#[async_trait]
pub trait SessionReadLimits: Send + Sync {
    /// The caps `user_id`'s session reads are held to. Never fails: a value
    /// that cannot be read is the compiled one.
    async fn caps(&self, user_id: Uuid) -> AthleteReadCaps;
}

/// The resolver a limiter was bound to, and what it resolved lately.
#[derive(Default)]
pub(super) struct SessionReadState {
    /// The bound resolver and how long one of its answers is kept.
    limits: OnceLock<(Arc<dyn SessionReadLimits>, Duration)>,
    /// Map of athlete -> the caps resolved for them, and when.
    resolved: DashMap<Uuid, (AthleteReadCaps, Instant)>,
}

impl fmt::Debug for SessionReadState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionReadState")
            .field("bound", &self.limits.get().is_some())
            .field("resolved", &self.resolved.len())
            .finish()
    }
}

/// Run `work` as an athlete's first wellness copy.
///
/// It is work nobody is waiting on ([`in_background`]), whose session reads
/// are counted in the first fill's own day
/// ([`SessionReadCaps::wellness_first_fill_daily`]) in place of the
/// background day, and still in the background burst.
///
/// The scope is a task-local: it holds only for `work` as it is polled.
pub async fn in_wellness_first_fill<F: Future>(work: F) -> F::Output {
    in_background(WELLNESS_FIRST_FILL.scope((), work)).await
}

/// Whether the running task is inside [`in_wellness_first_fill`].
fn in_first_fill() -> bool {
    WELLNESS_FIRST_FILL.try_with(|()| ()).is_ok()
}

/// `work`, run in the scope its caller is in.
///
/// Inside [`in_background`] when the caller is, so work handed to
/// `tokio::spawn` keeps counting as what it is. A task-local does not cross a
/// spawn on its own.
///
/// The scope is read where this is called, in the caller's task.
pub fn keep_scope<F: Future>(work: F) -> impl Future<Output = F::Output> {
    let background = is_background();
    async move {
        if background {
            in_background(work).await
        } else {
            work.await
        }
    }
}

/// The `counter_key` of `user_id`'s session reads at `backend` counted as
/// `counter`: `asked`, `background` or `wellness_first_fill`.
#[must_use]
pub fn session_read_counter_key(backend: &str, counter: &str, user_id: Uuid) -> String {
    format!("provider_session_reads:{backend}:{counter}:{user_id}")
}

/// The counter a cap is counted under, and its window.
const fn counted_in(cap: SessionReadCap) -> (&'static str, Duration) {
    match cap {
        SessionReadCap::AskedDaily => ("asked", ONE_DAY),
        SessionReadCap::Asked15Min => ("asked", FIFTEEN_MINUTES),
        SessionReadCap::BackgroundDaily => ("background", ONE_DAY),
        SessionReadCap::Background15Min => ("background", FIFTEEN_MINUTES),
        SessionReadCap::WellnessFirstFillDaily => ("wellness_first_fill", ONE_DAY),
    }
}

/// The caps a read of the running task is taken from, the burst first.
fn task_caps() -> [SessionReadCap; 2] {
    match (SessionReadKind::of_task(), in_first_fill()) {
        (SessionReadKind::Asked, _) => [SessionReadCap::Asked15Min, SessionReadCap::AskedDaily],
        (SessionReadKind::Background, false) => [
            SessionReadCap::Background15Min,
            SessionReadCap::BackgroundDaily,
        ],
        (SessionReadKind::Background, true) => [
            SessionReadCap::Background15Min,
            SessionReadCap::WellnessFirstFillDaily,
        ],
    }
}

/// Whether `error` is a session-read refusal, and of which kind.
///
/// Such a read was refused before anything reached the session, so a path
/// with another way to the same data (the athlete's own API grant, a stored
/// copy) may take it.
#[must_use]
pub fn session_read_cap(error: &AppError) -> Option<SessionReadKind> {
    match error.details.as_deref()?.get(SESSION_READ_CAP_DETAIL)? {
        Value::String(kind) if kind == SessionReadKind::Asked.as_str() => {
            Some(SessionReadKind::Asked)
        }
        Value::String(kind) if kind == SessionReadKind::Background.as_str() => {
            Some(SessionReadKind::Background)
        }
        _ => None,
    }
}

/// Whether `error` is a session-read refusal by a window that allows no read
/// at all (its cap is `0`).
///
/// Waiting for that window to end lets nothing through: the next window
/// allows none either, until an admin raises the cap. Work that would wait
/// it out fails instead, so its attempts end it.
#[must_use]
pub fn session_read_disabled(error: &AppError) -> bool {
    error
        .details
        .as_deref()
        .and_then(|details| details.get(SESSION_READ_DISABLED_DETAIL))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// The minutes until `retry_after_secs` has passed, rounded up, never `0`.
#[must_use]
pub const fn minutes_until(retry_after_secs: u64) -> u64 {
    let minutes = retry_after_secs.div_ceil(60);
    if minutes == 0 {
        1
    } else {
        minutes
    }
}

/// The message of a session-read refusal of `brand` (Strava, Garmin, …),
/// which reads resume after `retry_after_secs`.
fn refusal_message(brand: &str, retry_after_secs: u64) -> String {
    format!(
        "{brand} has been read as often as Dravr allows for now; live {brand} reads resume in \
         about {} min.",
        minutes_until(retry_after_secs)
    )
}

/// The refusal of a read of a session at `backend` by the running task,
/// retryable after `retry_after`; `disabled` when a window the read is
/// counted in allows none ([`session_read_disabled`]).
pub(super) fn refusal(backend: &str, retry_after: Duration, disabled: bool) -> AppError {
    let mut error = budget_refusal(backend, retry_after);
    error.message = refusal_message(backend, retry_after.as_secs());
    if let Some(Value::Object(details)) = error.details.as_deref_mut() {
        details.insert(
            SESSION_READ_CAP_DETAIL.to_owned(),
            Value::String(SessionReadKind::of_task().as_str().to_owned()),
        );
        if disabled {
            details.insert(SESSION_READ_DISABLED_DETAIL.to_owned(), Value::Bool(true));
        }
    }
    error
}

/// `error`, its message naming `brand` when it is a session-read refusal;
/// any other error is returned as it is.
#[must_use]
pub fn with_brand(mut error: AppError, brand: &str) -> AppError {
    if session_read_cap(&error).is_some() {
        error.message = refusal_message(brand, error.retry_after_secs().unwrap_or(0));
    }
    error
}

impl ProviderRateLimiter {
    /// Resolve each athlete's session-read caps through `limits`, keeping
    /// each answer for `ttl`. The first binding wins; a limiter never bound
    /// applies the Starter caps.
    pub fn bind_session_read_limits(&self, limits: Arc<dyn SessionReadLimits>, ttl: Duration) {
        if self.session_reads.limits.set((limits, ttl)).is_err() {
            debug!("session-read limits already bound; keeping the first");
        }
    }

    /// The caps `user_id`'s session reads are held to: as resolved within
    /// the bound TTL, else resolved now; the Starter caps when unbound.
    pub(super) async fn session_read_caps(&self, user_id: Uuid) -> AthleteReadCaps {
        let Some((limits, ttl)) = self.session_reads.limits.get() else {
            return AthleteReadCaps::compiled(UserTier::Starter);
        };
        // Cloned out, so no map guard is held across the resolver's reads.
        let cached = self
            .session_reads
            .resolved
            .get(&user_id)
            .map(|entry| entry.clone());
        if let Some((caps, _)) = cached.filter(|(_, at)| at.elapsed() < *ttl) {
            return caps;
        }
        let caps = limits.caps(user_id).await;
        self.session_reads
            .resolved
            .insert(user_id, (caps.clone(), Instant::now()));
        caps
    }

    /// Whether a window the running task's reads of `user_id`'s sessions are
    /// counted in allows none: an admin set its cap to `0`.
    pub(super) async fn session_reads_disabled(&self, user_id: Uuid) -> bool {
        let caps = self.session_read_caps(user_id).await.caps;
        task_caps().into_iter().any(|cap| cap.of(&caps) == 0)
    }

    /// The windows one read of `user_id`'s session at `backend` by the
    /// running task is taken from ([`task_caps`]), each taken whole.
    pub(super) async fn session_read_windows(&self, backend: &str, user_id: Uuid) -> Vec<Window> {
        let caps = self.session_read_caps(user_id).await.caps;
        task_caps()
            .into_iter()
            .map(|cap| {
                let (counter, window) = counted_in(cap);
                Window {
                    key: session_read_counter_key(backend, counter, user_id),
                    budget: Budget {
                        max_calls: cap.of(&caps),
                        window,
                    },
                    share_percent: 100,
                }
            })
            .collect()
    }

    /// Log the refusal of a read of `user_id`'s session at `backend`, by the
    /// window at `refusing` among `windows` ([`Self::session_read_windows`]),
    /// once, on the `usage` target: a warning for a read the athlete waits
    /// on, information for background work.
    pub(super) async fn log_session_read_refusal(
        &self,
        backend: &str,
        user_id: Uuid,
        windows: &[Window],
        refusing: usize,
        retry_after: Duration,
        now: DateTime<Utc>,
    ) {
        let (Some(window), Some(cap)) = (windows.get(refusing), task_caps().get(refusing).copied())
        else {
            return;
        };
        let limit = window.budget.allowance(window.share_percent);
        let current = self
            .counters
            .get_counter(
                PLATFORM_SCOPE,
                PLATFORM_SCOPE,
                &window.key,
                &budget_period(now, window.budget.window),
            )
            .await
            .map_or(limit, |counter| counter.value);
        let tier = self.session_read_caps(user_id).await.tier;
        let limit_type = cap.config_key(&tier);
        let kind = SessionReadKind::of_task().as_str();
        let window = if window.budget.window == ONE_DAY {
            "1d"
        } else {
            "15m"
        };
        let user_id = user_id.to_string();
        let retry_after_secs = retry_after.as_secs();
        match SessionReadKind::of_task() {
            SessionReadKind::Asked => warn!(
                target: USAGE_TARGET,
                event = LIMIT_REACHED_EVENT,
                user_id,
                limit_type,
                current,
                limit,
                surface = "provider_read",
                provider = backend,
                kind,
                window,
                tier = tier.as_str(),
                retry_after_secs,
                "usage limit reached"
            ),
            SessionReadKind::Background => info!(
                target: USAGE_TARGET,
                event = LIMIT_REACHED_EVENT,
                user_id,
                limit_type,
                current,
                limit,
                surface = "provider_read",
                provider = backend,
                kind,
                window,
                tier = tier.as_str(),
                retry_after_secs,
                "usage limit reached"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io;
    use std::sync::{Mutex, PoisonError};

    use pierre_core::errors::ErrorCode;
    use pierre_database::RepositoryRegistry;
    use pierre_test_support::db::create_test_db;
    use tracing::subscriber::{self, NoSubscriber};
    use tracing::Dispatch;
    use tracing_subscriber::fmt;

    use super::super::{
        detail_walk_athlete_counter_key, detail_walk_counter_key, in_detail_walk,
        refused_before_send, RequestBudget,
    };
    use super::*;

    const STRAVA_SESSION: &str = oauth_providers::SCIOTTE;
    const GARMIN_SESSION: &str = oauth_providers::SCIOTTE_GARMIN;

    async fn limiter() -> (Arc<RepositoryRegistry>, Arc<ProviderRateLimiter>) {
        let db = create_test_db()
            .await
            .unwrap_or_else(|e| panic!("test db: {e}"));
        let repos = Arc::clone(db.repositories());
        let limiter = Arc::new(ProviderRateLimiter::new(
            Arc::clone(&repos.usage_counters),
            Arc::clone(&repos.provider_connections),
        ));
        (repos, limiter)
    }

    /// What `key` holds in its `window` bucket now; `0` when nothing was
    /// counted.
    async fn spent(repos: &RepositoryRegistry, key: &str, window: Duration) -> i64 {
        repos
            .usage_counters
            .get_counter(
                PLATFORM_SCOPE,
                PLATFORM_SCOPE,
                key,
                &budget_period(Utc::now(), window),
            )
            .await
            .map_or(0, |counter| counter.value)
    }

    /// Add `value` to the `window` bucket under `key`.
    async fn spend(repos: &RepositoryRegistry, key: &str, window: Duration, value: u32) {
        repos
            .usage_counters
            .increment_counter(
                PLATFORM_SCOPE,
                PLATFORM_SCOPE,
                key,
                &budget_period(Utc::now(), window),
                i64::from(value),
            )
            .await
            .unwrap_or_else(|e| panic!("bug: the counter could not be spent: {e}"));
    }

    fn read_budget(
        limiter: &Arc<ProviderRateLimiter>,
        backend: &str,
        user_id: Uuid,
    ) -> RequestBudget {
        RequestBudget::for_session_read(limiter, backend, user_id)
            .unwrap_or_else(|| panic!("bug: {backend} sessions are counted"))
    }

    fn starter() -> SessionReadCaps {
        SessionReadCaps::for_tier(&UserTier::Starter)
    }

    /// A resolver whose answer the test changes.
    struct SetCaps(Mutex<AthleteReadCaps>);

    #[async_trait]
    impl SessionReadLimits for SetCaps {
        async fn caps(&self, _user_id: Uuid) -> AthleteReadCaps {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone()
        }
    }

    /// Keep two inert dispatchers registered for the rest of the test binary.
    ///
    /// While at most one dispatcher is registered, `tracing-core` computes a
    /// callsite's interest from the dispatcher of whichever thread registers
    /// it, without taking its lock. A sibling test reaching the usage log site
    /// first, on a thread with no subscriber, then caches it as "never" for
    /// every thread, and the line this test holds a subscriber for is dropped.
    /// The count is taken when a dispatcher registers, so it takes two to turn
    /// that path off before the test's setup runs; from then on every
    /// registration reads the registered dispatchers under the lock that
    /// installing the test's subscriber also takes.
    fn register_two_inert_dispatchers() {
        static INERT: OnceLock<[Dispatch; 2]> = OnceLock::new();
        INERT.get_or_init(|| {
            [
                Dispatch::new(NoSubscriber::default()),
                Dispatch::new(NoSubscriber::default()),
            ]
        });
    }

    /// Log lines written while a test holds the subscriber.
    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);

    impl io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl Captured {
        fn lines(&self) -> Vec<String> {
            let bytes = self
                .0
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            String::from_utf8_lossy(&bytes)
                .lines()
                .map(str::to_owned)
                .collect()
        }
    }

    #[tokio::test]
    async fn only_the_four_scraper_sessions_are_counted_each_on_its_own_key() {
        let (repos, limiter) = limiter().await;
        let athlete = Uuid::new_v4();
        for backend in SESSION_READ_BACKENDS {
            read_budget(&limiter, backend, athlete)
                .admit(backend)
                .await
                .unwrap_or_else(|e| panic!("bug: the first {backend} read is refused: {e}"));
        }
        for backend in SESSION_READ_BACKENDS {
            let key = session_read_counter_key(backend, "asked", athlete);
            assert_eq!(
                spent(&repos, &key, ONE_DAY).await,
                1,
                "bug: {backend}'s reads are not counted on their own key"
            );
        }
        for other in ["strava", "garmin", "intervals_icu"] {
            assert!(
                RequestBudget::for_session_read(&limiter, other, athlete).is_none(),
                "bug: {other} reads as a scraper session"
            );
        }
    }

    #[tokio::test]
    async fn an_asked_burst_refuses_the_31st_read_and_spares_background_and_the_day() {
        let (repos, limiter) = limiter().await;
        let athlete = Uuid::new_v4();
        let budget = read_budget(&limiter, STRAVA_SESSION, athlete);
        for read in 0..starter().asked_15min {
            assert!(
                budget.admit(STRAVA_SESSION).await.is_ok(),
                "bug: asked read {read} is within the burst"
            );
        }
        let refused = budget
            .admit(STRAVA_SESSION)
            .await
            .expect_err("the read past the asked burst is refused");
        assert_eq!(
            refused.code,
            ErrorCode::ExternalRateLimited,
            "bug: a spent burst refuses as a rate limit"
        );
        let wait = refused.retry_after_secs().unwrap_or(0);
        assert!(
            (1..=FIFTEEN_MINUTES.as_secs()).contains(&wait),
            "bug: the burst's wait {wait}s is not within its 15 minutes"
        );
        let asked = session_read_counter_key(STRAVA_SESSION, "asked", athlete);
        assert_eq!(
            spent(&repos, &asked, ONE_DAY).await,
            i64::from(starter().asked_15min),
            "bug: the burst's refusal took a slot of the day"
        );
        assert!(
            in_background(budget.admit(STRAVA_SESSION)).await.is_ok(),
            "bug: a spent asked burst held a background read"
        );
        let background = session_read_counter_key(STRAVA_SESSION, "background", athlete);
        assert_eq!(
            spent(&repos, &background, FIFTEEN_MINUTES).await,
            1,
            "bug: the background read is not counted under the background key"
        );
        assert_eq!(
            spent(&repos, &asked, FIFTEEN_MINUTES).await,
            i64::from(starter().asked_15min),
            "bug: a background read was counted under the asked key"
        );
    }

    #[tokio::test]
    async fn a_spent_day_refuses_until_the_utc_day_ends() {
        let (repos, limiter) = limiter().await;
        let athlete = Uuid::new_v4();
        let asked = session_read_counter_key(GARMIN_SESSION, "asked", athlete);
        // Unbound: the Starter day. One read left, then none.
        spend(&repos, &asked, ONE_DAY, starter().asked_daily - 1).await;
        let budget = read_budget(&limiter, GARMIN_SESSION, athlete);
        assert!(
            budget.admit(GARMIN_SESSION).await.is_ok(),
            "bug: the last read of the Starter day is refused"
        );
        let before = Utc::now();
        let refused = budget
            .admit(GARMIN_SESSION)
            .await
            .expect_err("the read past the Starter day is refused");
        let wait = refused.retry_after_secs().unwrap_or(0);
        let day_end = before.date_naive().succ_opt().map_or(before, |day| {
            day.and_hms_opt(0, 0, 0)
                .map_or(before, |midnight| midnight.and_utc())
        });
        let until_midnight = u64::try_from((day_end - before).num_seconds()).unwrap_or(0);
        assert!(
            wait.abs_diff(until_midnight) <= 2,
            "bug: a day refusal waits {wait}s, not the {until_midnight}s to UTC midnight"
        );
        assert_eq!(
            spent(&repos, &asked, FIFTEEN_MINUTES).await,
            1,
            "bug: the burst slot taken before the day refused was not given back"
        );
    }

    #[tokio::test]
    async fn a_read_inside_a_detail_walk_gets_session_read_windows_only() {
        let (repos, limiter) = limiter().await;
        let athlete = Uuid::new_v4();
        in_detail_walk(
            athlete,
            read_budget(&limiter, STRAVA_SESSION, athlete).admit(STRAVA_SESSION),
        )
        .await
        .unwrap_or_else(|e| panic!("bug: a walk's session read is refused: {e}"));
        let background = session_read_counter_key(STRAVA_SESSION, "background", athlete);
        assert_eq!(
            spent(&repos, &background, ONE_DAY).await,
            1,
            "bug: a walk's read is not one background session read"
        );
        for walk in [
            detail_walk_counter_key(&background),
            detail_walk_athlete_counter_key(&background, athlete),
        ] {
            assert_eq!(
                spent(&repos, &walk, ONE_DAY).await,
                0,
                "bug: a session read took a walk window"
            );
        }
    }

    #[tokio::test]
    async fn the_first_wellness_fill_has_its_own_day_inside_the_background_burst() {
        let (repos, limiter) = limiter().await;
        let athlete = Uuid::new_v4();
        let budget = read_budget(&limiter, GARMIN_SESSION, athlete);
        let background = session_read_counter_key(GARMIN_SESSION, "background", athlete);
        let first_fill = session_read_counter_key(GARMIN_SESSION, "wellness_first_fill", athlete);
        // The background day is spent: an ordinary background read is
        // refused, a first-fill read is not.
        spend(&repos, &background, ONE_DAY, starter().background_daily).await;
        assert!(
            in_background(budget.admit(GARMIN_SESSION)).await.is_err(),
            "bug: a spent background day admitted an ordinary background read"
        );
        in_wellness_first_fill(budget.admit(GARMIN_SESSION))
            .await
            .unwrap_or_else(|e| panic!("bug: the first fill was held by the background day: {e}"));
        assert_eq!(
            spent(&repos, &first_fill, ONE_DAY).await,
            1,
            "bug: a first-fill read is not counted in the first fill's day"
        );
        assert_eq!(
            spent(&repos, &background, ONE_DAY).await,
            i64::from(starter().background_daily),
            "bug: a first-fill read was counted in the background day"
        );
        assert_eq!(
            spent(&repos, &background, FIFTEEN_MINUTES).await,
            1,
            "bug: a first-fill read is not counted in the background burst"
        );
        // The burst is shared: the first fill spends it for every background
        // read, and an ordinary background read never touches the first
        // fill's day.
        spend(
            &repos,
            &background,
            FIFTEEN_MINUTES,
            starter().background_15min - 1,
        )
        .await;
        assert!(
            in_wellness_first_fill(budget.admit(GARMIN_SESSION))
                .await
                .is_err(),
            "bug: a spent background burst admitted a first-fill read"
        );
        assert_eq!(
            spent(&repos, &first_fill, ONE_DAY).await,
            1,
            "bug: a refused first-fill read kept a slot of its day"
        );
    }

    #[tokio::test]
    async fn a_bound_resolver_with_no_ttl_applies_a_change_on_the_next_read() {
        let (repos, limiter) = limiter().await;
        let athlete = Uuid::new_v4();
        let resolver = Arc::new(SetCaps(Mutex::new(AthleteReadCaps::compiled(
            UserTier::Professional,
        ))));
        limiter.bind_session_read_limits(Arc::<SetCaps>::clone(&resolver), Duration::ZERO);
        let budget = read_budget(&limiter, STRAVA_SESSION, athlete);
        let asked = session_read_counter_key(STRAVA_SESSION, "asked", athlete);
        // Past the Starter day, inside the Professional one.
        spend(&repos, &asked, ONE_DAY, starter().asked_daily).await;
        assert!(
            budget.admit(STRAVA_SESSION).await.is_ok(),
            "bug: the bound Professional day was not applied"
        );
        // A cap of 0 is the kill switch, applied on the very next read.
        if let Ok(mut caps) = resolver.0.lock() {
            caps.caps.asked_daily = 0;
        }
        let refused = budget
            .admit(STRAVA_SESSION)
            .await
            .expect_err("a cap of 0 refuses every read");
        assert_eq!(
            session_read_cap(&refused),
            Some(SessionReadKind::Asked),
            "bug: the kill switch's refusal does not name its kind"
        );
        assert!(
            session_read_disabled(&refused),
            "bug: a cap of 0 refuses without saying waiting lets nothing through"
        );
        // A spent window that allows reads is waited out, never disabled.
        if let Ok(mut caps) = resolver.0.lock() {
            caps.caps.asked_daily = starter().asked_daily;
        }
        let spent_day = budget
            .admit(STRAVA_SESSION)
            .await
            .expect_err("the Starter day is spent");
        assert!(
            !session_read_disabled(&spent_day),
            "bug: a spent window that allows reads reads as disabled"
        );
    }

    #[tokio::test]
    async fn a_refusal_is_marked_and_logged_once_on_the_usage_target() {
        register_two_inert_dispatchers();
        let (repos, limiter) = limiter().await;
        let athlete = Uuid::new_v4();
        let asked = session_read_counter_key(STRAVA_SESSION, "asked", athlete);
        spend(&repos, &asked, FIFTEEN_MINUTES, starter().asked_15min).await;
        let captured = Captured::default();
        let writer = captured.clone();
        let subscriber = fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let capturing = subscriber::set_default(subscriber);
        let refused = read_budget(&limiter, STRAVA_SESSION, athlete)
            .admit(STRAVA_SESSION)
            .await
            .expect_err("a spent burst refuses the read");
        drop(capturing);
        assert!(
            refused_before_send(&refused),
            "bug: a session-read refusal is not marked refused before send"
        );
        assert_eq!(
            session_read_cap(&refused),
            Some(SessionReadKind::Asked),
            "bug: an asked refusal does not name its kind"
        );
        let lines = captured.lines();
        let usage: Vec<&String> = lines
            .iter()
            .filter(|line| line.contains("usage limit reached"))
            .collect();
        assert_eq!(
            usage.len(),
            1,
            "bug: one refusal, one usage line: {lines:?}"
        );
        for field in [
            "usage.limit_reached",
            "sciotte_reads.asked_15min.starter",
            "provider_read",
            &athlete.to_string(),
            "window=\"15m\"",
            "kind=\"asked\"",
            "tier=\"starter\"",
            "current=30",
            "limit=30",
        ] {
            assert!(
                usage[0].contains(field),
                "bug: the usage line lacks {field}: {}",
                usage[0]
            );
        }
        assert!(
            usage[0].contains("WARN"),
            "bug: an asked refusal is not a warning: {}",
            usage[0]
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("Provider request budget spent")),
            "bug: a session-read refusal paged as an app's budget: {lines:?}"
        );
    }

    #[tokio::test]
    async fn the_brand_replaces_the_backend_in_a_session_read_refusal_only() {
        let (repos, limiter) = limiter().await;
        let athlete = Uuid::new_v4();
        let asked = session_read_counter_key(GARMIN_SESSION, "asked", athlete);
        spend(&repos, &asked, FIFTEEN_MINUTES, starter().asked_15min).await;
        let refused = read_budget(&limiter, GARMIN_SESSION, athlete)
            .admit(GARMIN_SESSION)
            .await
            .expect_err("a spent burst refuses the read");
        let branded = with_brand(refused, "Garmin");
        assert!(
            branded
                .message
                .starts_with("Garmin has been read as often as Dravr allows for now; live Garmin reads resume in about "),
            "bug: the refusal does not name the brand: {}",
            branded.message
        );
        let other = AppError::new(ErrorCode::ExternalServiceError, "the scraper is down");
        assert_eq!(
            with_brand(other, "Garmin").message,
            "the scraper is down",
            "bug: another error's message was rewritten"
        );
    }

    #[tokio::test]
    async fn keep_scope_carries_background_across_a_spawn() {
        let carried =
            in_background(async { tokio::spawn(keep_scope(async { is_background() })).await })
                .await
                .unwrap_or_else(|e| panic!("bug: the spawned task failed: {e}"));
        assert!(carried, "bug: a spawn inside the background lost its scope");
        let asked = tokio::spawn(keep_scope(async { is_background() }))
            .await
            .unwrap_or_else(|e| panic!("bug: the spawned task failed: {e}"));
        assert!(
            !asked,
            "bug: a spawn outside the background gained the scope"
        );
    }
}
