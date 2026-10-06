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
//! # Garmin attribution of intervals.icu wellness (carnet#521)
//!
//! intervals.icu's wellness rows name no source or device, while its API terms
//! bind Garmin-sourced data to Garmin's attribution. A row is attributed by the
//! athlete's devices instead: when the most recent intervals.icu activity that
//! names its recorder was recorded on a Garmin device, the athlete's
//! intervals.icu wellness is attributed to that device. The device is read
//! from the activity cache, once per read and only when an intervals.icu row
//! is served — never from the provider.

use chrono::{DateTime, Duration, Utc};
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::constants::oauth_providers::{
    activity_attribution, garmin_device, source_attribution, GARMIN_ATTRIBUTION, INTERVALS_ICU,
};
use pierre_core::errors::AppResult;
use pierre_core::models::{
    StoredHealthMetrics, StoredRecoveryMetrics, StoredSleepSession, TenantId,
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
    /// The records the gates in force let the reader see; not yet merged.
    pub records: Vec<T>,
    /// The attribution to show with them when an intervals.icu row is among
    /// them and the athlete's most recent intervals.icu recording was
    /// Garmin's: its device (`Garmin Forerunner 965`), or `Garmin` for a
    /// Garmin Connect relay naming no device; `None` otherwise.
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

/// The attribution served rows owe, noted on the running derivation for the
/// reply derived from them: none unless an intervals.icu row is among them.
async fn attribution_of(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: &TenantId,
    serves_intervals: bool,
) -> AppResult<Option<String>> {
    if !serves_intervals {
        return Ok(None);
    }
    let attribution = intervals_wellness_attribution(repos, user_id, tenant).await?;
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
    let records = if records.is_empty() || !ai_scope::policies_apply() {
        records
    } else {
        let sources = repos
            .data_sources
            .list_data_sources(user_id, tenant)
            .await?;
        ai_scope::filter_stored_health(terms, records, &sources)
    };
    let attribution = attribution_of(repos, user_id, tenant, serves_intervals(&records)).await?;
    Ok(StoredHealth {
        records,
        attribution,
    })
}

/// Sleep sessions starting inside the window, as the gates in force let the
/// reader see them; not yet merged.
///
/// # Errors
///
/// Returns an error when the sessions, or under a gate the athlete's data
/// sources, cannot be read: a row whose provider cannot be named is never
/// served.
pub async fn sleep_sessions(
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
pub async fn recovery_metrics(
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

/// Health snapshots inside the window, as the gates in force let the reader
/// see them; not yet merged.
///
/// # Errors
///
/// Returns an error when the snapshots, or under a gate the athlete's data
/// sources, cannot be read.
pub async fn health_snapshots(
    repos: &RepositoryRegistry,
    terms: &dyn ProviderTerms,
    user_id: Uuid,
    tenant: &TenantId,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> AppResult<StoredHealth<StoredHealthMetrics>> {
    let rows = repos
        .health_snapshots
        .get_health_snapshots(user_id, tenant, start, end)
        .await?;
    governed(repos, terms, user_id, tenant, rows).await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use chrono::{Duration, NaiveDate, TimeZone};
    use pierre_core::ai_policy::{AiUse, SourcePolicy};
    use pierre_core::models::{
        merge_recovery_metrics, merge_sleep_sessions, ActivityBuilder, DataSource, DeviceType,
        SportType,
    };
    use pierre_core::transport::{Transport, TransportPolicy};
    use pierre_providers::ai_scope::{ai_read, serve_over, tracking, Provenance};
    use pierre_test_support::db::create_test_db;
    use pierre_test_support::server::create_test_user;

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

    #[tokio::test]
    async fn wellness_from_another_provider_is_never_attributed_by_intervals_devices() {
        let athlete = athlete().await;
        let (read, _) = attributed_mornings(&athlete).await;
        assert_eq!(read.attribution, None);
    }

    #[tokio::test]
    async fn the_athletes_own_routes_read_every_row() {
        let athlete = athlete().await;
        assert_eq!(athlete.nights().await.len(), PROVIDERS.len());
        assert_eq!(athlete.mornings().await.len(), PROVIDERS.len());
    }
}
