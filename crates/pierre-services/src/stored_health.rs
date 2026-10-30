// ABOUTME: Stored sleep, recovery and health-snapshot reads under the provider terms of each row's data source
// ABOUTME: Filters rows by their data source's provider before any per-metric merge across sources

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Stored health reads (carnet#771).
//!
//! A stored sleep session, recovery reading or health snapshot names the data
//! source that synced it, never a provider, and every reader merges them per
//! night or per day across sources. So the provider terms — the AI rules
//! (carnet#723) and the transport gate (carnet#724) — are applied here, to
//! each row by its data source's provider and upstream source, before a reader
//! can merge: once merged, a withheld source's metric sits inside a served
//! record where no later filter can find it.
//!
//! Every tool, prompt and rail that reads these rows reads them through this
//! module. Outside any gate (the athlete's own surfaces, sync sweeps) the rows
//! come back as stored and the athlete's data sources are not read at all.
//!
//! # The one merge (carnet#1281)
//!
//! [`merged_sleep`], [`merged_recovery`] and [`merged_body`] are the only
//! callers of dravr-equilibre's per-night and per-day merge: a reader gets
//! one record per night or per date, gated, merged and attributed, and
//! never merges rows itself, so two tools can no longer answer the same
//! night two ways. A [`SourceFilter`] says which rows take part.
//!
//! # Garmin attribution of intervals.icu wellness (carnet#521)
//!
//! intervals.icu's wellness rows name no source or device, while its API terms
//! bind Garmin-sourced data to Garmin's attribution. A row is attributed by the
//! athlete's devices instead: when the most recent intervals.icu activity that
//! names its recorder was recorded on a Garmin device, the athlete's
//! intervals.icu wellness is attributed to that device. The device is read
//! from the activity cache, once per read and only when an intervals.icu row
//! is served — never from the provider.
//!
//! # Garmin attribution of Garmin data sources (carnet#1154)
//!
//! A row synced from a Garmin data source — the Garmin provider itself, a
//! service relaying Garmin, or one naming a Garmin device — owes Garmin's
//! attribution by the device its data source names. It is read from the data
//! sources a gate already lists; outside any gate, from the provider the row
//! was stored under.

use chrono::{DateTime, Duration, Utc};
use dravr_equilibre_sync::{
    merge_health_metrics, merge_recovery_metrics, merge_sleep_sessions, Merged,
};
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::constants::oauth_providers::{
    activity_attribution, garmin_device, garmin_service_attribution, source_attribution, GARMIN,
    GARMIN_ATTRIBUTION, INTERVALS_ICU,
};
use pierre_core::errors::AppResult;
use pierre_core::models::{
    DataSource, StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession, TenantId,
};
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope::{self, StoredHealthRecord};
use uuid::Uuid;

/// How far back the athlete's intervals.icu activities are searched for the
/// device that names their wellness recorder.
const DEVICE_LOOKBACK_DAYS: i64 = 365;

/// How many of those activities, newest first, are searched.
const DEVICE_LOOKBACK_ACTIVITIES: i64 = 50;

/// Stored health records as a reader may see them, with the attribution
/// they owe.
#[derive(Debug, Clone)]
pub struct StoredHealth<T> {
    /// The records the gates in force let the reader see: merged ones from
    /// the `merged_*` reads, raw rows inside this module.
    pub records: Vec<T>,
    /// The Garmin attribution to show with them: the device of the first row
    /// a Garmin data source synced (`Garmin Forerunner 965`, or `Garmin` when
    /// it names none); else, when an intervals.icu row is among them and the
    /// athlete's most recent intervals.icu recording was Garmin's, that
    /// device; `None` otherwise.
    pub attribution: Option<String>,
}

/// The attribution the athlete's intervals.icu wellness owes.
///
/// Read from the most recent intervals.icu activity that names its recorder:
/// its Garmin device (or `Garmin`, for a Garmin Connect relay naming no
/// device); `None` when that recorder is not Garmin's or no activity names
/// one.
///
/// # Errors
///
/// Returns an error when the activity cache cannot be read.
async fn intervals_wellness_attribution(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: &TenantId,
) -> AppResult<Option<String>> {
    let end = Utc::now();
    let activities = repos
        .activity_cache
        .get_cached_activities(
            user_id,
            tenant,
            Some(INTERVALS_ICU),
            end - Duration::days(DEVICE_LOOKBACK_DAYS),
            end,
            DEVICE_LOOKBACK_ACTIVITIES,
        )
        .await?;
    let latest = activities
        .iter()
        .find(|a| a.device_name().is_some() || source_attribution(a.source()).is_some());
    Ok(latest.and_then(|activity| match activity.device_name() {
        Some(device) if garmin_device(Some(device)) => activity_attribution(None, Some(device)),
        Some(_) => None,
        None => activity_attribution(activity.source(), None),
    }))
}

/// Whether an intervals.icu row is among `records`.
fn serves_intervals<T: StoredHealthRecord>(records: &[T]) -> bool {
    records
        .iter()
        .any(|record| record.source_name().eq_ignore_ascii_case(INTERVALS_ICU))
}

/// The attribution a Garmin data source's rows owe, by the device the source
/// names: as Garmin's own API names it for the Garmin provider, as a relay's
/// activity is attributed otherwise; `None` for a source that is not Garmin's.
fn garmin_source_attribution(source: &DataSource) -> Option<String> {
    let device = source.device_model.as_deref();
    if source.provider.eq_ignore_ascii_case(GARMIN) {
        return Some(garmin_service_attribution(device));
    }
    let relays_garmin = source_attribution(source.source.as_deref()).is_some();
    (relays_garmin || garmin_device(device))
        .then(|| activity_attribution(source.source.as_deref(), device))
        .flatten()
}

/// The attribution of the first of `records` a Garmin data source synced:
/// by its data source among `sources`, or by the provider it was stored
/// under when no gate listed them.
fn garmin_rows_attribution<T: StoredHealthRecord>(
    records: &[T],
    sources: &[DataSource],
) -> Option<String> {
    records.iter().find_map(|record| {
        sources
            .iter()
            .find(|source| source.id == record.data_source_id())
            .map_or_else(
                || source_attribution(Some(record.source_name())).map(str::to_owned),
                garmin_source_attribution,
            )
    })
}

/// The attribution served rows owe, noted on the running derivation for the
/// reply derived from them.
///
/// That of their Garmin data sources (`garmin`, read off the rows before
/// this awaits anything), else that of the athlete's intervals.icu wellness
/// when such a row is among them.
async fn attribution_of(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: &TenantId,
    garmin: Option<String>,
    serves_intervals: bool,
) -> AppResult<Option<String>> {
    let attribution = match garmin {
        Some(attribution) => Some(attribution),
        None if serves_intervals => intervals_wellness_attribution(repos, user_id, tenant).await?,
        None => None,
    };
    if attribution.is_some() {
        ai_scope::note_attribution(Some(GARMIN_ATTRIBUTION));
    }
    Ok(attribution)
}

/// The rows a reader may see of `records`, by each row's data source, with
/// the attribution the served rows owe.
///
/// The athlete's data sources are read once for the whole set, and only when
/// a gate applies and there is a row to govern.
async fn governed<T: StoredHealthRecord>(
    repos: &RepositoryRegistry,
    terms: &dyn ProviderTerms,
    user_id: Uuid,
    tenant: &TenantId,
    records: Vec<T>,
) -> AppResult<StoredHealth<T>> {
    let sources = if records.is_empty() || !ai_scope::policies_apply() {
        Vec::new()
    } else {
        repos
            .data_sources
            .list_data_sources(user_id, tenant)
            .await?
    };
    governed_by_sources(repos, terms, user_id, tenant, records, &sources).await
}

/// [`governed`] for a reader that already holds the athlete's data
/// `sources`, as the continuous-series read does: it lists them to know which
/// series to read.
pub(crate) async fn governed_by_sources<T: StoredHealthRecord>(
    repos: &RepositoryRegistry,
    terms: &dyn ProviderTerms,
    user_id: Uuid,
    tenant: &TenantId,
    records: Vec<T>,
    sources: &[DataSource],
) -> AppResult<StoredHealth<T>> {
    let records = if records.is_empty() || !ai_scope::policies_apply() {
        records
    } else {
        ai_scope::filter_stored_health(terms, records, sources)
    };
    let garmin = garmin_rows_attribution(&records, sources);
    let attribution =
        attribution_of(repos, user_id, tenant, garmin, serves_intervals(&records)).await?;
    Ok(StoredHealth {
        records,
        attribution,
    })
}

/// Which stored rows a merged read takes part in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFilter<'a> {
    /// Every source the gates in force let the reader see, merged.
    All,
    /// Only the rows one source synced (the sleep tools' `sleep_provider`
    /// argument, matched without regard to case), merged.
    Service(&'a str),
    /// Only the rows one linked platform filed, each returned as it was
    /// stored and never merged with another: a coach link reads what that
    /// platform holds, not the athlete's other sources.
    Link(&'a str),
}

impl SourceFilter<'_> {
    /// Whether a row synced by `source_name` takes part.
    fn admits(self, source_name: &str) -> bool {
        match self {
            Self::All => true,
            Self::Service(source) | Self::Link(source) => source_name.eq_ignore_ascii_case(source),
        }
    }
}

/// Narrow, gate and merge stored rows: the shared body of the `merged_*`
/// reads. A [`SourceFilter::Link`] read returns each kept row on its own.
async fn merged<T: StoredHealthRecord>(
    repos: &RepositoryRegistry,
    terms: &dyn ProviderTerms,
    (user_id, tenant): (Uuid, &TenantId),
    rows: Vec<T>,
    filter: SourceFilter<'_>,
    merge: impl FnOnce(Vec<T>) -> Vec<Merged<T>>,
) -> AppResult<StoredHealth<Merged<T>>> {
    let rows: Vec<T> = rows
        .into_iter()
        .filter(|row| filter.admits(row.source_name()))
        .collect();
    let StoredHealth {
        records,
        attribution,
    } = governed(repos, terms, user_id, tenant, rows).await?;
    let records = match filter {
        SourceFilter::Link(_) => records.into_iter().map(unmerged).collect(),
        SourceFilter::All | SourceFilter::Service(_) => merge(records),
    };
    Ok(StoredHealth {
        records,
        attribution,
    })
}

/// One stored row as a merged record of its own source alone.
fn unmerged<T: StoredHealthRecord>(record: T) -> Merged<T> {
    Merged {
        sources: vec![record.source_name().to_owned()],
        filled: Vec::new(),
        record,
    }
}

/// The athlete's nights starting inside the window, one per night across the
/// sources `filter` admits, ordered by start, naps included.
///
/// # Errors
///
/// Returns an error when the sessions, or under a gate the athlete's data
/// sources, cannot be read.
pub async fn merged_sleep(
    repos: &RepositoryRegistry,
    terms: &dyn ProviderTerms,
    user_id: Uuid,
    tenant: &TenantId,
    (start, end): (DateTime<Utc>, DateTime<Utc>),
    filter: SourceFilter<'_>,
) -> AppResult<StoredHealth<Merged<StoredSleepSession>>> {
    let rows = repos
        .sleep
        .get_sleep_sessions(user_id, tenant, start, end)
        .await?;
    merged(
        repos,
        terms,
        (user_id, tenant),
        rows,
        filter,
        merge_sleep_sessions,
    )
    .await
}

/// The athlete's recovery days inside the window, one per date across the
/// sources `filter` admits, ordered by date.
///
/// # Errors
///
/// Returns an error when the readings, or under a gate the athlete's data
/// sources, cannot be read.
pub async fn merged_recovery(
    repos: &RepositoryRegistry,
    terms: &dyn ProviderTerms,
    user_id: Uuid,
    tenant: &TenantId,
    (start, end): (DateTime<Utc>, DateTime<Utc>),
    filter: SourceFilter<'_>,
) -> AppResult<StoredHealth<Merged<StoredRecoveryMetrics>>> {
    let rows = repos
        .recovery
        .get_recovery_metrics(user_id, tenant, start, end)
        .await?;
    merged(
        repos,
        terms,
        (user_id, tenant),
        rows,
        filter,
        merge_recovery_metrics,
    )
    .await
}

/// The athlete's body readings inside the window, one per date across the
/// sources `filter` admits, ordered by date.
///
/// # Errors
///
/// Returns an error when the snapshots, or under a gate the athlete's data
/// sources, cannot be read.
pub async fn merged_body(
    repos: &RepositoryRegistry,
    terms: &dyn ProviderTerms,
    user_id: Uuid,
    tenant: &TenantId,
    (start, end): (DateTime<Utc>, DateTime<Utc>),
    filter: SourceFilter<'_>,
) -> AppResult<StoredHealth<Merged<StoredHealthMetrics>>> {
    let rows = repos
        .health_snapshots
        .get_health_snapshots(user_id, tenant, start, end)
        .await?;
    merged(
        repos,
        terms,
        (user_id, tenant),
        rows,
        filter,
        merge_health_metrics,
    )
    .await
}

/// Whether the athlete holds a stored night or recovery day the gates in
/// force let the reader see, in `[start, end]`.
///
/// Read from the rows rather than from which providers are connected, so an
/// athlete whose wellness arrived by a Garmin push or an export import counts
/// the same as one whose provider syncs it, and a connected provider that
/// never synced a night does not.
///
/// # Errors
///
/// Returns an error when the rows, or under a gate the athlete's data
/// sources, cannot be read.
pub async fn holds_recovery_rows(
    repos: &RepositoryRegistry,
    terms: &dyn ProviderTerms,
    user_id: Uuid,
    tenant: &TenantId,
    (start, end): (DateTime<Utc>, DateTime<Utc>),
) -> AppResult<bool> {
    // Where the rows came from, never the rows: governing a row needs only
    // its provider and data source, and a yes/no serves no reading, so it
    // owes no attribution either.
    let mut origins = repos
        .recovery
        .recovery_origins(user_id, tenant, start, end)
        .await?;
    origins.extend(
        repos
            .sleep
            .sleep_origins(user_id, tenant, start, end)
            .await?,
    );
    if origins.is_empty() || !ai_scope::policies_apply() {
        return Ok(!origins.is_empty());
    }
    let sources = repos
        .data_sources
        .list_data_sources(user_id, tenant)
        .await?;
    Ok(!ai_scope::filter_stored_health(terms, origins, &sources).is_empty())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use chrono::{Duration, NaiveDate, TimeZone};
    use pierre_core::ai_policy::{AiUse, SourcePolicy};
    use pierre_core::models::{ActivityBuilder, DataSource, DeviceType, SportType};
    use pierre_core::transport::{Transport, TransportPolicy};
    use pierre_providers::ai_scope::{ai_read, serve_over, tracking, Provenance};
    use pierre_test_support::db::create_test_db;
    use pierre_test_support::server::create_test_user;

    /// Sleep sessions starting inside the window, as the gates in force let the
    /// reader see them; not yet merged.
    ///
    /// # Errors
    ///
    /// Returns an error when the sessions, or under a gate the athlete's data
    /// sources, cannot be read: a row whose provider cannot be named is never
    /// served.
    async fn sleep_sessions(
        repos: &RepositoryRegistry,
        terms: &dyn ProviderTerms,
        user_id: Uuid,
        tenant: &TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<StoredHealth<StoredSleepSession>> {
        let rows = repos
            .sleep
            .get_sleep_sessions(user_id, tenant, start, end)
            .await?;
        governed(repos, terms, user_id, tenant, rows).await
    }

    /// Recovery readings inside the window, as the gates in force let the reader
    /// see them; not yet merged.
    ///
    /// # Errors
    ///
    /// Returns an error when the readings, or under a gate the athlete's data
    /// sources, cannot be read.
    async fn recovery_metrics(
        repos: &RepositoryRegistry,
        terms: &dyn ProviderTerms,
        user_id: Uuid,
        tenant: &TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    ) -> AppResult<StoredHealth<StoredRecoveryMetrics>> {
        let rows = repos
            .recovery
            .get_recovery_metrics(user_id, tenant, start, end)
            .await?;
        governed(repos, terms, user_id, tenant, rows).await
    }

    /// Terms no shipped provider declares yet: COROS keeps its data first-party
    /// (a `FirstPartyOnly` decision, carnet#724), Zepp bars every item from AI.
    struct Terms;

    const NEVER_AI: SourcePolicy = SourcePolicy {
        direct: AiUse::Deny,
        by_source: &[],
        other_sources: AiUse::Deny,
    };

    impl ProviderTerms for Terms {
        fn ai_policy(&self, provider: &str) -> Option<&'static SourcePolicy> {
            match provider {
                "zepp" => Some(&NEVER_AI),
                "garmin" | "coros" => Some(&SourcePolicy::ALLOW_ALL),
                _ => None,
            }
        }

        fn transport_policy(&self, provider: &str) -> Option<TransportPolicy> {
            match provider {
                "coros" => Some(TransportPolicy::FirstPartyOnly),
                "garmin" | "zepp" => Some(TransportPolicy::AnyTransport),
                _ => None,
            }
        }
    }

    const PROVIDERS: [&str; 3] = ["garmin", "coros", "zepp"];

    /// An athlete whose Garmin, COROS and Zepp sources each synced the same
    /// night and the same morning, each with values of its own.
    struct Athlete {
        repos: Arc<RepositoryRegistry>,
        user_id: Uuid,
        tenant: TenantId,
        start: DateTime<Utc>,
        end: DateTime<Utc>,
    }

    fn resting_hr(provider: &str) -> u32 {
        match provider {
            "garmin" => 48,
            "coros" => 44,
            _ => 40,
        }
    }

    fn night_of(
        user_id: Uuid,
        data_source_id: &str,
        provider: &str,
        night: DateTime<Utc>,
    ) -> StoredSleepSession {
        StoredSleepSession {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            data_source_id: data_source_id.to_owned(),
            is_nap: false,
            start_datetime: night,
            end_datetime: night + Duration::hours(8),
            total_sleep_seconds: Some(7 * 3600),
            deep_sleep_seconds: None,
            light_sleep_seconds: None,
            rem_sleep_seconds: None,
            awake_seconds: None,
            sleep_efficiency: None,
            avg_heart_rate: None,
            min_heart_rate: Some(resting_hr(provider)),
            avg_hrv: None,
            sleep_score: None,
            stages: Vec::new(),
            source_name: provider.to_owned(),
        }
    }

    fn morning_of(
        user_id: Uuid,
        data_source_id: &str,
        provider: &str,
        night: DateTime<Utc>,
    ) -> StoredRecoveryMetrics {
        StoredRecoveryMetrics {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            data_source_id: data_source_id.to_owned(),
            date: NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
            recovery_score: None,
            readiness_score: None,
            hrv_ms: None,
            hrv_rmssd: Some(f64::from(resting_hr(provider)) + 20.0),
            resting_heart_rate: Some(resting_hr(provider)),
            stress_score: None,
            body_battery: None,
            spo2: None,
            respiratory_rate: None,
            skin_temp_deviation: None,
            daily_strain: None,
            athlete_note: None,
            source_name: provider.to_owned(),
            recorded_at: night + Duration::hours(9),
        }
    }

    async fn athlete() -> Athlete {
        let db = create_test_db().await.unwrap();
        let repos = Arc::clone(db.repositories());
        let user = create_test_user(&format!("health-{}@example.com", Uuid::new_v4()), None);
        let user_id = repos.users.create(&user).await.unwrap();
        let tenant = TenantId::generate();
        let night = Utc.with_ymd_and_hms(2026, 9, 29, 22, 0, 0).unwrap();

        for provider in PROVIDERS {
            let data_source_id = repos
                .data_sources
                .upsert_data_source(
                    &tenant,
                    &DataSource {
                        id: String::new(),
                        user_id: user_id.to_string(),
                        provider: provider.to_owned(),
                        device_model: None,
                        software_version: None,
                        source: None,
                        device_type: DeviceType::Watch,
                        original_source_name: None,
                    },
                )
                .await
                .unwrap();
            repos
                .sleep
                .upsert_sleep_session(
                    &tenant,
                    &night_of(user_id, &data_source_id, provider, night),
                )
                .await
                .unwrap();
            repos
                .recovery
                .upsert_recovery_metrics(
                    &tenant,
                    &morning_of(user_id, &data_source_id, provider, night),
                )
                .await
                .unwrap();
        }
        Athlete {
            repos,
            user_id,
            tenant,
            start: night - Duration::days(1),
            end: night + Duration::days(1),
        }
    }

    impl Athlete {
        async fn nights(&self) -> Vec<StoredSleepSession> {
            sleep_sessions(
                &self.repos,
                &Terms,
                self.user_id,
                &self.tenant,
                self.start,
                self.end,
            )
            .await
            .unwrap()
            .records
        }

        async fn mornings(&self) -> Vec<StoredRecoveryMetrics> {
            recovery_metrics(
                &self.repos,
                &Terms,
                self.user_id,
                &self.tenant,
                self.start,
                self.end,
            )
            .await
            .unwrap()
            .records
        }
    }

    fn sorted(sources: &[String]) -> Vec<&str> {
        let mut sources: Vec<&str> = sources.iter().map(String::as_str).collect();
        sources.sort_unstable();
        sources
    }

    #[tokio::test]
    async fn an_external_call_reads_none_of_a_first_party_only_sources_sleep_or_recovery() {
        let athlete = athlete().await;
        let ((nights, mornings), withheld) = serve_over(
            Transport::McpHttp,
            ai_read(async { (athlete.nights().await, athlete.mornings().await) }),
        )
        .await;

        assert!(nights.iter().all(|n| n.source_name != "coros"));
        assert!(mornings.iter().all(|m| m.source_name != "coros"));
        assert_eq!(withheld.off_interface, 2, "COROS's night and morning");

        let night = merge_sleep_sessions(nights);
        assert_eq!(night.len(), 1);
        assert_eq!(sorted(&night[0].sources), vec!["garmin"]);
        let morning = merge_recovery_metrics(mornings);
        assert_eq!(morning.len(), 1);
        assert_eq!(sorted(&morning[0].sources), vec!["garmin"]);
        assert_eq!(
            morning[0].record.resting_heart_rate,
            Some(48),
            "neither COROS nor Zepp fills the served morning"
        );
    }

    #[tokio::test]
    async fn a_model_on_dravrs_own_surface_reads_every_source_but_the_one_barred_from_ai() {
        let athlete = athlete().await;
        let ((nights, mornings), withheld) = serve_over(
            Transport::WebApp,
            ai_read(async { (athlete.nights().await, athlete.mornings().await) }),
        )
        .await;

        let night = merge_sleep_sessions(nights);
        assert_eq!(night.len(), 1, "the permitted sources merge into one night");
        assert_eq!(
            sorted(&night[0].sources),
            vec!["coros", "garmin"],
            "first-party only is served first-party; Zepp never reaches the model"
        );
        let morning = merge_recovery_metrics(mornings);
        assert_eq!(sorted(&morning[0].sources), vec!["coros", "garmin"]);
        assert_eq!(withheld.dropped, 2, "Zepp's night and morning");
        assert_eq!(withheld.off_interface, 0);
    }

    /// An athlete whose intervals.icu account synced one morning of wellness
    /// and one ride recorded on `device`.
    async fn intervals_athlete(device: &str) -> Athlete {
        let db = create_test_db().await.unwrap();
        let repos = Arc::clone(db.repositories());
        let user = create_test_user(&format!("wellness-{}@example.com", Uuid::new_v4()), None);
        let user_id = repos.users.create(&user).await.unwrap();
        let tenant = TenantId::generate();
        let night = Utc.with_ymd_and_hms(2026, 9, 29, 22, 0, 0).unwrap();
        let data_source_id = repos
            .data_sources
            .upsert_data_source(
                &tenant,
                &DataSource {
                    id: String::new(),
                    user_id: user_id.to_string(),
                    provider: INTERVALS_ICU.to_owned(),
                    device_model: None,
                    software_version: None,
                    source: None,
                    device_type: DeviceType::Watch,
                    original_source_name: None,
                },
            )
            .await
            .unwrap();
        repos
            .recovery
            .upsert_recovery_metrics(
                &tenant,
                &morning_of(user_id, &data_source_id, INTERVALS_ICU, night),
            )
            .await
            .unwrap();
        // An older ride on another device: only the most recent recorder counts.
        let older = ActivityBuilder::new(
            "older",
            "Ride",
            SportType::Ride,
            night - Duration::days(20),
            3_600,
            INTERVALS_ICU,
        )
        .device_name("Wahoo ELEMNT BOLT")
        .build();
        let ride =
            ActivityBuilder::new("ride", "Ride", SportType::Ride, night, 3_600, INTERVALS_ICU)
                .source("strava")
                .device_name(device)
                .build();
        repos
            .activity_cache
            .upsert_activities(user_id, &tenant, INTERVALS_ICU, &[older, ride])
            .await
            .unwrap();
        Athlete {
            repos,
            user_id,
            tenant,
            start: night - Duration::days(1),
            end: night + Duration::days(1),
        }
    }

    async fn attributed_mornings(
        athlete: &Athlete,
    ) -> (StoredHealth<StoredRecoveryMetrics>, Provenance) {
        let turn = Provenance::new();
        let (read, _) = ai_read(tracking(
            turn.clone(),
            recovery_metrics(
                &athlete.repos,
                &Terms,
                athlete.user_id,
                &athlete.tenant,
                athlete.start,
                athlete.end,
            ),
        ))
        .await;
        (read.unwrap(), turn)
    }

    #[tokio::test]
    async fn a_garmin_athletes_intervals_wellness_is_attributed_to_their_device() {
        let athlete = intervals_athlete("Garmin Forerunner 965").await;
        let (read, turn) = attributed_mornings(&athlete).await;
        assert_eq!(read.records.len(), 1);
        assert_eq!(read.attribution.as_deref(), Some("Garmin Forerunner 965"));
        assert!(
            turn.attributions().contains(&GARMIN_ATTRIBUTION),
            "the reply derived from it owes the Garmin line"
        );
    }

    #[tokio::test]
    async fn another_devices_athletes_intervals_wellness_carries_no_attribution() {
        let athlete = intervals_athlete("COROS PACE 4").await;
        let (read, turn) = attributed_mornings(&athlete).await;
        assert_eq!(read.records.len(), 1);
        assert_eq!(read.attribution, None);
        assert!(turn.attributions().is_empty());
    }

    /// Wellness a Garmin data source synced owes Garmin's attribution itself,
    /// with no intervals.icu row or device involved (carnet#1154).
    #[tokio::test]
    async fn a_garmin_sources_wellness_is_attributed_to_garmin() {
        let athlete = athlete().await;
        let (read, turn) = attributed_mornings(&athlete).await;
        assert_eq!(read.attribution.as_deref(), Some(GARMIN_ATTRIBUTION));
        assert!(
            turn.attributions().contains(&GARMIN_ATTRIBUTION),
            "the reply derived from it owes the Garmin line"
        );
    }

    fn source_of(
        id: &str,
        provider: &str,
        source: Option<&str>,
        model: Option<&str>,
    ) -> DataSource {
        DataSource {
            id: id.to_owned(),
            user_id: "athlete".to_owned(),
            provider: provider.to_owned(),
            device_model: model.map(str::to_owned),
            software_version: None,
            source: source.map(str::to_owned),
            device_type: DeviceType::Watch,
            original_source_name: None,
        }
    }

    #[test]
    fn a_row_is_attributed_by_the_garmin_data_source_that_synced_it() {
        let night = Utc.with_ymd_and_hms(2026, 9, 29, 22, 0, 0).unwrap();
        let user_id = Uuid::new_v4();
        let sources = [
            source_of("garmin", "garmin", None, Some("Forerunner 965")),
            source_of("nolio-garmin", "nolio", Some("garmin"), None),
            source_of("nolio-edge", "nolio", None, Some("Garmin Edge 840")),
            source_of("nolio-zepp", "nolio", Some("zepp"), Some("Zepp Amazfit")),
            source_of("coros", "coros", None, Some("PACE 3")),
        ];
        for (id, stored_under, expected) in [
            ("garmin", "garmin", Some("Garmin Forerunner 965")),
            ("nolio-garmin", "nolio", Some("Garmin")),
            ("nolio-edge", "nolio", Some("Garmin Edge 840")),
            ("nolio-zepp", "nolio", None),
            ("coros", "coros", None),
            // No gate listed the sources: the provider it was stored under.
            ("unlisted", "garmin", Some("Garmin")),
            ("unlisted", "coros", None),
        ] {
            let row = morning_of(user_id, id, stored_under, night);
            assert_eq!(
                garmin_rows_attribution(&[row], &sources).as_deref(),
                expected,
                "{id} stored under {stored_under}"
            );
        }
    }

    #[tokio::test]
    async fn the_athletes_own_routes_read_every_row() {
        let athlete = athlete().await;
        assert_eq!(athlete.nights().await.len(), PROVIDERS.len());
        assert_eq!(athlete.mornings().await.len(), PROVIDERS.len());
    }

    impl Athlete {
        async fn merged_mornings(
            &self,
            filter: SourceFilter<'_>,
        ) -> Vec<Merged<StoredRecoveryMetrics>> {
            merged_recovery(
                &self.repos,
                &Terms,
                self.user_id,
                &self.tenant,
                (self.start, self.end),
                filter,
            )
            .await
            .unwrap()
            .records
        }

        async fn merged_nights(&self, filter: SourceFilter<'_>) -> Vec<Merged<StoredSleepSession>> {
            merged_sleep(
                &self.repos,
                &Terms,
                self.user_id,
                &self.tenant,
                (self.start, self.end),
                filter,
            )
            .await
            .unwrap()
            .records
        }
    }

    /// Every source's morning merges into one record; a named source narrows
    /// the merge to its own rows, matched without regard to case.
    #[tokio::test]
    async fn a_merged_read_merges_every_source_or_only_the_one_named() {
        let athlete = athlete().await;

        let all = athlete.merged_mornings(SourceFilter::All).await;
        assert_eq!(all.len(), 1, "one morning across three sources");
        assert_eq!(
            sorted(&all[0].sources),
            vec!["coros", "garmin", "zepp"],
            "every source takes part"
        );

        let garmin = athlete
            .merged_mornings(SourceFilter::Service("Garmin"))
            .await;
        assert_eq!(garmin.len(), 1, "one Garmin morning");
        assert_eq!(garmin[0].sources, vec!["garmin".to_owned()], "Garmin alone");
        assert_eq!(
            garmin[0].record.resting_heart_rate,
            Some(48),
            "Garmin's value"
        );
        assert_eq!(garmin[0].filled.len(), 0, "nothing else took part");

        let nights = athlete.merged_nights(SourceFilter::All).await;
        assert_eq!(nights.len(), 1, "one night across three sources");
        assert_eq!(
            athlete
                .merged_nights(SourceFilter::Service("polar"))
                .await
                .len(),
            0
        );
    }

    /// A link's read is the rows its platform filed, each as stored: never
    /// merged with the athlete's other sources, never filled from them.
    #[tokio::test]
    async fn a_link_read_returns_its_own_rows_unmerged() {
        let athlete = athlete().await;
        let coros = athlete.merged_mornings(SourceFilter::Link("coros")).await;
        assert_eq!(coros.len(), 1, "the link's one morning");
        assert_eq!(coros[0].sources, vec!["coros".to_owned()], "the link alone");
        assert_eq!(coros[0].record.resting_heart_rate, Some(44), "as stored");
        assert_eq!(coros[0].filled.len(), 0, "a link read fills nothing");

        let nights = athlete.merged_nights(SourceFilter::Link("zepp")).await;
        assert_eq!(nights.len(), 1, "the link's one night");
        assert_eq!(nights[0].record.source_name, "zepp");
    }

    /// The merged read applies each row's terms before it merges, as the raw
    /// read does: an external call never sees a first-party-only source's
    /// metric folded into the served morning.
    #[tokio::test]
    async fn a_merged_read_gates_each_row_before_the_merge() {
        let athlete = athlete().await;
        let (mornings, withheld) = serve_over(
            Transport::McpHttp,
            ai_read(athlete.merged_mornings(SourceFilter::All)),
        )
        .await;
        assert_eq!(mornings.len(), 1, "one served morning");
        assert_eq!(
            sorted(&mornings[0].sources),
            vec!["garmin"],
            "Garmin alone is served"
        );
        assert_eq!(
            mornings[0].record.resting_heart_rate,
            Some(48),
            "no gated source fills it"
        );
        assert_eq!(withheld.off_interface, 1, "COROS's morning");
        assert_eq!(withheld.dropped, 1, "Zepp's morning");
    }

    #[tokio::test]
    async fn an_athlete_with_a_stored_morning_holds_recovery_rows() {
        let athlete = athlete().await;
        let inside = holds_recovery_rows(
            &athlete.repos,
            &Terms,
            athlete.user_id,
            &athlete.tenant,
            (athlete.start, athlete.end),
        )
        .await
        .unwrap();
        assert!(inside, "the stored night and morning sit in the window");
        let before = athlete.start - Duration::days(30);
        let earlier = holds_recovery_rows(
            &athlete.repos,
            &Terms,
            athlete.user_id,
            &athlete.tenant,
            (before - Duration::days(10), before),
        )
        .await
        .unwrap();
        assert!(!earlier, "nothing is stored a month before");
    }

    /// Asking whether rows are there serves none of them: no attribution is
    /// owed by the reply, which shows no Garmin reading.
    #[tokio::test]
    async fn holding_recovery_rows_notes_no_attribution() {
        let athlete = athlete().await;
        let turn = Provenance::new();
        let (holds, _) = ai_read(tracking(
            turn.clone(),
            holds_recovery_rows(
                &athlete.repos,
                &Terms,
                athlete.user_id,
                &athlete.tenant,
                (athlete.start, athlete.end),
            ),
        ))
        .await;
        assert!(holds.unwrap());
        assert!(
            turn.attributions().is_empty(),
            "a yes/no owes no Garmin line"
        );
    }

    /// Rows a gate keeps from the reader do not make it a recovery source.
    #[tokio::test]
    async fn rows_no_gate_lets_through_hold_no_recovery_source() {
        let db = create_test_db().await.unwrap();
        let repos = Arc::clone(db.repositories());
        let user = create_test_user(&format!("zepp-{}@example.com", Uuid::new_v4()), None);
        let user_id = repos.users.create(&user).await.unwrap();
        let tenant = TenantId::generate();
        let night = Utc.with_ymd_and_hms(2026, 9, 29, 22, 0, 0).unwrap();
        let data_source_id = repos
            .data_sources
            .upsert_data_source(
                &tenant,
                &DataSource {
                    id: String::new(),
                    user_id: user_id.to_string(),
                    provider: "zepp".to_owned(),
                    device_model: None,
                    software_version: None,
                    source: None,
                    device_type: DeviceType::Watch,
                    original_source_name: None,
                },
            )
            .await
            .unwrap();
        repos
            .sleep
            .upsert_sleep_session(&tenant, &night_of(user_id, &data_source_id, "zepp", night))
            .await
            .unwrap();
        repos
            .recovery
            .upsert_recovery_metrics(
                &tenant,
                &morning_of(user_id, &data_source_id, "zepp", night),
            )
            .await
            .unwrap();
        let window = (night - Duration::days(1), night + Duration::days(1));

        let own = holds_recovery_rows(&repos, &Terms, user_id, &tenant, window)
            .await
            .unwrap();
        assert!(own, "the athlete's own surfaces see Zepp's rows");
        let (model, _) = ai_read(holds_recovery_rows(
            &repos, &Terms, user_id, &tenant, window,
        ))
        .await;
        assert!(!model.unwrap(), "Zepp bars every item from AI");
    }
}
