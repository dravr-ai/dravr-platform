// ABOUTME: The Garmin attribution stored wellness rows owe: by their Garmin data source, else the intervals.icu device
// ABOUTME: Noted on the running derivation for the reply derived from the rows; read once per governed read

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The attribution half of [`super`]'s governed reads (carnet#521,
//! carnet#1154): which Garmin attribution the rows a reader is served owe.

use chrono::{Duration, Utc};
use pierre_core::constants::oauth_providers::{
    activity_attribution, garmin_device, garmin_service_attribution, source_attribution, GARMIN,
    GARMIN_ATTRIBUTION, INTERVALS_ICU,
};
use pierre_core::errors::AppResult;
use pierre_core::models::{DataSource, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope::{self, StoredHealthRecord};
use uuid::Uuid;

/// How far back the athlete's intervals.icu activities are searched for the
/// device that names their wellness recorder.
const DEVICE_LOOKBACK_DAYS: i64 = 365;

/// How many of those activities, newest first, are searched.
const DEVICE_LOOKBACK_ACTIVITIES: i64 = 50;

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
pub(super) fn serves_intervals<T: StoredHealthRecord>(records: &[T]) -> bool {
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
pub(super) fn garmin_rows_attribution<T: StoredHealthRecord>(
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
pub(super) async fn attribution_of(
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

#[cfg(test)]
mod tests {
    use chrono::NaiveDate;
    use pierre_core::models::{DeviceType, StoredRecoveryMetrics};

    use super::*;

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

    /// A morning the data source `data_source_id` synced, stored under the
    /// provider `stored_under`.
    fn morning_of(data_source_id: &str, stored_under: &str) -> StoredRecoveryMetrics {
        StoredRecoveryMetrics {
            id: Uuid::new_v4().to_string(),
            user_id: Uuid::new_v4().to_string(),
            data_source_id: data_source_id.to_owned(),
            date: NaiveDate::from_ymd_opt(2026, 9, 30).unwrap(),
            recovery_score: None,
            readiness_score: None,
            hrv_ms: None,
            hrv_rmssd: Some(68.0),
            resting_heart_rate: Some(48),
            stress_score: None,
            body_battery: None,
            spo2: None,
            respiratory_rate: None,
            skin_temp_deviation: None,
            daily_strain: None,
            athlete_note: None,
            source_name: stored_under.to_owned(),
            recorded_at: Utc::now(),
        }
    }

    #[test]
    fn a_row_is_attributed_by_the_garmin_data_source_that_synced_it() {
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
            let row = morning_of(id, stored_under);
            assert_eq!(
                garmin_rows_attribution(&[row], &sources).as_deref(),
                expected,
                "{id} stored under {stored_under}"
            );
        }
    }
}
