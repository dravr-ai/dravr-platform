// ABOUTME: Stored continuous heart-rate and HRV series read through riviere's TimeSeriesStore query and aggregate
// ABOUTME: Scoped to the athlete's own data sources and governed by each source's provider terms

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Stored continuous series (carnet#999).
//!
//! The sync writes a provider's intraday samples into `data_point_series`
//! through riviere's [`TimeSeriesStore`](dravr_riviere::TimeSeriesStore),
//! keyed on the data source that synced them and a riviere [`SeriesType`].
//! That table carries no user or tenant column: a series belongs to whoever
//! owns its data source. So this read never takes a data-source id from its
//! caller. It lists the athlete's own sources in the tenant, and reads only
//! those.
//!
//! Each source's series is then governed through [`crate::stored_health`]
//! exactly as stored sleep, recovery and snapshot rows are (carnet#771): by
//! that source's provider and upstream source, with the Garmin attribution an
//! intervals.icu series owes (carnet#521). Series from different sources are
//! never merged: two devices' samples at the same instant are two readings.

use chrono::{DateTime, Utc};
use dravr_riviere::{AggregatedPoint, Aggregation, DataPoint, RiviereError, SeriesType, TimeRange};
use pierre_core::ai_policy::ProviderTerms;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope::StoredHealthRecord;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::stored_health::{governed_by_sources, StoredHealth};
use crate::wellness_store::INTRADAY_SERIES;

/// The continuous series an athlete-facing read may ask for: intraday heart
/// rate and the two HRV measures, every series the store takes.
pub const CONTINUOUS_METRICS: [SeriesType; 3] = INTRADAY_SERIES;

/// The series named `name` (riviere's `snake_case` name, `heart_rate`), when
/// it is one of [`CONTINUOUS_METRICS`].
#[must_use]
pub fn continuous_metric(name: &str) -> Option<SeriesType> {
    CONTINUOUS_METRICS
        .into_iter()
        .find(|series| series.as_str() == name)
}

/// How a series is read: every stored point, or one value per window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesRead {
    /// Every stored point in the range, ascending.
    Points,
    /// One aggregated value per window of `window_secs` seconds starting at
    /// the range's start; a window with no sample is left out.
    Windows {
        /// Width of each window, in seconds.
        window_secs: i64,
        /// How a window's samples fold into its value.
        aggregation: Aggregation,
    },
}

/// One data source's series over the range.
///
/// Exactly one of `points` and `windows` is filled, by the [`SeriesRead`]
/// asked for. A source with nothing in the range is not returned at all.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct SourceSeries {
    /// The data source that synced the series.
    pub data_source_id: String,
    /// The provider the data source syncs from (`whoop`, `garmin`).
    pub provider: String,
    /// The stored samples, ascending; filled for a points read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<DataPoint>,
    /// One value per window holding a sample; filled for a windowed read.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub windows: Vec<AggregatedPoint>,
}

impl SourceSeries {
    /// How many points or windows this source carries.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.points.len() + self.windows.len()
    }

    /// Whether this source carries neither a point nor a window.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl StoredHealthRecord for SourceSeries {
    fn data_source_id(&self) -> &str {
        &self.data_source_id
    }

    fn source_name(&self) -> &str {
        &self.provider
    }
}

/// The store's error as a database error naming the read.
fn store_error(series: SeriesType, e: &RiviereError) -> AppError {
    AppError::database(format!("read {} series: {e}", series.as_str()))
}

/// The athlete's `series` over `[start, end)`, one entry per data source of
/// theirs that holds a sample there, as the gates in force let the reader
/// see them, with the attribution the served series owe.
///
/// # Errors
///
/// Returns an error when the athlete's data sources or a source's series
/// cannot be read.
pub async fn continuous_series(
    repos: &RepositoryRegistry,
    terms: &dyn ProviderTerms,
    user_id: Uuid,
    tenant: &TenantId,
    series: SeriesType,
    (start, end): (DateTime<Utc>, DateTime<Utc>),
    read: SeriesRead,
) -> AppResult<StoredHealth<SourceSeries>> {
    let sources = repos
        .data_sources
        .list_data_sources(user_id, tenant)
        .await?;
    let range = TimeRange::new(start, end);
    let store = &repos.time_series_points;

    let mut found = Vec::new();
    for source in &sources {
        let (points, windows) = match read {
            SeriesRead::Points => {
                let result = store
                    .query(&source.id, series.id(), &range)
                    .await
                    .map_err(|e| store_error(series, &e))?;
                (result.points, Vec::new())
            }
            SeriesRead::Windows {
                window_secs,
                aggregation,
            } => {
                let windows = store
                    .aggregate(&source.id, series.id(), &range, window_secs, aggregation)
                    .await
                    .map_err(|e| store_error(series, &e))?;
                (Vec::new(), windows)
            }
        };
        let source_series = SourceSeries {
            data_source_id: source.id.clone(),
            provider: source.provider.clone(),
            points,
            windows,
        };
        if !source_series.is_empty() {
            found.push(source_series);
        }
    }

    governed_by_sources(repos, terms, user_id, tenant, found, &sources).await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use chrono::{Duration, TimeZone};
    use pierre_core::ai_policy::SourcePolicy;
    use pierre_core::constants::oauth_providers::INTERVALS_ICU;
    use pierre_core::models::{ActivityBuilder, DataSource, DeviceType, SportType};
    use pierre_core::transport::TransportPolicy;
    use pierre_test_support::db::create_test_db;
    use pierre_test_support::server::create_test_user;

    /// No provider declares terms here: the read is governed for attribution
    /// alone.
    struct NoTerms;

    impl ProviderTerms for NoTerms {
        fn ai_policy(&self, _provider: &str) -> Option<&'static SourcePolicy> {
            None
        }

        fn transport_policy(&self, _provider: &str) -> Option<TransportPolicy> {
            None
        }
    }

    /// An athlete whose intervals.icu source stored an hour of heart rate,
    /// with one intervals.icu ride recorded on `device`.
    async fn intervals_series(
        device: &str,
    ) -> (Arc<RepositoryRegistry>, Uuid, TenantId, DateTime<Utc>) {
        let db = create_test_db().await.unwrap();
        let repos = Arc::clone(db.repositories());
        let user = create_test_user(&format!("series-{}@example.com", Uuid::new_v4()), None);
        let user_id = repos.users.create(&user).await.unwrap();
        let tenant = TenantId::generate();
        let morning = Utc.with_ymd_and_hms(2026, 10, 12, 6, 0, 0).unwrap();
        let source_id = repos
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
            .time_series_points
            .insert_batch(
                &source_id,
                SeriesType::HeartRate.id(),
                vec![
                    DataPoint::new(morning, 52.0),
                    DataPoint::new(morning + Duration::minutes(30), 58.0),
                ],
            )
            .await
            .unwrap();
        let ride = ActivityBuilder::new(
            "ride",
            "Ride",
            SportType::Ride,
            morning - Duration::days(1),
            3_600,
            INTERVALS_ICU,
        )
        .device_name(device)
        .build();
        repos
            .activity_cache
            .upsert_activities(user_id, &tenant, INTERVALS_ICU, &[ride])
            .await
            .unwrap();
        (repos, user_id, tenant, morning)
    }

    async fn read_hour(device: &str) -> StoredHealth<SourceSeries> {
        let (repos, user_id, tenant, morning) = intervals_series(device).await;
        continuous_series(
            &repos,
            &NoTerms,
            user_id,
            &tenant,
            SeriesType::HeartRate,
            (morning, morning + Duration::hours(1)),
            SeriesRead::Points,
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn a_garmin_athletes_intervals_series_is_attributed_to_their_device() {
        let read = read_hour("Garmin Forerunner 965").await;
        assert_eq!(read.records.len(), 1);
        assert_eq!(read.records[0].points.len(), 2);
        assert_eq!(read.attribution.as_deref(), Some("Garmin Forerunner 965"));
    }

    #[tokio::test]
    async fn another_devices_intervals_series_carries_no_attribution() {
        let read = read_hour("COROS PACE 4").await;
        assert_eq!(read.records.len(), 1);
        assert_eq!(read.attribution, None);
    }
}
