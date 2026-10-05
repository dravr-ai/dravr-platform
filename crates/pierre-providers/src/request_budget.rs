// ABOUTME: Provider request budgets per OAuth app, counted in the database so every instance shares them
// ABOUTME: Every provider call is admitted against the budget of the app that signs it, before it is sent

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider request budgets, one per OAuth application.
//!
//! A provider limits the requests an application makes, whichever tenant or
//! athlete they are for: Strava counts every call against one 15-minute
//! window, reset at :00, :15, :30 and :45, and one daily window, reset at
//! midnight UTC, per app. A request has to fit every budget its provider has,
//! counted for the app whose client signed the token it carries: the server's
//! app, a Strava shared-pool app, a tenant's own app or an athlete's own app
//! each have their own windows. A tenant's own app carries the daily budget
//! its operator registered (`rate_limit_per_day`) in place of the provider's.
//!
//! Every provider request is admitted before it is sent: a credential carries
//! its [`RequestBudget`], and the provider asks it once per request, retries
//! included. A refusal is [`ErrorCode::ExternalRateLimited`] with the wait
//! until the spent window resets, and nothing is sent.
//!
//! The backend runs as several instances (Cloud Run scales it from zero to
//! three), so a count held in one process lets each instance spend the whole
//! budget. The counts live in the database instead, in `usage_counters` — the
//! table every quota counts in — one bucket per window under the platform
//! scope ([`PLATFORM_SCOPE`], a provider's budget belongs to no tenant or
//! user). The buckets are fixed and aligned to the epoch, as Strava's own
//! windows are, and each bucket's period label starts with its start time,
//! so the usage-counter pruning ages them out with the rest.
//!
//! A request is taken with one conditional increment per window, which lands
//! only while the bucket holds fewer than its allowance: instances racing for
//! the last slot get it once between them, and no window ever passes its
//! budget. When a later window refuses, the slots already taken for the
//! request are given back.
//!
//! Work nobody is waiting on — the walk of an athlete's history — runs inside
//! [`in_background`], whose requests stop at [`BACKGROUND_SHARE_PERCENT`] of
//! each window, so the rest stays for the requests an athlete is waiting on.

use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use pierre_core::constants::rate_limits;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_database::repositories::UsageCounterRepository;
#[cfg(any(
    feature = "provider-coros",
    feature = "provider-garmin",
    feature = "provider-intervals-icu",
    feature = "provider-strava",
    feature = "provider-terra",
    feature = "provider-whoop",
))]
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

#[cfg(any(
    feature = "provider-coros",
    feature = "provider-garmin",
    feature = "provider-intervals-icu",
    feature = "provider-strava",
    feature = "provider-terra",
    feature = "provider-whoop",
))]
use crate::core::OAuth2Credentials;

/// One day as a `Duration`.
pub const ONE_DAY: Duration = Duration::from_hours(24);

/// Strava's short window.
pub const FIFTEEN_MINUTES: Duration = Duration::from_mins(15);

/// The share of every window, in percent, that work nobody is waiting on may
/// spend: the walk of an athlete's whole Strava history for their personal
/// bests.
///
/// A quarter. Against Strava's standard read limits of 100 requests per 15
/// minutes and 1,000 per day, the walk gets 25 requests a window and 250 a
/// day, one streams request per past run, so a 1,000-run history is seeded in
/// about four days, while the requests an athlete is waiting on keep three
/// quarters of every window.
pub const BACKGROUND_SHARE_PERCENT: u32 = 25;

/// The `tenant_id` and `user_id` a provider's budget buckets are counted
/// under: the budget is the application's, owed to no tenant or user.
pub const PLATFORM_SCOPE: &str = "platform";

/// The `counter_key` of the budget buckets of `app` at `provider`: one set of
/// windows per OAuth client, since a provider counts each app on its own.
#[must_use]
pub fn budget_counter_key(provider: &str, app: &str) -> String {
    format!("provider_requests:{provider}:{app}")
}

tokio::task_local! {
    /// Set while the running task does work nobody is waiting on.
    static BACKGROUND: ();
}

/// Run `work` as work nobody is waiting on: each provider request it makes is
/// admitted only while every window holds fewer requests than
/// [`BACKGROUND_SHARE_PERCENT`] of its budget.
pub async fn in_background<F: Future>(work: F) -> F::Output {
    BACKGROUND.scope((), work).await
}

/// Whether the running task is inside [`in_background`].
fn is_background() -> bool {
    BACKGROUND.try_with(|()| ()).is_ok()
}

/// The period label of the `window` bucket that holds `now`.
///
/// The bucket's start, to the millisecond, then the window's length in
/// milliseconds, so two windows starting at the same instant keep separate
/// buckets and the label sorts by date for the pruning.
#[must_use]
pub fn budget_period(now: DateTime<Utc>, window: Duration) -> String {
    let (start_ms, _) = bucket_bounds(now, window);
    let start = DateTime::from_timestamp_millis(start_ms).unwrap_or(now);
    format!(
        "{}/{}",
        start.format("%Y-%m-%dT%H:%M:%S%.3fZ"),
        window_ms(window)
    )
}

/// A window's length in milliseconds, never below one.
fn window_ms(window: Duration) -> i64 {
    i64::try_from(window.as_millis()).unwrap_or(i64::MAX).max(1)
}

/// The start and the end, in unix milliseconds, of the epoch-aligned
/// `window` bucket that holds `now`.
fn bucket_bounds(now: DateTime<Utc>, window: Duration) -> (i64, i64) {
    let length = window_ms(window);
    let now_ms = now.timestamp_millis();
    let start = now_ms - now_ms.rem_euclid(length);
    (start, start.saturating_add(length))
}

/// One budget: at most `max_calls` requests in each `window`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Budget {
    max_calls: u32,
    window: Duration,
}

impl Budget {
    /// The requests a caller asking for `share_percent` of the window may
    /// have counted in it.
    fn allowance(self, share_percent: u32) -> i64 {
        i64::from(self.max_calls) * i64::from(share_percent) / 100
    }
}

/// Holds each provider's budgets and takes requests from them in the
/// database, per OAuth app, so every instance counts into the same windows.
pub struct ProviderRateLimiter {
    /// Where the windows are counted: `usage_counters`, shared by every
    /// instance of the backend.
    counters: Arc<dyn UsageCounterRepository>,
    /// Map of `provider_name` -> every budget a request to it must fit.
    /// Configuration only, the same on every instance; read and cloned out
    /// before any database call.
    budgets: DashMap<String, Vec<Budget>>,
}

/// Result of a rate limit check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RateLimitStatus {
    /// The request was taken from every window of the provider's budget.
    Allowed,
    /// A window is spent; retry after the given duration.
    Exceeded {
        /// Duration until the spent window resets.
        retry_after: Duration,
    },
}

impl ProviderRateLimiter {
    /// A rate limiter counting in `counters`, pre-loaded with known provider
    /// rate limits.
    #[must_use]
    pub fn new(counters: Arc<dyn UsageCounterRepository>) -> Self {
        let daily = |max_calls| {
            vec![Budget {
                max_calls,
                window: ONE_DAY,
            }]
        };
        let budgets = DashMap::new();
        // Strava: the standard read limits, per app.
        budgets.insert(
            "strava".to_owned(),
            vec![
                Budget {
                    max_calls: rate_limits::STRAVA_RATE_LIMIT_15MIN,
                    window: FIFTEEN_MINUTES,
                },
                Budget {
                    max_calls: rate_limits::STRAVA_RATE_LIMIT_DAILY,
                    window: ONE_DAY,
                },
            ],
        );
        budgets.insert(
            "garmin".to_owned(),
            daily(rate_limits::GARMIN_DEFAULT_DAILY_RATE_LIMIT),
        );
        budgets.insert(
            "whoop".to_owned(),
            daily(rate_limits::WHOOP_DEFAULT_DAILY_RATE_LIMIT),
        );
        budgets.insert(
            "terra".to_owned(),
            daily(rate_limits::TERRA_DEFAULT_DAILY_RATE_LIMIT),
        );

        info!(
            provider_count = budgets.len(),
            "Provider rate limiter initialized"
        );

        Self { counters, budgets }
    }

    /// Take one request to `provider`, signed by `app`, from that app's
    /// windows: the whole of each window for a request an athlete is waiting
    /// on, the background share inside [`in_background`]. `daily_limit`, when
    /// set, is the app's own daily budget in place of the provider's.
    ///
    /// # Errors
    /// Returns a database error when a window cannot be counted.
    pub async fn acquire(
        &self,
        provider: &str,
        app: &str,
        daily_limit: Option<u32>,
    ) -> AppResult<RateLimitStatus> {
        let background = is_background();
        let share = if background {
            BACKGROUND_SHARE_PERCENT
        } else {
            100
        };
        let status = self
            .acquire_share(provider, app, daily_limit, share)
            .await?;
        if let RateLimitStatus::Exceeded { retry_after } = &status {
            if background {
                debug!(
                    provider,
                    app,
                    retry_after_secs = retry_after.as_secs(),
                    "Background share of the app's provider budget spent"
                );
            } else {
                warn!(
                    provider,
                    app,
                    retry_after_secs = retry_after.as_secs(),
                    "Provider request budget of the app spent"
                );
            }
        }
        Ok(status)
    }

    /// The windows a request to `provider` must fit: the provider's, with the
    /// daily window's allowance replaced by `daily_limit` when the app has
    /// its own (added when the provider has none).
    fn budgets_for(&self, provider: &str, daily_limit: Option<u32>) -> Vec<Budget> {
        let mut budgets: Vec<Budget> = self
            .budgets
            .get(provider)
            .map(|budgets| budgets.clone())
            .unwrap_or_default();
        if let Some(max_calls) = daily_limit {
            match budgets.iter_mut().find(|budget| budget.window == ONE_DAY) {
                Some(daily) => daily.max_calls = max_calls,
                None => budgets.push(Budget {
                    max_calls,
                    window: ONE_DAY,
                }),
            }
        }
        budgets
    }

    /// Take one request from every window of `app` at `provider` that holds
    /// fewer than `share_percent` of its budget; when one does not, give back
    /// the windows already taken and say how long until that one resets.
    async fn acquire_share(
        &self,
        provider: &str,
        app: &str,
        daily_limit: Option<u32>,
        share_percent: u32,
    ) -> AppResult<RateLimitStatus> {
        let budgets = self.budgets_for(provider, daily_limit);
        let key = budget_counter_key(provider, app);
        let now = Utc::now();
        let mut taken: Vec<String> = Vec::with_capacity(budgets.len());
        for budget in budgets {
            let period = budget_period(now, budget.window);
            let landed = self
                .counters
                .increment_counter_below(
                    PLATFORM_SCOPE,
                    PLATFORM_SCOPE,
                    &key,
                    &period,
                    budget.allowance(share_percent),
                )
                .await?;
            if landed {
                taken.push(period);
                continue;
            }
            for period in &taken {
                self.counters
                    .increment_counter(PLATFORM_SCOPE, PLATFORM_SCOPE, &key, period, -1)
                    .await?;
            }
            let (_, end_ms) = bucket_bounds(now, budget.window);
            let retry_ms = u64::try_from(end_ms - now.timestamp_millis()).unwrap_or(0);
            return Ok(RateLimitStatus::Exceeded {
                retry_after: Duration::from_millis(retry_ms),
            });
        }
        Ok(RateLimitStatus::Allowed)
    }

    /// Replace `provider`'s budgets with `budgets`, each `(max_calls,
    /// window)`.
    pub fn set_budgets(&self, provider: &str, budgets: &[(u32, Duration)]) {
        self.budgets.insert(
            provider.to_owned(),
            budgets
                .iter()
                .map(|(max_calls, window)| Budget {
                    max_calls: *max_calls,
                    window: *window,
                })
                .collect(),
        );
        info!(
            provider = provider,
            budgets = ?budgets,
            "Provider rate limit budgets set"
        );
    }
}

/// The budget a credential's provider requests are admitted against: the
/// shared windows of the OAuth app that signs them.
#[derive(Clone)]
pub struct RequestBudget {
    limiter: Arc<ProviderRateLimiter>,
    app: String,
    daily_limit: Option<u32>,
}

impl RequestBudget {
    /// The windows of `app` (the signing client's id) in `limiter`, with the
    /// app's own `daily_limit` when it registered one.
    #[must_use]
    pub const fn new(
        limiter: Arc<ProviderRateLimiter>,
        app: String,
        daily_limit: Option<u32>,
    ) -> Self {
        Self {
            limiter,
            app,
            daily_limit,
        }
    }

    /// Admit one request to `provider`, or refuse it with
    /// [`ErrorCode::ExternalRateLimited`] and the wait until the spent window
    /// resets.
    ///
    /// A window that cannot be counted refuses work nobody is waiting on and
    /// lets a request an athlete is waiting on through, logged as an error:
    /// the budget protects the provider grant, and an unreachable database is
    /// an outage of its own, paged by that log.
    ///
    /// # Errors
    /// Returns the refusal, or for background work the counting error.
    pub async fn admit(&self, provider: &str) -> AppResult<()> {
        match self
            .limiter
            .acquire(provider, &self.app, self.daily_limit)
            .await
        {
            Ok(RateLimitStatus::Allowed) => Ok(()),
            Ok(RateLimitStatus::Exceeded { retry_after }) => Err(AppError::new(
                ErrorCode::ExternalRateLimited,
                format!("{provider} request budget is spent for now"),
            )
            .with_retry_after(retry_after.as_secs())),
            Err(e) if is_background() => Err(e),
            Err(e) => {
                error!(provider, app = %self.app, error = %e, "provider request budget could not be counted; the request is sent uncounted");
                Ok(())
            }
        }
    }
}

impl fmt::Debug for RequestBudget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestBudget")
            .field("app", &self.app)
            .field("daily_limit", &self.daily_limit)
            .finish_non_exhaustive()
    }
}

/// The budget the credentials in a provider's slot carry, cloned out so the
/// lock is released before the request is admitted.
#[cfg(any(
    feature = "provider-coros",
    feature = "provider-garmin",
    feature = "provider-intervals-icu",
    feature = "provider-strava",
    feature = "provider-terra",
    feature = "provider-whoop",
))]
pub(crate) async fn carried_by(
    credentials: &RwLock<Option<OAuth2Credentials>>,
) -> Option<RequestBudget> {
    credentials
        .read()
        .await
        .as_ref()
        .and_then(|credentials| credentials.request_budget.clone())
}

/// Admit one request to `provider` against `budget`, when the credential
/// carries one.
///
/// # Errors
/// Returns [`RequestBudget::admit`]'s refusal.
pub async fn admit(budget: Option<&RequestBudget>, provider: &str) -> AppResult<()> {
    match budget {
        Some(budget) => budget.admit(provider).await,
        None => Ok(()),
    }
}
