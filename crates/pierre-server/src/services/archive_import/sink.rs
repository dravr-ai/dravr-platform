// ABOUTME: Barrage's ProjectionSink in the platform: imported records into cached_activities, wellness into the wellness store
// ABOUTME: Samples first (origin='archive', the original's SHA-256 as source_ref), then the rows, bests and list refresh

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Serving what an import stored
//!
//! dravr-barrage keeps the imported records and their originals; the platform
//! serves them the way it serves every provider's activities, so every tool
//! reads an imported history unchanged. Each chunk barrage commits is
//! projected here, idempotently (a retried chunk projects the same rows
//! again):
//!
//! 1. each record's decoded samples into the one per-activity sample store,
//!    `activity_streams`, with `origin = 'archive'`, the source key as the
//!    provider and the original's SHA-256 as `source_ref`, at the format
//!    version barrage decoded them to — before the row they belong to, so a
//!    cached archive activity is never read without its samples;
//! 2. the records' activities into the activity cache under the source key
//!    (`strava_archive`), which no provider sync prunes;
//! 3. the best efforts each run with samples sets, recorded without an
//!    announcement (an import is history, not news);
//! 4. the gear the export named for each run, walk or hike, attributed in the
//!    athlete's gear ledger unless the workout already holds a pair
//!    ([`attribute_imported_gear`]);
//! 5. once per chunk, the agent's cached lists dropped and the training-load
//!    rollup recomputed.
//!
//! The wellness an export holds (the Garmin archive's nights and days) is
//! written through the one wellness writer, [`WellnessStore`], under the
//! source key and a data source named for it, as rows every merged read
//! sees: a night whole, each day record as a partial summary, since each
//! file of the export carries only some of a day's readings and none
//! retracts another's. Retracting a source deletes every row the key
//! contributed, its samples and wellness included.

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use dravr_barrage::cageux::models::TimeSeriesData;
use dravr_barrage::{
    ActivityId, ActivityRecord, BarrageError, ProjectionSink, Scope, SourceKey, WellnessRecords,
};
use pierre_core::errors::AppError;
use pierre_core::models::{Activity, DataSource, DeviceType, TenantId};
use pierre_database::repositories::{ActivityStreamsWrite, PurgeReason, StreamOrigin};
use pierre_services::wellness_store::{WellnessStore, WellnessWriteMode};
use pierre_tool_runtime::runtime::ToolRuntime;
use tracing::info;
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::services::activity_upload::after_stored_activities_changed;
use crate::services::gear_ledger::{attribute_imported_gear, release_activity_gear, ImportedGear};

/// The `source` a data source carries for series an archive import wrote.
const ARCHIVE_SOURCE_MARKER: &str = "archive";

/// Projects barrage's chunks into the platform's stores.
pub struct CachedActivitiesSink {
    resources: Arc<ServerContext>,
}

impl fmt::Debug for CachedActivitiesSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CachedActivitiesSink")
            .finish_non_exhaustive()
    }
}

impl CachedActivitiesSink {
    /// A sink over `resources`' stores.
    #[must_use]
    pub const fn new(resources: Arc<ServerContext>) -> Self {
        Self { resources }
    }

    fn runtime(&self) -> Arc<dyn ToolRuntime> {
        Arc::<ServerContext>::clone(&self.resources)
    }
}

/// Whose rows an import's wellness is filed as: the scope's athlete, under
/// the import's key and data source, whatever the records name.
struct Filing<'import> {
    user: &'import str,
    data_source_id: &'import str,
    key: &'import str,
}

impl Filing<'_> {
    /// Stamp a record's owner, data source and key.
    fn stamp(&self, user_id: &mut String, data_source_id: &mut String, source_name: &mut String) {
        self.user.clone_into(user_id);
        self.data_source_id.clone_into(data_source_id);
        self.key.clone_into(source_name);
    }
}

/// The platform's ids for `scope`.
fn ids(scope: &Scope) -> (TenantId, Uuid) {
    (TenantId::from_uuid(scope.tenant_id.0), scope.user_id.0)
}

/// A platform error as the projection error barrage retries the step on.
fn projection(error: &AppError) -> BarrageError {
    BarrageError::Projection(error.to_string())
}

#[async_trait]
impl ProjectionSink for CachedActivitiesSink {
    async fn project(
        &self,
        scope: &Scope,
        source: SourceKey,
        rows: &[ActivityRecord],
        series: &[Option<TimeSeriesData>],
    ) -> Result<(), BarrageError> {
        if rows.is_empty() {
            return Ok(());
        }
        let (tenant_id, user_id) = ids(scope);
        let repos = &self.resources.common.repos;
        let synced_at = Utc::now();
        for (row, samples) in rows.iter().zip(series) {
            let Some(samples) = samples else {
                continue;
            };
            let source_ref = row.provenance.original_sha256.map(|digest| digest.to_hex());
            repos
                .activity_streams
                .upsert_activity_streams(&ActivityStreamsWrite {
                    tenant_id: &tenant_id,
                    user_id,
                    provider: source.as_str(),
                    activity_id: row.activity_id.as_str(),
                    origin: StreamOrigin::Archive,
                    source_ref: source_ref.as_deref(),
                    streams: samples,
                    synced_at,
                })
                .await
                .map_err(|e| projection(&e))?;
        }
        let activities: Vec<Activity> = rows.iter().map(|row| row.activity.clone()).collect();
        repos
            .activity_cache
            .upsert_activities_synced_at(
                user_id,
                &tenant_id,
                source.as_str(),
                &activities,
                synced_at,
            )
            .await
            .map_err(|e| projection(&e))?;
        // Best efforts are measured where the health sync is built in, as
        // the seed of a connected athlete's are.
        #[cfg(feature = "health-sync")]
        self.record_bests(tenant_id, user_id, &activities, series)
            .await?;
        let gear: Vec<ImportedGear<'_>> = rows
            .iter()
            .filter_map(|row| {
                row.provenance
                    .source_gear
                    .as_deref()
                    .map(|gear_name| ImportedGear {
                        activity: &row.activity,
                        gear_name,
                    })
            })
            .collect();
        attribute_imported_gear(repos, user_id, &tenant_id, source.as_str(), &gear)
            .await
            .map_err(|e| projection(&e))?;
        after_stored_activities_changed(&self.runtime(), tenant_id, user_id).await;
        Ok(())
    }

    async fn project_wellness(
        &self,
        scope: &Scope,
        source: SourceKey,
        wellness: &WellnessRecords,
    ) -> Result<(), BarrageError> {
        if wellness.is_empty() {
            return Ok(());
        }
        let (tenant_id, user_id) = ids(scope);
        let repos = &self.resources.common.repos;
        let key = source.as_str();
        let user = user_id.to_string();
        let data_source = DataSource {
            id: Uuid::new_v4().to_string(),
            user_id: user.clone(),
            provider: key.to_owned(),
            device_model: None,
            software_version: None,
            source: Some(ARCHIVE_SOURCE_MARKER.to_owned()),
            device_type: DeviceType::Unknown,
            original_source_name: None,
        };
        let store = WellnessStore::new(repos);
        let source_id = store
            .record_data_source(&tenant_id, &data_source)
            .await
            .map_err(|e| projection(&e))?;
        let own = Filing {
            user: &user,
            data_source_id: &source_id,
            key,
        };
        let synced_at = Utc::now();
        let mut writes = store.writes("archive import");
        for night in &wellness.sleep {
            let mut night = night.clone();
            own.stamp(
                &mut night.user_id,
                &mut night.data_source_id,
                &mut night.source_name,
            );
            writes
                .sleep(tenant_id, &night, synced_at)
                .await
                .map_err(|e| projection(&e))?;
        }
        for day in &wellness.recovery {
            let mut day = day.clone();
            own.stamp(
                &mut day.user_id,
                &mut day.data_source_id,
                &mut day.source_name,
            );
            writes
                .recovery(
                    tenant_id,
                    &day,
                    WellnessWriteMode::PartialSummary,
                    synced_at,
                )
                .await
                .map_err(|e| projection(&e))?;
        }
        for day in &wellness.body {
            let mut day = day.clone();
            own.stamp(
                &mut day.user_id,
                &mut day.data_source_id,
                &mut day.source_name,
            );
            writes
                .body(
                    tenant_id,
                    &day,
                    WellnessWriteMode::PartialSummary,
                    synced_at,
                )
                .await
                .map_err(|e| projection(&e))?;
        }
        for day in &wellness.daily_activity {
            let mut day = day.clone();
            own.stamp(
                &mut day.user_id,
                &mut day.data_source_id,
                &mut day.source_name,
            );
            writes
                .daily_activity(
                    tenant_id,
                    &day,
                    WellnessWriteMode::PartialSummary,
                    synced_at,
                )
                .await
                .map_err(|e| projection(&e))?;
        }
        writes.report();
        info!(
            %user_id,
            source = key,
            records = wellness.len(),
            "imported wellness stored"
        );
        Ok(())
    }

    async fn retract(
        &self,
        scope: &Scope,
        source: SourceKey,
        ids_to_retract: Option<&[ActivityId]>,
    ) -> Result<(), BarrageError> {
        let (tenant_id, user_id) = ids(scope);
        let repos = &self.resources.common.repos;
        let key = source.as_str();
        match ids_to_retract {
            None => {
                let purge = repos
                    .provider_data
                    .purge_user_provider_data(
                        user_id,
                        &tenant_id,
                        key,
                        PurgeReason::AthleteDeletedImport,
                    )
                    .await
                    .map_err(|e| projection(&e))?;
                // A provider purge spares samples it did not push; an imported
                // source's go with the source.
                let samples = repos
                    .activity_streams
                    .delete_provider_activity_streams(&tenant_id, user_id, key)
                    .await
                    .map_err(|e| projection(&e))?;
                info!(%user_id, source = key, removed = purge.total(), samples, "imported history retracted");
            }
            Some(activity_ids) => {
                for id in activity_ids {
                    release_activity_gear(repos, user_id, &tenant_id, key, id.as_str())
                        .await
                        .map_err(|e| projection(&e))?;
                    repos
                        .activity_cache
                        .delete_cached_activity(user_id, &tenant_id, key, id.as_str())
                        .await
                        .map_err(|e| projection(&e))?;
                    repos
                        .activity_streams
                        .delete_activity_streams(&tenant_id, user_id, key, id.as_str())
                        .await
                        .map_err(|e| projection(&e))?;
                }
            }
        }
        after_stored_activities_changed(&self.runtime(), tenant_id, user_id).await;
        Ok(())
    }
}

impl CachedActivitiesSink {
    /// The best efforts the chunk's runs set, measured on their samples and
    /// recorded without an announcement.
    #[cfg(feature = "health-sync")]
    async fn record_bests(
        &self,
        tenant_id: TenantId,
        user_id: Uuid,
        activities: &[Activity],
        series: &[Option<TimeSeriesData>],
    ) -> Result<(), BarrageError> {
        use pierre_services::personal_bests::is_measured_sport;

        let bests = self.resources.personal_bests();
        for (activity, samples) in activities.iter().zip(series) {
            let Some(samples) = samples else {
                continue;
            };
            if !is_measured_sport(activity.sport_type()) {
                continue;
            }
            bests
                .record_run(user_id, tenant_id, activity, Some(samples), false)
                .await
                .map_err(|e| projection(&e))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use dravr_barrage::parse::SERIES_FORMAT_VERSION;
    use pierre_database::repositories::STREAMS_FORMAT_VERSION;

    /// Both numbers are stored: barrage decodes a series at its version and
    /// the sample store records that version with it, so the two must agree
    /// or a projected row would be read under the wrong gap rules.
    #[test]
    fn the_sample_store_records_the_format_barrage_decodes_to() {
        assert_eq!(
            STREAMS_FORMAT_VERSION, SERIES_FORMAT_VERSION,
            "a stored format version both sides persist"
        );
    }
}
