// ABOUTME: Provider request budgets per OAuth app and per API key, counted in the database so every instance shares them
// ABOUTME: Every provider call is admitted against the windows of the app that signs it, or of its key, before it is sent

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider request budgets, per OAuth application and per provider account.
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
//! Intervals.icu sizes an OAuth app's windows by the athletes who granted
//! it: one daily pool for the whole app, 100 requests for each of them,
//! never below 5,000 nor above 50,000, which any of its athletes may spend,
//! and a rolling 15-minute window of an eighth of the day, never below 2,500.
//! The athletes are counted in the database
//! ([`ProviderConnectionRepository::count_server_level_grants`]) and the count is
//! kept for a minute, so a budget check reads it at most once a minute per
//! instance rather than once per request. A personal API key has
//! windows of its own, with no app involved, counted under the account it
//! belongs to ([`api_key_counter_key`]). The account is the provider's id
//! for it, never the key: no secret reaches a counter.
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
//! each window, an app's pooled windows included, so a backfill can never
//! spend what the reads an athlete is waiting on need.

use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use dashmap::DashMap;
use pierre_core::constants::rate_limits;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_database::repositories::{ProviderConnectionRepository, UsageCounterRepository};
#[cfg(any(
    feature = "provider-coros",
    feature = "provider-garmin",
    feature = "provider-intervals-icu",
    feature = "provider-strava",
    feature = "provider-terra",
    feature = "provider-wahoo",
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
    feature = "provider-wahoo",
    feature = "provider-whoop",
))]
use crate::core::OAuth2Credentials;

/// One day as a `Duration`.
pub const ONE_DAY: Duration = Duration::from_hours(24);

/// Strava's short window.
pub const FIFTEEN_MINUTES: Duration = Duration::from_mins(15);

/// Wahoo's short window.
pub const FIVE_MINUTES: Duration = Duration::from_mins(5);

/// Wahoo's hourly window.
pub const ONE_HOUR: Duration = Duration::from_hours(1);

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

/// How long a count of the athletes who granted a provider's server-level
/// app sizes that app's pool before it is read again: a new grant widens the
/// pool within a minute, and each instance reads the count at most once a
/// minute, never once per request.
const GRANT_COUNT_TTL: Duration = Duration::from_mins(1);

/// The fixed buckets a rolling window as long as one bucket can overlap: a
/// window that starts inside one bucket ends inside the next.
const BUCKETS_A_ROLLING_WINDOW_SPANS: u32 = 2;

/// The `counter_key` of the budget buckets of `app` at `provider`: one set of
/// windows per OAuth client, since a provider counts each app on its own.
#[must_use]
pub fn budget_counter_key(provider: &str, app: &str) -> String {
    format!("provider_requests:{provider}:{app}")
}

/// The `counter_key` of the budget buckets of the personal API key at
/// `provider` that belongs to `account`, the provider's id for the account
/// that issued it.
#[must_use]
pub fn api_key_counter_key(provider: &str, account: &str) -> String {
    format!("provider_key_requests:{provider}:{account}")
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

/// An app's windows at a provider that sizes them by the athletes who
/// granted the app: a daily pool of `per_grant` requests for each of them,
/// between `min_daily` and `max_daily`, and a rolling `short_window` of the
/// day divided by `short_divisor`, never below `min_short`.
///
/// The limiter counts fixed buckets aligned to the epoch, while the
/// provider's short window rolls. A rolling window as long as a bucket
/// overlaps two adjacent buckets, so each bucket holds half the rolling
/// limit ([`BUCKETS_A_ROLLING_WINDOW_SPANS`]): together the two can never
/// pass it, wherever the provider's window falls. A full-size bucket would
/// let up to twice the limit through across a boundary, a burst the provider
/// refuses with its own 429, and `usage_counters` keeps no per-request
/// timestamps from which a sliding window could be counted. The pool is
/// counted per UTC day, as every daily window here is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GrantPool {
    per_grant: u32,
    min_daily: u32,
    max_daily: u32,
    short_window: Duration,
    short_divisor: u32,
    min_short: u32,
}

impl GrantPool {
    /// Intervals.icu's terms for an OAuth app (forum.intervals.icu/t/609).
    const fn intervals_icu() -> Self {
        Self {
            per_grant: rate_limits::INTERVALS_ICU_OAUTH_DAILY_PER_ATHLETE,
            min_daily: rate_limits::INTERVALS_ICU_OAUTH_DAILY_MIN,
            max_daily: rate_limits::INTERVALS_ICU_OAUTH_DAILY_MAX,
            short_window: FIFTEEN_MINUTES,
            short_divisor: rate_limits::INTERVALS_ICU_OAUTH_15MIN_DAILY_DIVISOR,
            min_short: rate_limits::INTERVALS_ICU_OAUTH_15MIN_MIN,
        }
    }

    /// The daily pool of an app `grants` athletes granted.
    fn daily(self, grants: u32) -> u32 {
        self.per_grant
            .saturating_mul(grants)
            .max(self.min_daily)
            .min(self.max_daily)
    }

    /// The windows of an app whose day holds `daily` requests: the short
    /// bucket at its share of the rolling limit that day sets, then the day.
    fn windows(self, daily: u32) -> Vec<Budget> {
        let rolling = (daily / self.short_divisor.max(1)).max(self.min_short);
        vec![
            Budget {
                max_calls: rolling / BUCKETS_A_ROLLING_WINDOW_SPANS,
                window: self.short_window,
            },
            Budget {
                max_calls: daily,
                window: ONE_DAY,
            },
        ]
    }
}

/// The athletes counted as having granted a provider's server-level app,
/// and when they were counted.
#[derive(Debug, Clone, Copy)]
struct GrantCount {
    grants: u32,
    counted_at: Instant,
}

/// The budgets `map` holds for `provider`, cloned out so no map guard is
/// held across a database call; none when it holds no entry.
fn configured(map: &DashMap<String, Vec<Budget>>, provider: &str) -> Vec<Budget> {
    map.get(provider)
        .map(|budgets| budgets.clone())
        .unwrap_or_default()
}

/// Holds each provider's budgets and takes requests from them in the
/// database, per OAuth app and per personal API key, so every instance
/// counts into the same windows.
pub struct ProviderRateLimiter {
    /// Where the windows are counted: `usage_counters`, shared by every
    /// instance of the backend.
    counters: Arc<dyn UsageCounterRepository>,
    /// Where the athletes who granted a provider's server-level app are
    /// counted, for the providers in `pools`.
    grants: Arc<dyn ProviderConnectionRepository>,
    /// Map of `provider_name` -> every budget a request to it must fit, per
    /// app. Configuration only, the same on every instance; read and cloned
    /// out before any database call, as are `pools` and `api_key_budgets`.
    budgets: DashMap<String, Vec<Budget>>,
    /// Map of `provider_name` -> how its apps' windows are sized from the
    /// athletes who granted them, in place of `budgets`.
    pools: DashMap<String, GrantPool>,
    /// Map of `provider_name` -> the last count of the athletes who granted
    /// its server-level app, read again once [`GRANT_COUNT_TTL`] old.
    grant_counts: DashMap<String, GrantCount>,
    /// Map of `provider_name` -> the budgets it keeps for each personal API
    /// key.
    api_key_budgets: DashMap<String, Vec<Budget>>,
}

/// Whose windows a request is counted in.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Holder {
    /// The OAuth app whose client signs the credential, with the daily
    /// budget its operator registered when it has one.
    App {
        app: String,
        daily_limit: Option<u32>,
    },
    /// A personal API key, by the provider account it belongs to.
    ApiKey { account: String },
}

impl Holder {
    /// The signing app, `None` for a personal API key.
    fn app(&self) -> Option<&str> {
        match self {
            Self::App { app, .. } => Some(app),
            Self::ApiKey { .. } => None,
        }
    }
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
    /// rate limits, sizing a pooling provider's app windows by the grants
    /// `grants` counts.
    #[must_use]
    pub fn new(
        counters: Arc<dyn UsageCounterRepository>,
        grants: Arc<dyn ProviderConnectionRepository>,
    ) -> Self {
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
        // Wahoo: three windows per app, the production limits (a sandbox
        // app's are an eighth to a twentieth of these, and Wahoo's own 429
        // stops it first).
        budgets.insert(
            "wahoo".to_owned(),
            vec![
                Budget {
                    max_calls: rate_limits::WAHOO_RATE_LIMIT_5MIN,
                    window: FIVE_MINUTES,
                },
                Budget {
                    max_calls: rate_limits::WAHOO_RATE_LIMIT_HOURLY,
                    window: ONE_HOUR,
                },
                Budget {
                    max_calls: rate_limits::WAHOO_RATE_LIMIT_DAILY,
                    window: ONE_DAY,
                },
            ],
        );
        // Intervals.icu: an OAuth app's windows are one pool its athletes
        // share, sized by how many granted it; a personal API key has
        // windows of its own and no app.
        let pools = DashMap::new();
        pools.insert("intervals_icu".to_owned(), GrantPool::intervals_icu());
        let api_key_budgets = DashMap::new();
        api_key_budgets.insert(
            "intervals_icu".to_owned(),
            vec![
                Budget {
                    max_calls: rate_limits::INTERVALS_ICU_API_KEY_15MIN,
                    window: FIFTEEN_MINUTES,
                },
                Budget {
                    max_calls: rate_limits::INTERVALS_ICU_API_KEY_DAILY,
                    window: ONE_DAY,
                },
            ],
        );

        info!(
            provider_count = budgets.len() + pools.len(),
            "Provider rate limiter initialized"
        );

        Self {
            counters,
            grants,
            budgets,
            pools,
            grant_counts: DashMap::new(),
            api_key_budgets,
        }
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
        self.acquire_for(
            provider,
            &Holder::App {
                app: app.to_owned(),
                daily_limit,
            },
        )
        .await
    }

    /// Take one request to `provider` from every window `holder` is counted
    /// in, at the share the running task may spend.
    async fn acquire_for(&self, provider: &str, holder: &Holder) -> AppResult<RateLimitStatus> {
        let background = is_background();
        let share = if background {
            BACKGROUND_SHARE_PERCENT
        } else {
            100
        };
        let windows = self.windows_for(provider, holder).await;
        let status = self.take(&windows, share).await?;
        if let RateLimitStatus::Exceeded { retry_after } = &status {
            // No app: the windows spent are a personal API key's own.
            let app = holder.app();
            if background {
                debug!(
                    provider,
                    app,
                    retry_after_secs = retry_after.as_secs(),
                    "Background share of the provider budget spent"
                );
            } else {
                warn!(
                    provider,
                    app,
                    retry_after_secs = retry_after.as_secs(),
                    "Provider request budget spent"
                );
            }
        }
        Ok(status)
    }

    /// The windows a request to `provider` must fit: the provider's, with the
    /// daily window's allowance replaced by `daily_limit` when the app has
    /// its own (added when the provider has none).
    fn budgets_for(&self, provider: &str, daily_limit: Option<u32>) -> Vec<Budget> {
        let mut budgets = configured(&self.budgets, provider);
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

    /// The windows of an app at `provider`: sized by the athletes who
    /// granted the app where the provider pools them, else the provider's,
    /// and either way with the day's allowance replaced by `daily_limit`
    /// when the app's operator registered one.
    ///
    /// At a pooling provider an app with no registered limit is the
    /// server-level app, whose grants are the ones counted: a tenant's own
    /// app always carries the limit its operator registered, the Strava pool
    /// apps are Strava's, and the `user_oauth_app_credentials` provider CHECK
    /// admits no user's own app for Intervals.icu.
    async fn app_budgets(&self, provider: &str, daily_limit: Option<u32>) -> Vec<Budget> {
        let pool = self.pools.get(provider).map(|pool| *pool);
        let Some(pool) = pool else {
            return self.budgets_for(provider, daily_limit);
        };
        let daily = match daily_limit {
            Some(daily) => daily,
            None => pool.daily(self.granted_athletes(provider).await),
        };
        pool.windows(daily)
    }

    /// The athletes who granted `provider`'s server-level app, as counted
    /// within the last [`GRANT_COUNT_TTL`], else counted now.
    ///
    /// A count the database cannot give is taken as none, which holds the
    /// pool to its minimum, the least the provider ever allows an app, and
    /// is kept as long as a count is, so a database that cannot answer is
    /// not asked again on every request.
    async fn granted_athletes(&self, provider: &str) -> u32 {
        let cached = self.grant_counts.get(provider).map(|count| *count);
        if let Some(count) = cached.filter(|count| count.counted_at.elapsed() < GRANT_COUNT_TTL) {
            return count.grants;
        }
        let grants = match self.grants.count_server_level_grants(provider).await {
            Ok(grants) => grants,
            Err(e) => {
                warn!(provider, error = %e, "The athletes who granted the app could not be counted; its request pool is held to its minimum");
                0
            }
        };
        self.grant_counts.insert(
            provider.to_owned(),
            GrantCount {
                grants,
                counted_at: Instant::now(),
            },
        );
        grants
    }

    /// Every window a request to `provider` counted for `holder` is taken
    /// from, each with the counter key it is counted under: an app's, or a
    /// personal key's.
    async fn windows_for(&self, provider: &str, holder: &Holder) -> Vec<(String, Budget)> {
        let (key, budgets) = match holder {
            Holder::App { app, daily_limit } => (
                budget_counter_key(provider, app),
                self.app_budgets(provider, *daily_limit).await,
            ),
            Holder::ApiKey { account } => (
                api_key_counter_key(provider, account),
                configured(&self.api_key_budgets, provider),
            ),
        };
        budgets
            .into_iter()
            .map(|budget| (key.clone(), budget))
            .collect()
    }

    /// Take one request from every one of `windows` that holds fewer than
    /// `share_percent` of its budget; when one does not, give back the
    /// windows already taken and say how long until that one resets.
    async fn take(
        &self,
        windows: &[(String, Budget)],
        share_percent: u32,
    ) -> AppResult<RateLimitStatus> {
        let now = Utc::now();
        let mut taken: Vec<(&str, String)> = Vec::with_capacity(windows.len());
        for (key, budget) in windows {
            let period = budget_period(now, budget.window);
            let landed = self
                .counters
                .increment_counter_below(
                    PLATFORM_SCOPE,
                    PLATFORM_SCOPE,
                    key,
                    &period,
                    budget.allowance(share_percent),
                )
                .await?;
            if landed {
                taken.push((key, period));
                continue;
            }
            for (key, period) in &taken {
                self.counters
                    .increment_counter(PLATFORM_SCOPE, PLATFORM_SCOPE, key, period, -1)
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
    /// window)`, in place of any pool its apps' windows were sized by.
    pub fn set_budgets(&self, provider: &str, budgets: &[(u32, Duration)]) {
        self.pools.remove(provider);
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
/// shared windows of the OAuth app that signs them, or those its provider
/// keeps for a personal API key.
#[derive(Clone)]
pub struct RequestBudget {
    limiter: Arc<ProviderRateLimiter>,
    holder: Holder,
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
            holder: Holder::App { app, daily_limit },
        }
    }

    /// The windows the provider keeps for one personal API key, counted for
    /// `account`, the provider's id for the account the key belongs to. No
    /// app signs a key, so no app's windows apply.
    #[must_use]
    pub const fn for_api_key(limiter: Arc<ProviderRateLimiter>, account: String) -> Self {
        Self {
            limiter,
            holder: Holder::ApiKey { account },
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
        match self.limiter.acquire_for(provider, &self.holder).await {
            Ok(RateLimitStatus::Allowed) => Ok(()),
            Ok(RateLimitStatus::Exceeded { retry_after }) => Err(AppError::new(
                ErrorCode::ExternalRateLimited,
                format!("{provider} request budget is spent for now"),
            )
            .with_retry_after(retry_after.as_secs())),
            Err(e) if is_background() => Err(e),
            Err(e) => {
                error!(provider, app = self.holder.app(), error = %e, "provider request budget could not be counted; the request is sent uncounted");
                Ok(())
            }
        }
    }
}

impl fmt::Debug for RequestBudget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RequestBudget")
            .field("holder", &self.holder)
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
    feature = "provider-wahoo",
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Intervals.icu's daily pool is 100 requests for each athlete who
    /// granted the app, held to at least 5,000 and at most 50,000: the 50th
    /// athlete still leaves it at the minimum and the 51st widens it, the
    /// 500th fills it and the 501st adds nothing.
    #[test]
    fn the_intervals_pool_grows_by_a_hundred_an_athlete_between_its_clamps() {
        let pool = GrantPool::intervals_icu();
        assert_eq!(pool.daily(0), 5_000);
        assert_eq!(pool.daily(50), 5_000);
        assert_eq!(pool.daily(51), 5_100);
        assert_eq!(pool.daily(499), 49_900);
        assert_eq!(pool.daily(500), 50_000);
        assert_eq!(pool.daily(501), 50_000);
        assert_eq!(pool.daily(u32::MAX), 50_000, "the product saturates");
    }

    /// The rolling 15-minute limit is an eighth of the day, at least 2,500,
    /// and each fixed 15-minute bucket holds half of it, so two adjacent
    /// buckets never pass it: 1,250 a bucket until the day passes 20,000,
    /// 3,125 at the day's 50,000.
    #[test]
    fn each_fifteen_minute_bucket_holds_half_the_rolling_limit() {
        let pool = GrantPool::intervals_icu();
        let short = |daily| {
            let windows = pool.windows(daily);
            assert_eq!(windows[1].max_calls, daily, "the day is the pool");
            assert_eq!(windows[1].window, ONE_DAY);
            assert_eq!(windows[0].window, FIFTEEN_MINUTES);
            windows[0].max_calls
        };
        assert_eq!(short(5_000), 1_250);
        assert_eq!(short(20_000), 1_250);
        assert_eq!(short(24_000), 1_500);
        assert_eq!(short(50_000), 3_125);
    }
}
