// ABOUTME: Reads the daily CTL/ATL/TSB series from stored activities, and the rows a model may read of it
// ABOUTME: One rule for every reader: under a gate that withholds a session, the series is recomputed without it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The read side of the daily training-state rollup.
//!
//! The persisted rollup (`training_history`) sums every stored activity: it is
//! the athlete's own series, written by `pierre-tool-runtime`'s compute. A
//! model, or a caller over an external transport, may read only what each
//! provider's terms let it see (carnet#723, carnet#724), so
//! [`history_rows_for_model`] answers it with a series recomputed from the
//! permitted activities whenever anything in the window is withheld.
//!
//! The rule lives here, below both of its readers: the tool runtime's tools
//! and this crate's background jobs (the outcome evaluator's LLM judge,
//! carnet#734). One copy, so a background path cannot drift from the tools.
//!
//! The window is the one the stored history can honestly warm: `ctl`/`atl`/
//! `tsb` are plain `f64` seeded at zero, so a day without
//! [`warmup_days`] of activities behind it is never computed — it would read
//! as a real, low chronic load.

use chrono::{Duration, NaiveDate, TimeZone, Utc};
use dravr_cageux::config::intelligence::AlgorithmConfig;
use pierre_config::environment::default_provider;
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::civil_time::{local_date, resolve_zone};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, DailyTrainingState, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_fitness_compute::training_history_compute::{
    compute_training_history, warmup_days, AthleteInputs, MAX_BACKFILL_DAYS,
};
use pierre_providers::{ai_scope, backend_resolver};
use uuid::Uuid;

/// Read limit for a single deterministic read of a historical window from the
/// durable activity cache.
///
/// Decoupled from the user's display limit so the cache read returns the
/// COMPLETE window (the display limit caps only the returned list afterwards).
/// Generous enough to cover a dense season (>=2 activities/day for a year)
/// without truncating the window read; the durable table is already
/// retention-bounded, so this only guards against an unbounded read. When a
/// window fills this cap the served `window_total` is only a lower bound, so
/// `activity_coverage_note` frames the count as "at least {total}".
pub const HISTORICAL_WINDOW_READ_LIMIT: usize = 2_000;

/// Slack (days) added to each end of the cache read.
///
/// An activity whose local date sits inside the window must not be missed
/// because its UTC instant falls outside it, and zone offsets reach ±14h. The
/// pure compute re-filters on the athlete's civil date, so reading wide is free
/// and reading tight loses rows.
const CACHE_READ_EDGE_SLACK_DAYS: i64 = 2;

/// What a history read needs: the stored data, the provider terms that govern
/// what a model may see of it, and the algorithm configuration it is scored by.
#[derive(Clone, Copy)]
pub struct HistorySources<'a> {
    /// The repositories the activities, thresholds and rollup are read from.
    pub repos: &'a RepositoryRegistry,
    /// Each provider's terms (the provider registry).
    pub terms: &'a dyn ProviderTerms,
    /// The training-load algorithm configuration the series is scored by.
    pub algorithms: &'a AlgorithmConfig,
}

/// How much of the requested window the stored activities could stand behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryCoverage {
    /// Stored history warms the whole requested window.
    Complete,
    /// Stored history begins too late to warm the whole ask; only
    /// `[trustworthy_from, to]` was computed.
    Partial {
        /// First day whose CTL/ATL/TSB the stored history can stand behind.
        trustworthy_from: NaiveDate,
    },
    /// No stored activities in the read window at all. Nothing was computed —
    /// a zero-seeded series would read as a real chronic load.
    NoStoredActivities,
}

/// Daily rows computed from the durable cache for a read that stores nothing.
#[derive(Debug, Clone)]
pub struct TrainingHistoryRead {
    /// One row per day of `[coverage's first vouched day, to]`, oldest first.
    /// Empty when the stored history warms no day of the ask.
    pub states: Vec<DailyTrainingState>,
    /// How much of the ask the stored history supported.
    pub coverage: HistoryCoverage,
}

/// Read the athlete's configured timezone, if any.
///
/// # Errors
///
/// Returns [`AppError`] when the user read fails.
pub async fn user_timezone(repos: &RepositoryRegistry, user_id: Uuid) -> AppResult<Option<String>> {
    Ok(repos
        .users
        .get_global(user_id)
        .await?
        .and_then(|u| u.timezone))
}

/// Compute daily training-history rows for `[from, to]` from the durable
/// activity cache, and nothing else.
///
/// The read a page makes: the same stored activities, thresholds, civil days
/// and warm-up rule as the persisted compute, so the page and the agent read
/// one series — but it writes no row, clears none, and asks the capture rail
/// for nothing. A page is opened many times a day by an athlete who asked for
/// no history; a capture per visit would page a scrape backend again every
/// time a thin history was looked at.
///
/// An athlete with no connected provider has no stored activities, and is
/// answered [`HistoryCoverage::NoStoredActivities`] rather than refused.
///
/// # Errors
///
/// Returns [`AppError`] when the window is invalid or a repository read fails.
pub async fn read_history_from_cache(
    sources: HistorySources<'_>,
    tenant_id: TenantId,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> AppResult<TrainingHistoryRead> {
    validate_window(from, to)?;
    let nothing_stored = TrainingHistoryRead {
        states: Vec::new(),
        coverage: HistoryCoverage::NoStoredActivities,
    };
    let Some(backend) = resolve_compute_backend(sources.repos, tenant_id, user_id).await? else {
        return Ok(nothing_stored);
    };
    let window = load_stored_window(sources, tenant_id, user_id, backend, from, to).await?;
    let Some(oldest_stored) = window.oldest_stored else {
        return Ok(nothing_stored);
    };
    let trustworthy_from = first_vouched_day(from, oldest_stored, window.warmup);
    if trustworthy_from > to {
        return Ok(TrainingHistoryRead {
            states: Vec::new(),
            coverage: HistoryCoverage::Partial { trustworthy_from },
        });
    }
    let states = compute_states(sources, tenant_id, user_id, &window, trustworthy_from, to).await?;
    Ok(TrainingHistoryRead {
        states,
        coverage: if trustworthy_from <= from {
            HistoryCoverage::Complete
        } else {
            HistoryCoverage::Partial { trustworthy_from }
        },
    })
}

/// Read-only fetch of persisted rows in `[from, to]`.
///
/// The read side of the rollup: no cache read, no compute, no provider.
///
/// # Errors
///
/// Returns [`AppError`] when the window is inverted or the repository read fails.
pub async fn fetch_history_rows(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> AppResult<Vec<DailyTrainingState>> {
    if to < from {
        return Err(AppError::invalid_input("from > to"));
    }
    repos
        .training_history
        .get_training_history(tenant_id, user_id, from, to)
        .await
}

/// The history rows a model, or a caller over an external transport, may read
/// for `[from, to]`.
///
/// The persisted rollup sums every stored activity. Under a gate whose window
/// holds activities a provider's terms withhold — from AI (carnet#723) or from
/// this transport (carnet#724) — the rows are computed on the fly from the
/// permitted activities instead, under the same gates: the same warm-up rule
/// as the rollup, so the two agree when nothing is withheld, and then the
/// stored rows are served.
///
/// # Errors
///
/// Returns an error when the window is inverted or a read fails.
pub async fn history_rows_for_model(
    sources: HistorySources<'_>,
    tenant_id: TenantId,
    user_id: Uuid,
    from: NaiveDate,
    to: NaiveDate,
) -> AppResult<Vec<DailyTrainingState>> {
    if ai_scope::policies_apply() {
        // Boxed: the compute's future is large, and every caller of this read
        // would otherwise carry it inline.
        let (computed, withheld) = ai_scope::tallied(Box::pin(read_history_from_cache(
            sources, tenant_id, user_id, from, to,
        )))
        .await;
        if !withheld.is_empty() {
            ai_scope::record_withheld(withheld);
            return Ok(computed?.states);
        }
    }
    fetch_history_rows(sources.repos, tenant_id, user_id, from, to).await
}

/// Refuse a window that is inverted or longer than the compute is bounded to.
///
/// # Errors
///
/// Returns [`AppError::invalid_input`] for an inverted or oversized window.
pub fn validate_window(from: NaiveDate, to: NaiveDate) -> AppResult<()> {
    if to < from {
        return Err(AppError::invalid_input("from > to"));
    }
    if (to - from).num_days() > MAX_BACKFILL_DAYS {
        return Err(AppError::invalid_input(format!(
            "backfill window exceeds the maximum of {MAX_BACKFILL_DAYS} days"
        )));
    }
    Ok(())
}

/// The athlete's stored activities behind a window, with what computing from
/// them needs.
pub struct StoredWindow {
    /// The backend whose cached rows were read.
    pub backend: ComputeBackend,
    /// The athlete's configured timezone, if any.
    pub timezone: Option<String>,
    /// Days of history a day needs behind it, at the configured chronic window.
    pub warmup: i64,
    /// Stored activities covering the window and its warm-up.
    pub activities: Vec<Activity>,
    /// The athlete's civil date of the oldest of them; `None` when the read
    /// held nothing.
    pub oldest_stored: Option<NaiveDate>,
}

/// Read the stored activities behind `[from, to]` and its warm-up.
///
/// What a model may see of them, when a gate applies: the persisted compute
/// runs this under `ai_scope::unfiltered`, so the rollup keeps every row.
///
/// # Errors
///
/// Returns [`AppError`] when a repository read fails.
pub async fn load_stored_window(
    sources: HistorySources<'_>,
    tenant_id: TenantId,
    user_id: Uuid,
    backend: ComputeBackend,
    from: NaiveDate,
    to: NaiveDate,
) -> AppResult<StoredWindow> {
    // The rollup buckets on the athlete's civil day. Persisting UTC-day buckets
    // shifted the whole CTL/ATL/TSB series against their own calendar for
    // anyone training in the evening (registre#200).
    let timezone = user_timezone(sources.repos, user_id).await?;
    let zone = resolve_zone(timezone.as_deref());

    let warmup = warmup_days(sources.algorithms.params.training_load_ctl_days);
    let activities = read_cached_window(
        sources,
        tenant_id,
        user_id,
        &backend.slug,
        from - Duration::days(warmup),
        to,
    )
    .await?;
    let oldest_stored = activities
        .iter()
        .map(|a: &Activity| local_date(a.start_date(), zone))
        .min();
    Ok(StoredWindow {
        backend,
        timezone,
        warmup,
        activities,
        oldest_stored,
    })
}

/// The first day the stored history can stand behind: anything earlier would
/// be computed against a partly-empty warm-up and read as a real, low CTL.
#[must_use]
pub fn first_vouched_day(from: NaiveDate, oldest_stored: NaiveDate, warmup: i64) -> NaiveDate {
    from.max(oldest_stored + Duration::days(warmup))
}

/// The daily rows of `[from, to]` from a stored window, each session scored
/// against the athlete's saved thresholds.
///
/// # Errors
///
/// Returns [`AppError`] when the thresholds read or the compute fails.
pub async fn compute_states(
    sources: HistorySources<'_>,
    tenant_id: TenantId,
    user_id: Uuid,
    window: &StoredWindow,
    from: NaiveDate,
    to: NaiveDate,
) -> AppResult<Vec<DailyTrainingState>> {
    let inputs = athlete_inputs(sources.repos, tenant_id, user_id).await?;
    compute_training_history(
        &window.activities,
        inputs,
        from,
        to,
        sources.algorithms,
        window.timezone.as_deref(),
    )
    .map_err(|e| AppError::internal(format!("training-load series: {e}")))
}

/// The backend a compute reads, and whether its connection is still live.
pub struct ComputeBackend {
    /// Canonical slug the cached rows are keyed by.
    pub slug: String,
    /// Whether the athlete must reconnect before any new history can arrive.
    pub requires_reauth: bool,
}

/// Resolve the backend slug whose cached rows a compute reads, or `None`
/// when the athlete has no provider connected.
///
/// Canonicalised through [`backend_resolver::resolve_backend`] because that is
/// what the write side keys on: a Garmin athlete's rows are written under
/// `sciotte_garmin` while the connection says `garmin`, and reading the raw
/// connection slug would miss every row it wrote.
///
/// # Errors
///
/// Returns [`AppError`] when the connection read fails.
pub async fn resolve_compute_backend(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
) -> AppResult<Option<ComputeBackend>> {
    let (requested, requires_reauth) = if let Some(p) = default_provider() {
        (p, false)
    } else if let Some(conn) = repos
        .provider_connections
        .resolve_most_recent(user_id, Some(tenant_id))
        .await?
    {
        let requires_reauth = conn.status.requires_reauth();
        (conn.provider, requires_reauth)
    } else {
        return Ok(None);
    };
    Ok(Some(ComputeBackend {
        slug: backend_resolver::resolve_backend(
            &repos.auth_repos(),
            user_id,
            Some(tenant_id),
            &requested,
        )
        .await,
        requires_reauth,
    }))
}

/// Read the athlete's stored activities covering `[start, to]`, newest first.
async fn read_cached_window(
    sources: HistorySources<'_>,
    tenant_id: TenantId,
    user_id: Uuid,
    backend: &str,
    start: NaiveDate,
    to: NaiveDate,
) -> AppResult<Vec<Activity>> {
    let slack = Duration::days(CACHE_READ_EDGE_SLACK_DAYS);
    let start_ts = Utc.from_utc_datetime(&(start - slack).and_hms_opt(0, 0, 0).unwrap_or_default());
    let end_ts = Utc.from_utc_datetime(&(to + slack).and_hms_opt(0, 0, 0).unwrap_or_default());
    let limit = i64::try_from(HISTORICAL_WINDOW_READ_LIMIT).unwrap_or(i64::MAX);
    let rows = sources
        .repos
        .activity_cache
        .get_cached_activities(user_id, &tenant_id, Some(backend), start_ts, end_ts, limit)
        .await?;
    // A history computed for a model sums only what it may see; the persisted
    // rollup is computed unfiltered.
    Ok(ai_scope::filter_activities(sources.terms, rows))
}

/// Per-user physiology, absent where the profile is silent.
async fn athlete_inputs(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    user_id: Uuid,
) -> AppResult<AthleteInputs> {
    let profile = repos
        .user_physiological_profile
        .get_user_physiological_profile(tenant_id, user_id)
        .await?
        // A profile written from first-party-only data is withheld from an
        // external caller (carnet#769).
        .filter(|profile| ai_scope::admit_derived(profile.transport_policy));
    Ok(AthleteInputs::from_profile(profile.as_ref()))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::future::Future;

    use super::*;
    use chrono::{DateTime, NaiveTime};
    use pierre_core::ai_policy::SourcePolicy;
    use pierre_core::config::profiles::FitnessLevel;
    use pierre_core::models::{
        ActivityBuilder, ConnectionType, SportType, UserPhysiologicalProfile,
    };
    use pierre_core::transport::{Transport, TransportPolicy};
    use pierre_database::backends::factory::Database;
    use pierre_providers::provider_terms::{NOLIO, NOLIO_TRANSPORT};
    use pierre_test_support::db::create_test_db;
    use pierre_test_support::server::create_test_user;

    /// The relay whose terms the tests read under: Nolio, as its API terms
    /// declare it (garmin allowed, strava existence only, zepp denied).
    pub const RELAY: &str = "nolio";

    /// Terms that know only the Nolio relay.
    pub struct NolioTerms;

    impl ProviderTerms for NolioTerms {
        fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy> {
            (provider == RELAY).then_some(&NOLIO)
        }

        fn transport_policy(&self, provider: &str) -> Option<TransportPolicy> {
            (provider == RELAY).then_some(NOLIO_TRANSPORT)
        }
    }

    /// Run `fut` the way a model reads on one of Dravr's own surfaces.
    pub async fn as_model_read<F: Future>(fut: F) -> F::Output {
        ai_scope::serve_over(Transport::PlatformJob, ai_scope::for_model(fut)).await
    }

    /// An athlete with a database, a tenant and the relay connected.
    pub struct Athlete {
        pub db: Database,
        pub user_id: Uuid,
        pub tenant: TenantId,
    }

    pub async fn athlete() -> Athlete {
        let db = create_test_db().await.unwrap();
        let user = create_test_user(&format!("{}@relay.test", Uuid::new_v4()), None);
        let user_id = db.repositories().users.create(&user).await.unwrap();
        let tenant = TenantId::parse_str(&Uuid::new_v4().to_string()).unwrap();
        let repos = db.repositories();
        repos
            .provider_connections
            .register_connection(user_id, tenant, RELAY, &ConnectionType::OAuth, None)
            .await
            .unwrap();
        // Thresholds, so every session scores a real, non-zero load.
        let profile = UserPhysiologicalProfile {
            user_id,
            vo2_max: Some(52.0),
            resting_hr: Some(50),
            max_hr: Some(190),
            lactate_threshold_percentage: Some(0.85),
            threshold_hr: None,
            age: Some(34),
            weight: Some(72.0),
            fitness_level: FitnessLevel::Advanced,
            primary_sport: SportType::Run,
            training_experience_years: Some(10),
            ftp_watts: Some(280),
            threshold_pace_sec_per_km: Some(225.0),
            hr_zones: None,
            power_zones: None,
            critical_power_watts: None,
            w_prime_joules: None,
            critical_speed_mps: None,
            d_prime_meters: None,
            transport_policy: TransportPolicy::AnyTransport,
        };
        repos
            .user_physiological_profile
            .upsert_user_physiological_profile(tenant, user_id, &profile)
            .await
            .unwrap();
        Athlete {
            db,
            user_id,
            tenant,
        }
    }

    /// Noon UTC `days_ago` days before today.
    pub fn noon(days_ago: i64) -> DateTime<Utc> {
        let day = Utc::now().date_naive() - Duration::days(days_ago);
        Utc.from_utc_datetime(&day.and_time(NaiveTime::from_hms_opt(12, 0, 0).unwrap()))
    }

    /// A relayed session recorded by `source`.
    pub fn relayed(id: &str, name: &str, source: &str, start: DateTime<Utc>) -> Activity {
        ActivityBuilder::new(id, name, SportType::Run, start, 3_600, RELAY)
            .distance_meters(12_000.0)
            .average_heart_rate(160)
            .source(source)
            .build()
    }

    impl Athlete {
        pub async fn cache(&self, activities: &[Activity]) {
            self.db
                .repositories()
                .activity_cache
                .upsert_activities(self.user_id, &self.tenant, RELAY, activities)
                .await
                .unwrap();
        }

        /// A garmin-sourced run every third day from `oldest_days_ago` to today.
        pub async fn cache_garmin_history(&self, oldest_days_ago: i64) {
            let runs: Vec<Activity> = (0..=oldest_days_ago)
                .rev()
                .filter(|d| d % 3 == 0)
                .map(|d| relayed(&format!("g{d}"), "Garmin run", "garmin", noon(d)))
                .collect();
            self.cache(&runs).await;
        }

        /// A persisted rollup row for each day of `[from, to]` carrying `tsb`.
        pub async fn persist_rollup(&self, from: NaiveDate, to: NaiveDate, tsb: f64) {
            let states: Vec<DailyTrainingState> = from
                .iter_days()
                .take_while(|d| *d <= to)
                .map(|date| DailyTrainingState {
                    tsb,
                    ctl: 60.0,
                    atl: 60.0 - tsb,
                    form_ctl: 60.0,
                    ..DailyTrainingState::zero(date)
                })
                .collect();
            self.db
                .repositories()
                .training_history
                .upsert_training_history_batch(self.tenant, self.user_id, &states)
                .await
                .unwrap();
        }
    }

    fn sources<'a>(fx: &'a Athlete, algorithms: &'a AlgorithmConfig) -> HistorySources<'a> {
        HistorySources {
            repos: fx.db.repositories(),
            terms: &NolioTerms,
            algorithms,
        }
    }

    const PERSISTED_TSB: f64 = 42.0;

    #[tokio::test]
    async fn a_model_reads_a_series_computed_without_the_withheld_session() {
        let fx = athlete().await;
        let algorithms = AlgorithmConfig::default();
        let to = Utc::now().date_naive();
        let from = to - Duration::days(14);
        fx.persist_rollup(from, to, PERSISTED_TSB).await;
        fx.cache_garmin_history(200).await;
        // A zepp session on a day no garmin run falls on: Nolio denies zepp.
        fx.cache(&[relayed("z", "Secret zepp ride", "zepp", noon(1))])
            .await;

        let athlete_view =
            history_rows_for_model(sources(&fx, &algorithms), fx.tenant, fx.user_id, from, to)
                .await
                .unwrap();
        assert!(
            athlete_view
                .iter()
                .all(|row| (row.tsb - PERSISTED_TSB).abs() < f64::EPSILON),
            "outside a model read the stored rollup is served"
        );

        let model_view = as_model_read(history_rows_for_model(
            sources(&fx, &algorithms),
            fx.tenant,
            fx.user_id,
            from,
            to,
        ))
        .await
        .unwrap();
        assert!(!model_view.is_empty(), "the cache warms the window");
        assert!(
            model_view
                .iter()
                .all(|row| (row.tsb - PERSISTED_TSB).abs() > f64::EPSILON),
            "a model never reads the rollup that summed the withheld session"
        );
        let zepp_day = model_view
            .iter()
            .find(|row| row.date == noon(1).date_naive())
            .expect("the zepp day is in the window");
        assert!(
            zepp_day.daily_load.abs() < f64::EPSILON,
            "the withheld session adds no load: {}",
            zepp_day.daily_load
        );

        let unfiltered =
            read_history_from_cache(sources(&fx, &algorithms), fx.tenant, fx.user_id, from, to)
                .await
                .unwrap();
        let athlete_zepp_day = unfiltered
            .states
            .iter()
            .find(|row| row.date == noon(1).date_naive())
            .unwrap();
        assert!(
            athlete_zepp_day.daily_load > 0.0,
            "the athlete's own series does count it"
        );
    }

    #[tokio::test]
    async fn with_nothing_withheld_a_model_reads_the_stored_rollup() {
        let fx = athlete().await;
        let algorithms = AlgorithmConfig::default();
        let to = Utc::now().date_naive();
        let from = to - Duration::days(14);
        fx.persist_rollup(from, to, PERSISTED_TSB).await;
        fx.cache_garmin_history(200).await;

        let model_view = as_model_read(history_rows_for_model(
            sources(&fx, &algorithms),
            fx.tenant,
            fx.user_id,
            from,
            to,
        ))
        .await
        .unwrap();
        assert_eq!(model_view.len(), 15);
        assert!(model_view
            .iter()
            .all(|row| (row.tsb - PERSISTED_TSB).abs() < f64::EPSILON));
    }
}
