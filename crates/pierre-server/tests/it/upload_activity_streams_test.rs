// ABOUTME: An upload's samples are decoded once into activity_streams (origin upload) and read from there, the file the fallback
// ABOUTME: The backfill writes the rows earlier uploads lack, once; deleting the activity deletes its samples

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Uploaded samples in the one per-activity sample store (carnet#1219).
//!
//! Every test uploads a FIT file written by `pierre_test_support::fit` through
//! the real upload service, and reads back through the real reader the
//! authentication chokepoint hands out for the `upload` key. Which source a
//! read answered from is proven by taking the other one away: with the file
//! gone a read can only come from the store, with the row gone only from the
//! file.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use std::sync::Arc;

use bytes::Bytes;
use chrono::{DateTime, Duration, Utc};
use dravr_barrage::parse::fit::{FitActivityFile, Position};
use pierre_core::constants::oauth_providers::UPLOAD;
use pierre_core::models::{TenantId, TimeSeriesData};
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::{GAP_FILLED_FORMAT_VERSION, STREAMS_FORMAT_VERSION};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::activity_upload::{
    delete_uploaded_activity, upload_activity_file,
};
use pierre_mcp_server::services::upload_streams_backfill::{
    run_upload_streams_backfill_pass, UploadStreamsBackfillReport,
};
use pierre_providers::core::{FitnessProvider, OwnerStores};
use pierre_providers::upload_provider::{file_sha256, UserProvidedProvider};
use pierre_test_support::fit::FitWorkout;
use pierre_tool_runtime::runtime::ToolRuntime;
use uuid::Uuid;

/// One athlete and the tenant they upload in.
struct Athlete {
    user_id: Uuid,
    tenant: TenantId,
}

async fn seed_athlete(resources: &Arc<ServerContext>) -> Athlete {
    let email = format!("upload-streams-{}@example.com", Uuid::new_v4());
    let (user_id, _, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .unwrap();
    Athlete { user_id, tenant }
}

fn runtime(resources: &Arc<ServerContext>) -> Arc<dyn ToolRuntime> {
    Arc::<ServerContext>::clone(resources)
}

/// Yesterday at 07:00 UTC.
fn yesterday_morning() -> DateTime<Utc> {
    (Utc::now() - Duration::days(1))
        .date_naive()
        .and_hms_opt(7, 0, 0)
        .unwrap()
        .and_utc()
}

/// A ten-minute ride's file.
fn ride_file() -> Vec<u8> {
    FitWorkout::ride(yesterday_morning(), 600).encode()
}

/// The series the file's first session decodes to: what the store must hold.
fn decoded_series(file: &[u8]) -> TimeSeriesData {
    FitActivityFile::parse(file)
        .unwrap()
        .session_series(0, Position::Read)
        .unwrap()
}

/// Upload `file` and return the id of the activity its first session became.
async fn upload(resources: &Arc<ServerContext>, athlete: &Athlete, file: &[u8]) -> String {
    let outcome = upload_activity_file(
        &runtime(resources),
        athlete.tenant,
        athlete.user_id,
        Bytes::copy_from_slice(file),
    )
    .await
    .unwrap();
    outcome.stored[0].id().to_owned()
}

/// The reader the authentication chokepoint hands out for the `upload` key.
fn reader(resources: &Arc<ServerContext>, athlete: &Athlete) -> UserProvidedProvider {
    let repos = &resources.common.repos;
    UserProvidedProvider::upload(
        OwnerStores::from_repos(repos, athlete.user_id, athlete.tenant),
        Arc::clone(&repos.uploaded_activity_files),
    )
}

async fn stored(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    id: &str,
) -> Option<TimeSeriesData> {
    resources
        .common
        .repos
        .activity_streams
        .get_activity_streams(&athlete.tenant, athlete.user_id, UPLOAD, id)
        .await
        .unwrap()
}

/// The `origin`, `source_ref` and `format_version` columns of one stored row.
async fn provenance(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    id: &str,
) -> (String, Option<String>, i16) {
    let sql = "SELECT origin, source_ref, format_version FROM activity_streams \
               WHERE tenant_id = $1 AND CAST(user_id AS TEXT) = $2 \
               AND provider = $3 AND activity_id = $4";
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => sqlx::query_as(sql)
            .bind(athlete.tenant.to_string())
            .bind(athlete.user_id.to_string())
            .bind(UPLOAD)
            .bind(id)
            .fetch_one(sqlite.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => sqlx::query_as(sql)
            .bind(athlete.tenant.to_string())
            .bind(athlete.user_id.to_string())
            .bind(UPLOAD)
            .bind(id)
            .fetch_one(postgres.pool())
            .await
            .unwrap(),
    }
}

/// The row a gap-filled seed writes, replacing the one the upload wrote.
/// `$uuid` is the cast Postgres needs on the bound user id.
macro_rules! seed_gap_filled_sql {
    ($uuid:literal) => {
        concat!(
            "INSERT INTO activity_streams (tenant_id, user_id, provider, activity_id, origin, \
             source_ref, format_version, streams_json, synced_at) VALUES ($1, $2",
            $uuid,
            ", $3, $4, $5, $6, $7, $8, $9) \
             ON CONFLICT (tenant_id, user_id, provider, activity_id) DO UPDATE SET \
             origin = EXCLUDED.origin, source_ref = EXCLUDED.source_ref, \
             format_version = EXCLUDED.format_version, streams_json = EXCLUDED.streams_json, \
             synced_at = EXCLUDED.synced_at"
        )
    };
}

/// Write one row at the gap-filled format version, as a build before
/// carnet#1226 wrote it: plain fixes, one copied into the sample without one.
async fn seed_gap_filled_row(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    provider: &str,
    id: &str,
    origin: &str,
    source_ref: Option<&str>,
) {
    let json = r#"{"timestamps":[0,1,2],"heart_rate":[120,null,122],
        "gps_coordinates":[[45.5,-73.6],[45.5,-73.6],[45.5002,-73.6]],
        "distance":[0.0,0.0,22.2]}"#;
    let synced_at = Utc::now() - Duration::days(1);
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(seed_gap_filled_sql!(""))
                .bind(athlete.tenant.to_string())
                .bind(athlete.user_id.to_string())
                .bind(provider)
                .bind(id)
                .bind(origin)
                .bind(source_ref)
                .bind(GAP_FILLED_FORMAT_VERSION)
                .bind(json)
                .bind(synced_at)
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => {
            sqlx::query(seed_gap_filled_sql!("::uuid"))
                .bind(athlete.tenant.to_string())
                .bind(athlete.user_id.to_string())
                .bind(provider)
                .bind(id)
                .bind(origin)
                .bind(source_ref)
                .bind(GAP_FILLED_FORMAT_VERSION)
                .bind(json)
                .bind(synced_at)
                .execute(postgres.pool())
                .await
                .unwrap();
        }
    }
}

async fn drop_file(resources: &Arc<ServerContext>, athlete: &Athlete, file: &[u8]) {
    let dropped = resources
        .common
        .repos
        .uploaded_activity_files
        .delete_uploaded_file(&athlete.tenant, athlete.user_id, &file_sha256(file))
        .await
        .unwrap();
    assert!(dropped, "test setup: the uploaded file was kept");
}

async fn drop_row(resources: &Arc<ServerContext>, athlete: &Athlete, id: &str) {
    let dropped = resources
        .common
        .repos
        .activity_streams
        .delete_activity_streams(&athlete.tenant, athlete.user_id, UPLOAD, id)
        .await
        .unwrap();
    assert!(dropped, "test setup: the upload's samples row was written");
}

#[tokio::test]
async fn an_upload_stores_its_decoded_samples_once_naming_the_file() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources).await;
    let file = ride_file();

    let id = upload(&resources, &athlete, &file).await;

    let samples = stored(&resources, &athlete, &id).await.unwrap();
    assert_eq!(
        samples,
        decoded_series(&file),
        "the stored row is the file's decoded series"
    );
    assert!(
        samples.gps_coordinates.is_some(),
        "the ride's track is kept"
    );
    assert!(
        samples.heart_rate.is_some(),
        "the ride's heart rate is kept"
    );
    assert_eq!(
        provenance(&resources, &athlete, &id).await,
        (
            "upload".to_owned(),
            Some(file_sha256(&file)),
            STREAMS_FORMAT_VERSION
        ),
        "the row says it came from an upload and names the file"
    );
}

#[tokio::test]
async fn the_samples_are_read_from_the_store_without_the_file() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources).await;
    let file = ride_file();
    let id = upload(&resources, &athlete, &file).await;

    drop_file(&resources, &athlete, &file).await;

    let reader = reader(&resources, &athlete);
    assert_eq!(
        reader.get_activity_streams(&id).await.unwrap(),
        Some(decoded_series(&file)),
        "with the file gone, the series can only come from the store"
    );
    let activity = reader.get_activity_with_streams(&id).await.unwrap();
    assert_eq!(
        activity.time_series_data(),
        Some(&decoded_series(&file)),
        "the stored series is attached to the cached activity"
    );
    assert_eq!(activity.laps().map(Vec::len), Some(1), "the cached laps");
}

#[tokio::test]
async fn an_upload_without_stored_samples_is_read_from_its_file() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources).await;
    let file = ride_file();
    let id = upload(&resources, &athlete, &file).await;

    drop_row(&resources, &athlete, &id).await;

    let reader = reader(&resources, &athlete);
    assert_eq!(
        reader.get_activity_streams(&id).await.unwrap(),
        Some(decoded_series(&file)),
        "with the row gone, the series is decoded from the file"
    );
    let activity = reader.get_activity_with_streams(&id).await.unwrap();
    assert_eq!(
        activity.time_series_data(),
        Some(&decoded_series(&file)),
        "the activity read with its series is decoded from the file"
    );
    assert!(
        stored(&resources, &athlete, &id).await.is_none(),
        "a read never writes: the backfill does"
    );
}

#[tokio::test]
async fn the_backfill_stores_an_earlier_uploads_samples_once() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources).await;
    let file = ride_file();
    let id = upload(&resources, &athlete, &file).await;
    // An upload made before the store held uploads: its file, no row.
    drop_row(&resources, &athlete, &id).await;

    let first = run_upload_streams_backfill_pass(&resources.common.repos)
        .await
        .unwrap();

    assert_eq!(
        first,
        UploadStreamsBackfillReport {
            files: 1,
            activities: 1,
            unreadable: 0,
        },
        "one file decoded for the one activity owed"
    );
    assert_eq!(
        stored(&resources, &athlete, &id).await,
        Some(decoded_series(&file)),
        "the backfilled row is the file's decoded series"
    );
    assert_eq!(
        provenance(&resources, &athlete, &id).await,
        (
            "upload".to_owned(),
            Some(file_sha256(&file)),
            STREAMS_FORMAT_VERSION
        ),
        "the backfilled row says it came from an upload and names the file"
    );
    let second = run_upload_streams_backfill_pass(&resources.common.repos)
        .await
        .unwrap();
    assert_eq!(
        second,
        UploadStreamsBackfillReport::default(),
        "nothing is owed twice"
    );
}

#[tokio::test]
async fn the_backfill_passes_over_an_upload_whose_file_is_gone() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources).await;
    let file = ride_file();
    let id = upload(&resources, &athlete, &file).await;
    drop_row(&resources, &athlete, &id).await;
    drop_file(&resources, &athlete, &file).await;

    let report = run_upload_streams_backfill_pass(&resources.common.repos)
        .await
        .unwrap();

    assert_eq!(
        report,
        UploadStreamsBackfillReport {
            files: 0,
            activities: 0,
            unreadable: 1,
        },
        "an activity whose file is gone is counted, not written"
    );
    assert!(
        stored(&resources, &athlete, &id).await.is_none(),
        "nothing is written without the file"
    );
}

#[tokio::test]
async fn deleting_an_upload_deletes_its_samples() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources).await;
    let id = upload(&resources, &athlete, &ride_file()).await;

    delete_uploaded_activity(&runtime(&resources), athlete.tenant, athlete.user_id, &id)
        .await
        .unwrap();

    assert!(
        stored(&resources, &athlete, &id).await.is_none(),
        "the samples go with the activity"
    );
}

#[tokio::test]
async fn another_athletes_samples_are_never_read() {
    let resources = common::create_test_server_resources().await.unwrap();
    let owner = seed_athlete(&resources).await;
    let other = seed_athlete(&resources).await;
    let id = upload(&resources, &owner, &ride_file()).await;

    assert!(
        stored(&resources, &other, &id).await.is_none(),
        "the store is keyed by the owner"
    );
    assert!(
        reader(&resources, &other)
            .get_activity_streams(&id)
            .await
            .is_err(),
        "another athlete's reader finds neither the row nor the file"
    );
}

#[tokio::test]
async fn a_gap_filled_upload_row_is_replayed_from_its_file() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources).await;
    let file = ride_file();
    let id = upload(&resources, &athlete, &file).await;
    let sha = file_sha256(&file);
    seed_gap_filled_row(&resources, &athlete, UPLOAD, &id, "upload", Some(&sha)).await;

    assert_eq!(
        reader(&resources, &athlete)
            .get_activity_streams(&id)
            .await
            .unwrap(),
        Some(decoded_series(&file)),
        "a gap-filled row is never read: the file is decoded again"
    );

    let report = run_upload_streams_backfill_pass(&resources.common.repos)
        .await
        .unwrap();
    assert_eq!(
        report,
        UploadStreamsBackfillReport {
            files: 1,
            activities: 1,
            unreadable: 0,
        },
        "the backfill replays the gap-filled row from its file"
    );
    assert_eq!(
        provenance(&resources, &athlete, &id).await,
        ("upload".to_owned(), Some(sha), STREAMS_FORMAT_VERSION),
        "the replayed row is at the current version"
    );
    assert_eq!(
        stored(&resources, &athlete, &id).await,
        Some(decoded_series(&file)),
        "the replayed row is the file's decoded series"
    );
    assert_eq!(
        run_upload_streams_backfill_pass(&resources.common.repos)
            .await
            .unwrap(),
        UploadStreamsBackfillReport::default(),
        "a replayed row is not owed again"
    );
}

#[tokio::test]
async fn a_gap_filled_pushed_row_is_read_without_its_track_and_distance() {
    let resources = common::create_test_server_resources().await.unwrap();
    let athlete = seed_athlete(&resources).await;
    seed_gap_filled_row(&resources, &athlete, "garmin", "pushed-1", "push", None).await;

    let streams = resources
        .common
        .repos
        .activity_streams
        .get_activity_streams(&athlete.tenant, athlete.user_id, "garmin", "pushed-1")
        .await
        .unwrap()
        .expect("a pushed row has no source to replay, so it is still read");

    assert_eq!(
        streams.gps_coordinates, None,
        "a copied fix cannot be told from a recorded one"
    );
    assert_eq!(
        streams.distance, None,
        "a repeated distance cannot be told from a recorded one"
    );
    assert_eq!(
        streams.heart_rate,
        Some(vec![Some(120), None, Some(122)]),
        "the channels that kept their gaps are read as stored"
    );
}
