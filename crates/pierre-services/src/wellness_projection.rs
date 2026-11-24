// ABOUTME: The one projection of a batch of wellness records into the platform's stores, for every ingress that hands one
// ABOUTME: Records the data source, stamps owner and key, writes each kind in its own mode, then SDNN and the T2–T3 records

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Projecting wellness records
//!
//! An archive import (barrage's sink), a Terra webhook and a phone's health
//! store sync (`/api/device-health`) each hand the platform a batch of nights
//! and days. [`project`] files every such batch:
//!
//! 1. the batch's data source is recorded ([`WellnessStore::record_data_source`]);
//! 2. every record is filed for the athlete, under that data source and the
//!    provider key as its `source_name`, whatever the record named;
//! 3. nights are written whole; recovery days, body days and daily activity
//!    totals each in the [`WellnessWriteMode`] the ingress names
//!    ([`WellnessModes`]): a partial summary when a day arrives in pieces, a
//!    full day when the batch holds the whole day;
//! 4. readings measured as SDNN go to their own series, since a day's HRV is
//!    read as rMSSD;
//! 5. the T2–T3 records go through [`store_sensitive`], whatever the athlete
//!    allowed: the tier consent gates who else reads them (carnet#1544).
//!
//! Every write goes through the [`WellnessStore`], so WHOOP's terms apply here
//! as at every other door. Garmin's push keeps its own path: it writes
//! Garmin's wire summaries one kind at a time and holds no batch.

use dravr_barrage::WellnessRecords;
use dravr_tronc::dates::Timestamp;
use pierre_core::errors::AppResult;
use pierre_core::models::{DataSource, TenantId};
use pierre_database::repositories::RecordFiling;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

use crate::health_records::{store_sensitive, SensitiveStored};
use crate::wellness_store::{IntradaySeries, WellnessLanded, WellnessStore, WellnessWriteMode};

/// How each kind of day record is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WellnessModes {
    /// A day's recovery readings.
    pub recovery: WellnessWriteMode,
    /// A day's body measurements.
    pub body: WellnessWriteMode,
    /// A day's activity totals.
    pub daily_activity: WellnessWriteMode,
}

/// Whose records a batch is filed as, under which key and data source, and
/// how its days are written.
#[derive(Debug, Clone)]
pub struct WellnessFiling<'filing> {
    /// The athlete's tenant.
    pub tenant_id: TenantId,
    /// The athlete.
    pub user_id: Uuid,
    /// The provider key every record is filed under, as its `source_name`.
    pub provider: &'filing str,
    /// The data source the batch came from, recorded (or refreshed) first.
    pub data_source: DataSource,
    /// The write mode of each kind of day record.
    pub modes: WellnessModes,
    /// What the batch is, named in the log of what WHOOP's terms withheld.
    pub label: &'static str,
}

/// What one batch landed as.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ProjectedCounts {
    /// Nights written (one a later receipt already holds is not).
    pub nights: usize,
    /// Recovery days written.
    pub recovery: usize,
    /// Body days written.
    pub body: usize,
    /// Daily activity totals written.
    pub daily_activity: usize,
    /// What the T2–T3 records wrote and dropped.
    pub sensitive: SensitiveStored,
}

/// The owner, data source and key a filed record is stamped with.
struct Stamp<'filing> {
    user: String,
    data_source_id: &'filing str,
    key: &'filing str,
}

impl Stamp<'_> {
    /// Stamp a record's owner, data source and key.
    fn stamp(&self, user_id: &mut String, data_source_id: &mut String, source_name: &mut String) {
        self.user.clone_into(user_id);
        self.data_source_id.clone_into(data_source_id);
        self.key.clone_into(source_name);
    }
}

/// One when the store wrote the record.
fn written(landed: WellnessLanded) -> usize {
    usize::from(landed == WellnessLanded::Written)
}

/// Store `wellness`, received at `at`, as `filing` says. An empty batch
/// writes nothing, not even its data source.
///
/// # Errors
///
/// The first store error; what was written before it stays.
pub async fn project(
    repos: &RepositoryRegistry,
    filing: WellnessFiling<'_>,
    wellness: &WellnessRecords,
    at: Timestamp,
) -> AppResult<ProjectedCounts> {
    let mut counts = ProjectedCounts::default();
    if wellness.is_empty() {
        return Ok(counts);
    }
    let WellnessFiling {
        tenant_id,
        user_id,
        provider,
        data_source,
        modes,
        label,
    } = filing;
    let store = WellnessStore::new(repos);
    let source_id = store.record_data_source(&tenant_id, &data_source).await?;
    let own = Stamp {
        user: user_id.to_string(),
        data_source_id: &source_id,
        key: provider,
    };
    let mut writes = store.writes(label);
    for night in &wellness.sleep {
        let mut night = night.clone();
        own.stamp(
            &mut night.user_id,
            &mut night.data_source_id,
            &mut night.source_name,
        );
        counts.nights += written(writes.sleep(tenant_id, &night, at).await?);
    }
    for day in &wellness.recovery {
        let mut day = day.clone();
        own.stamp(
            &mut day.user_id,
            &mut day.data_source_id,
            &mut day.source_name,
        );
        counts.recovery += written(writes.recovery(tenant_id, &day, modes.recovery, at).await?);
    }
    for day in &wellness.body {
        let mut day = day.clone();
        own.stamp(
            &mut day.user_id,
            &mut day.data_source_id,
            &mut day.source_name,
        );
        counts.body += written(writes.body(tenant_id, &day, modes.body, at).await?);
    }
    for day in &wellness.daily_activity {
        let mut day = day.clone();
        own.stamp(
            &mut day.user_id,
            &mut day.data_source_id,
            &mut day.source_name,
        );
        counts.daily_activity += written(
            writes
                .daily_activity(tenant_id, &day, modes.daily_activity, at)
                .await?,
        );
    }
    // SDNN keeps its own series: a day's HRV is read as rMSSD.
    store
        .store_series(
            &source_id,
            IntradaySeries::hrv_sdnn(
                wellness
                    .hrv_sdnn
                    .iter()
                    .map(|reading| (reading.at, reading.milliseconds)),
            ),
        )
        .await?;
    writes.report();
    if wellness.holds_sensitive() {
        counts.sensitive = store_sensitive(
            repos,
            RecordFiling {
                tenant_id: &tenant_id,
                user_id,
                provider,
                data_source_id: &source_id,
            },
            wellness,
        )
        .await?;
    }
    Ok(counts)
}

#[cfg(test)]
mod tests {
    use dravr_tronc::dates::{CivilDate, CivilDateTime, Elapsed};
    use pierre_core::models::{DeviceType, StoredSleepSession, User};
    use pierre_test_support::db::create_test_db;

    use super::*;

    const KEY: &str = "terra";

    fn night(start: Timestamp) -> StoredSleepSession {
        StoredSleepSession {
            id: String::new(),
            user_id: "someone-else".to_owned(),
            data_source_id: "not-recorded".to_owned(),
            is_nap: false,
            start_datetime: start,
            end_datetime: start + Elapsed::hours(8),
            total_sleep_seconds: Some(25_200),
            deep_sleep_seconds: None,
            light_sleep_seconds: None,
            rem_sleep_seconds: None,
            awake_seconds: Some(1_800),
            sleep_efficiency: None,
            avg_heart_rate: None,
            min_heart_rate: None,
            avg_hrv: None,
            sleep_score: None,
            avg_spo2: None,
            stages: Vec::new(),
            source_name: "the recorder's own name".to_owned(),
        }
    }

    fn filing(tenant_id: TenantId, user_id: Uuid) -> WellnessFiling<'static> {
        WellnessFiling {
            tenant_id,
            user_id,
            provider: KEY,
            data_source: DataSource {
                id: Uuid::new_v4().to_string(),
                user_id: user_id.to_string(),
                provider: KEY.to_owned(),
                device_model: None,
                software_version: None,
                source: Some("oura".to_owned()),
                device_type: DeviceType::Unknown,
                original_source_name: Some("oura".to_owned()),
            },
            modes: WellnessModes {
                recovery: WellnessWriteMode::PartialSummary,
                body: WellnessWriteMode::PartialSummary,
                daily_activity: WellnessWriteMode::FullDay,
            },
            label: "projection test",
        }
    }

    #[tokio::test]
    async fn a_batch_is_filed_for_the_athlete_under_the_key_and_counts_what_landed() {
        let db = create_test_db().await.expect("test db"); // Safe: test assertion
        let repos = db.repositories();
        let user = User::new(
            format!("projection-{}@example.com", Uuid::new_v4()),
            "argon2-hash-placeholder".to_owned(),
            None,
        );
        let user_id = repos.users.create(&user).await.expect("user"); // Safe: test assertion
        let tenant_id = TenantId::from_uuid(Uuid::new_v4());
        let start = CivilDate::from_ymd(2026, 11, 20)
            .and_then(|day| day.and_hms(23, 0, 0))
            .and_then(CivilDateTime::and_utc)
            .expect("a valid instant"); // Safe: test assertion // Safe: test assertion
        let wellness = WellnessRecords {
            sleep: vec![night(start)],
            ..WellnessRecords::default()
        };
        let received = start + Elapsed::days(1);

        let counts = project(repos, filing(tenant_id, user_id), &wellness, received)
            .await
            .expect("projected"); // Safe: test assertion
        assert_eq!(
            counts,
            ProjectedCounts {
                nights: 1,
                ..ProjectedCounts::default()
            },
            "bug: one night written, nothing else"
        );
        let sources = repos
            .data_sources
            .list_data_sources(user_id, &tenant_id)
            .await
            .expect("sources"); // Safe: test assertion
        assert_eq!(sources.len(), 1, "bug: the batch's one data source");
        let stored = repos
            .sleep
            .get_sleep_sessions(user_id, &tenant_id, start, start)
            .await
            .expect("nights"); // Safe: test assertion
        assert_eq!(
            stored.len(),
            1,
            "bug: the night is read back for the athlete"
        );
        assert_eq!(
            (
                stored[0].source_name.as_str(),
                stored[0].data_source_id.as_str()
            ),
            (KEY, sources[0].id.as_str()),
            "bug: the night is filed under the key and the recorded source, not what it named"
        );

        let earlier = received - Elapsed::hours(1);
        let again = project(repos, filing(tenant_id, user_id), &wellness, earlier)
            .await
            .expect("projected again"); // Safe: test assertion
        assert_eq!(
            again.nights, 0,
            "bug: a night superseded by a later receipt was counted as written"
        );
    }

    #[tokio::test]
    async fn an_empty_batch_records_no_data_source() {
        let db = create_test_db().await.expect("test db"); // Safe: test assertion
        let repos = db.repositories();
        let (tenant_id, user_id) = (TenantId::from_uuid(Uuid::new_v4()), Uuid::new_v4());

        let counts = project(
            repos,
            filing(tenant_id, user_id),
            &WellnessRecords::default(),
            Timestamp::now(),
        )
        .await
        .expect("projected"); // Safe: test assertion

        assert_eq!(
            counts,
            ProjectedCounts::default(),
            "bug: an empty batch wrote"
        );
        assert!(
            repos
                .data_sources
                .list_data_sources(user_id, &tenant_id)
                .await
                .expect("sources") // Safe: test assertion
                .is_empty(),
            "bug: an empty batch recorded a data source"
        );
    }
}
