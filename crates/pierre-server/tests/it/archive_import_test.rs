// ABOUTME: Importing an athlete's own Strava or Garmin export end to end: link, steps, projection, reads, delete, erasure
// ABOUTME: barrage on memory stores with a fixture fetcher; the routes, the in-process runner and the sink are the real ones

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Archive imports (carnet#1218).
//!
//! Each test builds a server context whose imports run on barrage's memory
//! stores and a fetcher that serves a synthetic Strava export, then drives the
//! `/api/imports` routes as the athlete's own session would. The steps run in
//! this process, exactly as a local stack runs them; the projection, the
//! provider reads and the deletion paths are the production ones.

use crate::common;

use std::io::{Cursor, Write};
use std::sync::Arc;
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use bytes::Bytes;
use chrono::{DateTime, Duration, Utc};
use dravr_barrage::blob::memory::MemoryBlobStore;
use dravr_barrage::blob::BlobKey;
use dravr_barrage::digest::Sha256;
use dravr_barrage::store::memory::{MemoryActivityRecordStore, MemoryImportJobStore};
use dravr_barrage::store::{BarrageStores, JobStatus, NewImportJob, PageReq, RecordQuery};
use dravr_barrage::{BarrageError, Importer, Scope, SourceKey, StepOutcome};
use futures_util::stream::{self, BoxStream, StreamExt};
use pierre_core::constants::oauth_providers::{GARMIN_ARCHIVE, STRAVA, STRAVA_ARCHIVE};
use pierre_core::models::TenantId;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_database::repositories::DeletionActor;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::imports::import_routes;
use pierre_mcp_server::routes::onboarding::OnboardingRoutes;
use pierre_mcp_server::services::archive_import::fetch::{ExportFetcher, LinkCheck, LinkRefusal};
use pierre_mcp_server::services::archive_import::link::{link_digest, open, seal};
use pierre_mcp_server::services::archive_import::runner::ImportRunner;
use pierre_mcp_server::services::archive_import::sink::CachedActivitiesSink;
use pierre_mcp_server::services::archive_import::sweeps::erase_deleted_accounts;
use pierre_mcp_server::services::archive_import::{ArchiveImports, ImportWiring};
use pierre_providers::upload_provider::file_sha256;
use pierre_services::stored_health;
use pierre_services::user_removal::{remove_user, UserRemoval};
use pierre_test_support::fit::FitWorkout;
use pierre_tool_runtime::protocol::{UniversalRequest, UniversalToolExecutor};
use serde_json::{json, Value};
use tokio::time::sleep;
use tower::ServiceExt;
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

/// A link the fixture fetcher serves the export behind.
pub(crate) const LINK: &str = "https://export.example.com/export_1234.zip?signature=abc";

/// A link the fixture fetcher refuses, as tronc's guard refuses a host off
/// the allowlist.
pub(crate) const REFUSED_LINK: &str = "https://elsewhere.example.com/export.zip";

/// Strava's `activities.csv` header row (English), as barrage pins it.
const CSV_HEADER: &str = "Activity ID,Activity Date,Activity Name,Activity Type,\
    Activity Description,Elapsed Time,Distance,Max Heart Rate,Relative Effort,Commute,\
    Activity Private Note,Activity Gear,Filename,Athlete Weight,Bike Weight,Elapsed Time,\
    Moving Time,Distance,Max Speed,Average Speed,Elevation Gain,Max Heart Rate,\
    Average Heart Rate,Calories";

/// Serves one export, and refuses [`REFUSED_LINK`].
struct FixtureFetcher {
    export: Vec<u8>,
}

#[async_trait]
impl ExportFetcher for FixtureFetcher {
    async fn check(&self, link: &str) -> Result<LinkCheck, LinkRefusal> {
        if link == REFUSED_LINK {
            return Err(LinkRefusal::HostNotAllowed);
        }
        Ok(LinkCheck {
            size_bytes: Some(self.export.len() as u64),
        })
    }

    async fn download(&self, _link: &str) -> BoxStream<'static, Result<Bytes, BarrageError>> {
        let chunks: Vec<Result<Bytes, BarrageError>> = self
            .export
            .chunks(4096)
            .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
            .collect();
        stream::iter(chunks).boxed()
    }
}

/// Imports on fresh memory stores, fetching `export`, steps in this process.
pub(crate) fn imports_serving(export: Vec<u8>) -> (ArchiveImports, BarrageStores) {
    let stores = BarrageStores {
        blobs: Arc::new(MemoryBlobStore::new()),
        records: Arc::new(MemoryActivityRecordStore::new()),
        jobs: Arc::new(MemoryImportJobStore::new()),
    };
    let imports = ArchiveImports::new(ImportWiring {
        stores: stores.clone(),
        fetcher: Arc::new(FixtureFetcher { export }),
        runner: ImportRunner::InProcess,
    });
    (imports, stores)
}

/// One CSV row: id, start, name, type, elapsed seconds, filename.
fn csv_row(
    id: &str,
    start: DateTime<Utc>,
    name: &str,
    kind: &str,
    elapsed: u32,
    file: &str,
) -> String {
    let date = start.format("%b %-d, %Y, %-I:%M:%S %p");
    format!(
        "{id},\"{date}\",{name},{kind},,{elapsed},5.0,,,false,,,{file},,,{elapsed}.0,{elapsed}.0,\
         5000.0,4.0,3.1,40.0,,,321"
    )
}

/// A ZIP archive of `(name, bytes)` members.
fn zip_of(members: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    for (name, bytes) in members {
        writer.start_file(*name, options).expect("a member starts");
        writer.write_all(bytes).expect("a member is written");
    }
    writer.finish().expect("the archive closes").into_inner()
}

/// Ten days ago at 07:00 UTC.
fn ten_days_ago() -> DateTime<Utc> {
    (Utc::now() - Duration::days(10))
        .date_naive()
        .and_hms_opt(7, 0, 0)
        .expect("a valid time")
        .and_utc()
}

/// A Strava export holding a recorded ride (`9001`, a FIT file) and a manual
/// run (`9002`, no file).
pub(crate) fn strava_export() -> Vec<u8> {
    let start = ten_days_ago();
    let rows = [
        csv_row(
            "9001",
            start,
            "Morning Ride",
            "Ride",
            600,
            "activities/9001.fit",
        ),
        csv_row(
            "9002",
            start + Duration::days(1),
            "Treadmill",
            "Run",
            1200,
            "",
        ),
    ];
    let mut csv = format!("{CSV_HEADER}\n");
    for row in &rows {
        csv.push_str(row);
        csv.push('\n');
    }
    zip_of(&[
        ("activities.csv", csv.as_bytes()),
        (
            "activities/9001.fit",
            &FitWorkout::ride(start, 600).encode(),
        ),
        ("messages/inbox.json", b"{\"private\": true}"),
    ])
}

/// A Strava export holding one recorded ride, `id`, of `seconds` one-second
/// samples from `start`.
pub(crate) fn one_ride_export(id: &str, start: DateTime<Utc>, seconds: u32) -> Vec<u8> {
    let file = format!("activities/{id}.fit");
    let csv = format!(
        "{CSV_HEADER}\n{}\n",
        csv_row(id, start, "Evening Ride", "Ride", seconds, &file)
    );
    zip_of(&[
        ("activities.csv", csv.as_bytes()),
        (&file, &FitWorkout::ride(start, seconds).encode()),
    ])
}

/// One athlete: the account, the tenant they act in and a session bearer.
pub(crate) struct Athlete {
    pub(crate) user_id: Uuid,
    pub(crate) tenant: TenantId,
    pub(crate) token: String,
}

impl Athlete {
    fn scope(&self) -> Scope {
        Scope::new(self.tenant.as_uuid(), self.user_id)
    }
}

pub(crate) async fn seed_athlete(resources: &Arc<ServerContext>, label: &str) -> Athlete {
    let email = format!("{label}-{}@example.com", Uuid::new_v4());
    let (user_id, user, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .expect("the athlete is created");
    let token = common::generate_test_token(resources, &user).await;
    Athlete {
        user_id,
        tenant,
        token,
    }
}

pub(crate) async fn call(
    resources: &Arc<ServerContext>,
    method: &str,
    uri: &str,
    bearer: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {bearer}"))
        .header("content-type", "application/json")
        .body(body.map_or_else(Body::empty, |body| Body::from(body.to_string())))
        .expect("a request");
    let response = import_routes()
        .with_state(Arc::clone(resources))
        .oneshot(request)
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// Start an import of the athlete's export and wait for it to finish.
pub(crate) async fn import_export(resources: &Arc<ServerContext>, athlete: &Athlete) -> Value {
    let (status, started) = call(
        resources,
        "POST",
        "/api/imports/strava_archive",
        &athlete.token,
        Some(json!({ "url": LINK })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{started}");
    for _ in 0..200 {
        let (_, current) = call(
            resources,
            "GET",
            "/api/imports/strava_archive/current",
            &athlete.token,
            None,
        )
        .await;
        let status = current["import"]["status"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        if ["completed", "failed", "cancelled"].contains(&status.as_str()) {
            return current["import"].clone();
        }
        sleep(StdDuration::from_millis(25)).await;
    }
    panic!("the import did not finish");
}

#[tokio::test]
async fn an_export_imports_into_the_cache_and_the_sample_store() {
    let (imports, _) = imports_serving(strava_export());
    let resources = common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context");
    let athlete = seed_athlete(&resources, "archive-import").await;

    let finished = import_export(&resources, &athlete).await;

    assert_eq!(finished["status"], "completed", "{finished}");
    assert_eq!(
        finished["history_held"], true,
        "the history it brought in is held: {finished}"
    );
    assert_eq!(
        finished["imported"], 2,
        "the ride and the manual run: {finished}"
    );
    assert_eq!(finished["failed"], 0, "{finished}");
    assert!(
        finished.get("url").is_none(),
        "the link is never served back"
    );
    let cache = &resources.common.repos.activity_cache;
    for id in ["9001", "9002"] {
        let row = cache
            .get_cached_activity(athlete.user_id, &athlete.tenant, STRAVA_ARCHIVE, id)
            .await
            .expect("the cache reads");
        assert!(
            row.is_some(),
            "activity {id} is served under strava_archive"
        );
    }
    let streams = &resources.common.repos.activity_streams;
    let ride = streams
        .get_activity_streams(&athlete.tenant, athlete.user_id, STRAVA_ARCHIVE, "9001")
        .await
        .expect("the sample store reads")
        .expect("the ride's samples were projected");
    assert_eq!(ride.timestamps.len(), 600, "every sample of the recording");
    let manual = streams
        .get_activity_streams(&athlete.tenant, athlete.user_id, STRAVA_ARCHIVE, "9002")
        .await
        .expect("the sample store reads");
    assert!(manual.is_none(), "a manual entry has no samples to project");
    let synced = cache
        .latest_activity_sync_any(athlete.user_id, &athlete.tenant)
        .await
        .expect("the freshness reads");
    assert_eq!(synced, None, "an import is not a provider sync");
}

/// `get_activities` as the agent calls it, in the athlete's tenant.
pub(crate) async fn agent_activities(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    parameters: Value,
) -> Value {
    let response = UniversalToolExecutor::new(resources.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_transport(Transport::WebApp)
        .execute_tool(UniversalRequest {
            tool_name: "get_activities".to_owned(),
            parameters,
            user_id: athlete.user_id.to_string(),
            protocol: "mcp".to_owned(),
            tenant_id: Some(athlete.tenant.to_string()),
        })
        .await
        .expect("the tool dispatches");
    assert!(response.success, "get_activities ran: {:?}", response.error);
    response.result.expect("the tool returned a payload")
}

/// A tool as the agent calls it, in the athlete's tenant.
async fn agent_tool(
    resources: &Arc<ServerContext>,
    athlete: &Athlete,
    tool: &str,
    parameters: Value,
) -> Value {
    let response = UniversalToolExecutor::new(resources.clone())
        .with_scopes(OAuthScope::self_grant())
        .with_transport(Transport::WebApp)
        .execute_tool(UniversalRequest {
            tool_name: tool.to_owned(),
            parameters,
            user_id: athlete.user_id.to_string(),
            protocol: "mcp".to_owned(),
            tenant_id: Some(athlete.tenant.to_string()),
        })
        .await
        .expect("the tool dispatches");
    assert!(response.success, "{tool} ran: {:?}", response.error);
    response.result.expect("the tool returned a payload")
}

/// A Garmin Connect export holding wellness alone, for the athlete in
/// Toronto (five hours west), dated `days_ago` days before today: a day's
/// summary, its overnight HRV, a weigh-in, a `VO2max` estimate and the
/// night before it.
fn garmin_wellness_export(days_ago: i64) -> Vec<u8> {
    let day = (Utc::now() - Duration::days(days_ago)).date_naive();
    let date = day.format("%Y-%m-%d").to_string();
    let uds = json!([{
        "calendarDate": date, "totalSteps": 11_284,
        "activeKilocalories": 520.0, "totalKilocalories": 2410.0,
        "moderateIntensityMinutes": 22, "vigorousIntensityMinutes": 41,
        "restingHeartRate": 49, "averageSpo2Value": 96.0,
        "allDayStress": { "aggregatorList": [{ "type": "TOTAL", "averageStressLevel": 28 }] },
        "bodyBattery": { "bodyBatteryStatList": [
            { "bodyBatteryStatType": "HIGHEST", "statsValue": 88 }
        ]},
        "wellnessStartTimeGmt": format!("{date}T05:00:00.0"),
        "wellnessStartTimeLocal": format!("{date}T00:00:00.0")
    }]);
    let hrv = json!([{ "calendarDate": date, "metrics": [{ "type": "HRV", "value": 58.0 }] }]);
    let bio = json!([{ "metaData": { "calendarDate": format!("{date}T00:00:00.0") },
                       "weight": { "weight": 71_800.0,
                                   "timestampGMT": format!("{date}T12:00:00.0") } }]);
    let max_met = json!([{ "calendarDate": date, "vo2MaxValue": 52.0, "sport": "RUNNING" }]);
    let sleep = json!([{
        "calendarDate": date,
        "sleepStartTimestampGMT": format!("{date}T04:05:00.0"),
        "sleepEndTimestampGMT": format!("{date}T11:02:00.0"),
        "deepSleepSeconds": 5400, "lightSleepSeconds": 13200,
        "remSleepSeconds": 5100, "awakeSleepSeconds": 1320,
        "sleepScores": { "overallScore": 82 }
    }]);
    let docs = [uds, hrv, bio, max_met, sleep].map(|doc| doc.to_string());
    zip_of(&[
        (
            "DI_CONNECT/DI-Connect-Aggregator/UDSFile_2026-01-01_2026-12-31.json",
            docs[0].as_bytes(),
        ),
        (
            "DI_CONNECT/DI-Connect-Wellness/2026-01-01_2026-12-31_100001_healthStatusData.json",
            docs[1].as_bytes(),
        ),
        (
            "DI_CONNECT/DI-Connect-Wellness/100001_userBioMetrics.json",
            docs[2].as_bytes(),
        ),
        (
            "DI_CONNECT/DI-Connect-Metrics/MetricsMaxMetData_20260101_20261231_100001.json",
            docs[3].as_bytes(),
        ),
        (
            "DI_CONNECT/DI-Connect-Wellness/2026-01-01_2026-12-31_100001_sleepData.json",
            docs[4].as_bytes(),
        ),
    ])
}

/// T1 (JF's trigger, carnet#1306): an athlete whose only data is their own
/// Garmin export gets their resting heart rate, HRV, `SpO2`, stress, Body
/// Battery, weight, `VO2max`, steps and sleep back from the tools, and
/// counts as holding recovery data. The export's wellness used to land only
/// as series no daily reader reads.
#[tokio::test]
async fn an_export_only_athlete_reads_their_wellness_back_from_the_tools() {
    let (imports, stores) = imports_serving(Vec::new());
    let resources = common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context");
    let athlete = seed_athlete(&resources, "garmin-export-wellness").await;

    // The Garmin import is not offered on the routes yet, so barrage's own
    // importer runs it, projecting through the platform's production sink.
    let importer = Importer::new(
        stores,
        Arc::new(CachedActivitiesSink::new(Arc::clone(&resources))),
    );
    let scope = athlete.scope();
    let job = importer
        .create(
            &scope,
            NewImportJob {
                source: SourceKey::GarminArchive,
                status: JobStatus::Queued,
                url_sha256: None,
                url_ciphertext: Some(b"sealed link".to_vec()),
                expires_at: None,
            },
        )
        .await
        .expect("the job is created");
    let export = garmin_wellness_export(3);
    let body = stream::iter(vec![Ok(Bytes::from(export))]).boxed();
    let mut outcome = importer
        .download(&scope, job.id, body)
        .await
        .expect("the export spools");
    while let StepOutcome::Next { expect_cursor } = outcome {
        outcome = importer
            .step(&scope, job.id, expect_cursor)
            .await
            .expect("a step runs");
    }
    assert_eq!(
        outcome,
        StepOutcome::Finished(JobStatus::Completed),
        "the export imports"
    );

    let recovery = agent_tool(&resources, &athlete, "get_recovery_metrics", json!({})).await;
    let day = &recovery["metrics"][0];
    assert_eq!(recovery["count"], 1, "one recovery day: {recovery}");
    assert_eq!(day["resting_heart_rate"], 49, "resting heart rate: {day}");
    assert_eq!(
        day["hrv_rmssd"], 58.0,
        "the overnight HRV joins the day: {day}"
    );
    assert_eq!(day["spo2"], 96.0, "SpO2: {day}");
    assert_eq!(day["stress_score"], 28, "stress: {day}");
    assert_eq!(day["body_battery"], 88, "Body Battery: {day}");
    assert_eq!(
        day["source_name"], GARMIN_ARCHIVE,
        "the export's key: {day}"
    );

    let body = agent_tool(&resources, &athlete, "get_health_snapshots", json!({})).await;
    let snapshot = &body["snapshots"][0];
    assert_eq!(body["count"], 1, "one body day: {body}");
    assert_eq!(snapshot["weight_kg"], 71.8, "the weigh-in: {snapshot}");
    assert_eq!(
        snapshot["vo2_max"], 52.0,
        "VO2max joins the day: {snapshot}"
    );
    let activity = &body["daily_activity"][0];
    assert_eq!(activity["steps"], 11_284, "the day's steps: {body}");
    assert_eq!(
        activity["total_kcal"], 2410.0,
        "the day's energy: {activity}"
    );
    assert_eq!(
        activity["intensity_minutes"], 104,
        "intensity minutes as Garmin counts them: {activity}"
    );

    let sleep = agent_tool(&resources, &athlete, "get_sleep_sessions", json!({})).await;
    let night = &sleep["sessions"][0];
    assert_eq!(sleep["count"], 1, "one night (carnet#1272): {sleep}");
    assert_eq!(
        night["total_sleep_seconds"], 23_700,
        "the night's sleep: {night}"
    );
    assert_eq!(night["sleep_score"], 82, "the night's score: {night}");

    let now = Utc::now();
    let holds = stored_health::holds_recovery_rows(
        &resources.common.repos,
        resources.data().provider_registry().as_ref(),
        athlete.user_id,
        &athlete.tenant,
        (now - Duration::days(14), now),
    )
    .await
    .expect("the origins read");
    assert!(holds, "bug: an export-only athlete holds recovery data");
}

fn activity_ids(payload: &Value) -> Vec<String> {
    let mut ids: Vec<String> = payload["activities"]
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter_map(|row| row["id"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids
}

#[tokio::test]
async fn the_agent_reads_imported_history_with_nothing_connected() {
    let (imports, _) = imports_serving(strava_export());
    let resources = common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context");
    let athlete = seed_athlete(&resources, "archive-agent").await;
    import_export(&resources, &athlete).await;

    let unpinned = agent_activities(&resources, &athlete, json!({ "limit": 10 })).await;
    assert_eq!(activity_ids(&unpinned), ["9001", "9002"], "{unpinned}");
    let as_strava = agent_activities(&resources, &athlete, json!({ "provider": STRAVA })).await;
    assert_eq!(
        activity_ids(&as_strava),
        ["9001", "9002"],
        "asking for Strava serves the history imported from it: {as_strava}"
    );
}

#[tokio::test]
async fn deleting_the_imported_history_removes_rows_samples_and_records() {
    let (imports, stores) = imports_serving(strava_export());
    let resources = common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context");
    let athlete = seed_athlete(&resources, "archive-delete").await;
    import_export(&resources, &athlete).await;

    let (status, body) = call(
        &resources,
        "DELETE",
        "/api/imports/strava_archive/data",
        &athlete.token,
        None,
    )
    .await;

    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["originals_deleted"], 1, "the ride's FIT file: {body}");
    let held = resources
        .common
        .repos
        .activity_cache
        .held_user_provided(athlete.user_id, &athlete.tenant)
        .await
        .expect("the cache reads");
    assert!(
        held.is_empty(),
        "no imported row is served any more: {held:?}"
    );
    let samples = resources
        .common
        .repos
        .activity_streams
        .get_activity_streams(&athlete.tenant, athlete.user_id, STRAVA_ARCHIVE, "9001")
        .await
        .expect("the sample store reads");
    assert!(samples.is_none(), "the samples went with the history");
    let records = stores
        .records
        .list(&athlete.scope(), &RecordQuery::all(PageReq::first(50)))
        .await
        .expect("barrage reads");
    assert!(records.items.is_empty(), "barrage holds no record of it");
    let (_, current) = call(
        &resources,
        "GET",
        "/api/imports/strava_archive/current",
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(
        current["import"]["history_held"], false,
        "the completed import reports its history gone: {current}"
    );
}

#[tokio::test]
async fn a_second_import_of_a_source_waits_for_the_first() {
    let (imports, _) = imports_serving(strava_export());
    let resources = common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context");
    let athlete = seed_athlete(&resources, "archive-conflict").await;

    let (first, waiting) = call(
        &resources,
        "POST",
        "/api/imports/strava_archive",
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(first, StatusCode::CREATED, "{waiting}");
    assert_eq!(waiting["status"], "awaiting_link", "{waiting}");
    let (second, body) = call(
        &resources,
        "POST",
        "/api/imports/strava_archive",
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(
        second,
        StatusCode::CONFLICT,
        "one active import per source: {body}"
    );

    // Pasting the link replaces the placeholder that waited for it.
    let finished = import_export(&resources, &athlete).await;
    assert_eq!(finished["status"], "completed", "{finished}");
}

#[tokio::test]
async fn a_refused_link_starts_nothing() {
    let (imports, _) = imports_serving(strava_export());
    let resources = common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context");
    let athlete = seed_athlete(&resources, "archive-refused").await;

    let (check, verdict) = call(
        &resources,
        "POST",
        "/api/imports/strava_archive/check",
        &athlete.token,
        Some(json!({ "url": REFUSED_LINK })),
    )
    .await;
    assert_eq!(check, StatusCode::OK, "{verdict}");
    assert_eq!(verdict["accepted"], false, "{verdict}");
    assert_eq!(verdict["refusal"], "link_host_not_allowed", "{verdict}");

    let (status, body) = call(
        &resources,
        "POST",
        "/api/imports/strava_archive",
        &athlete.token,
        Some(json!({ "url": REFUSED_LINK })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body.to_string().contains("link_host_not_allowed"), "{body}");
    assert!(
        !body.to_string().contains("elsewhere.example.com"),
        "the link never comes back in an error: {body}"
    );
    let (_, current) = call(
        &resources,
        "GET",
        "/api/imports/strava_archive/current",
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(current["import"], Value::Null, "{current}");
}

#[tokio::test]
async fn only_the_athletes_own_session_manages_imports() {
    let (imports, _) = imports_serving(strava_export());
    let resources = common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context");
    let athlete = seed_athlete(&resources, "archive-api-key").await;
    let api_key =
        common::issue_test_api_key(&resources.agent.database, athlete.user_id, "importer")
            .await
            .expect("an API key");

    let (status, body) = call(
        &resources,
        "POST",
        "/api/imports/strava_archive",
        &api_key,
        Some(json!({ "url": LINK })),
    )
    .await;

    assert!(
        matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN),
        "an API key starts no import: {status} {body}"
    );
    let (_, current) = call(
        &resources,
        "GET",
        "/api/imports/strava_archive/current",
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(
        current["import"],
        Value::Null,
        "nothing was started: {current}"
    );
    let (unknown, _) = call(
        &resources,
        "POST",
        "/api/imports/garmin_archive",
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(
        unknown,
        StatusCode::NOT_FOUND,
        "only offered sources are importable"
    );
}

#[tokio::test]
async fn a_sealed_link_opens_only_for_the_athlete_it_was_sealed_for() {
    let resources = common::create_test_server_resources()
        .await
        .expect("a server context");
    let security = resources.common.repos.security.as_ref();
    let mine = Scope::new(Uuid::new_v4(), Uuid::new_v4());
    let theirs = Scope::new(mine.tenant_id.0, Uuid::new_v4());
    let sealed = seal(security, &mine, SourceKey::StravaArchive, LINK).expect("the link seals");
    let digest = link_digest(LINK);

    let opened =
        open(security, &mine, SourceKey::StravaArchive, &digest, &sealed).expect("it opens");
    assert_eq!(
        opened, LINK,
        "the athlete it was sealed for reads the link back"
    );
    assert!(
        open(
            security,
            &theirs,
            SourceKey::StravaArchive,
            &digest,
            &sealed
        )
        .is_err(),
        "a ciphertext copied onto another athlete's import does not decrypt"
    );
    assert!(
        !String::from_utf8_lossy(&sealed).contains("export.example.com"),
        "the stored bytes do not hold the link in the clear"
    );
}

#[tokio::test]
async fn a_deleted_account_is_erased_from_barrage() {
    let (imports, stores) = imports_serving(strava_export());
    let resources = common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context");
    let athlete = seed_athlete(&resources, "archive-erasure").await;
    import_export(&resources, &athlete).await;
    let upload = FitWorkout::ride(ten_days_ago(), 600).encode();
    let upload_sha = file_sha256(&upload);
    resources
        .common
        .originals
        .put_upload(
            &athlete.tenant,
            athlete.user_id,
            &upload_sha,
            Bytes::from(upload),
        )
        .await
        .expect("the upload is kept");
    let upload_key = BlobKey::upload(
        athlete.scope(),
        Sha256::parse_hex(&upload_sha).expect("a hash"),
    );
    assert!(
        stores
            .blobs
            .get(&upload_key)
            .await
            .expect("blobs read")
            .is_some(),
        "test setup: the upload lives in the deployment's one blob store"
    );

    let removal = remove_user(
        &resources.common.repos,
        None,
        None,
        None,
        athlete.user_id,
        DeletionActor::Athlete,
    )
    .await
    .expect("the removal runs");
    assert!(matches!(removal, UserRemoval::Removed(_)), "{removal:?}");
    let erased = erase_deleted_accounts(&resources)
        .await
        .expect("the sweep runs");

    assert_eq!(erased, 1, "the account queued by its delete is erased");
    let records = stores
        .records
        .list(&athlete.scope(), &RecordQuery::all(PageReq::first(50)))
        .await
        .expect("barrage reads");
    assert!(
        records.items.is_empty(),
        "no record of the deleted account is left"
    );
    assert!(
        stores
            .blobs
            .get(&upload_key)
            .await
            .expect("blobs read")
            .is_none(),
        "bug: the file the deleted account uploaded is left in the blob store"
    );
    let again = erase_deleted_accounts(&resources)
        .await
        .expect("the sweep runs");
    assert_eq!(again, 0, "an erased account leaves the outbox");
}

/// A deployment where imports do not run still keeps uploaded files, so a
/// deleted account's erasure runs there too: the sweep works through the
/// queue on the originals store alone rather than refusing for want of
/// imports.
#[tokio::test]
async fn a_deleted_account_is_erased_where_imports_do_not_run() {
    let resources = common::create_test_server_resources()
        .await
        .expect("a server context");
    assert!(
        !resources.common.archive_imports.is_enabled(),
        "test setup: imports do not run here"
    );
    let athlete = seed_athlete(&resources, "upload-erasure").await;
    let upload = FitWorkout::ride(ten_days_ago(), 600).encode();
    let sha = file_sha256(&upload);
    resources
        .common
        .originals
        .put_upload(&athlete.tenant, athlete.user_id, &sha, Bytes::from(upload))
        .await
        .expect("the upload is kept");

    let removal = remove_user(
        &resources.common.repos,
        None,
        None,
        None,
        athlete.user_id,
        DeletionActor::Athlete,
    )
    .await
    .expect("the removal runs");
    assert!(matches!(removal, UserRemoval::Removed(_)), "{removal:?}");
    let erased = erase_deleted_accounts(&resources)
        .await
        .expect("bug: the erasure must run where uploads are kept, imports or not");
    assert_eq!(erased, 1, "the account queued by its delete is erased");
    assert_eq!(
        erase_deleted_accounts(&resources)
            .await
            .expect("the sweep runs"),
        0,
        "an erased account leaves the outbox"
    );
}

#[tokio::test]
async fn imported_rows_are_never_pruned_by_a_provider_sync() {
    let (imports, _) = imports_serving(strava_export());
    let resources = common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context");
    let athlete = seed_athlete(&resources, "archive-prune").await;
    import_export(&resources, &athlete).await;

    let pruned = resources
        .common
        .repos
        .activity_cache
        .prune_activities_before(athlete.user_id, &athlete.tenant, STRAVA_ARCHIVE, Utc::now())
        .await
        .expect("the prune runs");

    assert_eq!(
        pruned, 0,
        "the athlete's own copy is the record, never a cache to prune"
    );
}

#[tokio::test]
async fn a_server_without_imports_says_so() {
    let resources = common::create_test_server_resources()
        .await
        .expect("a server context");
    let athlete = seed_athlete(&resources, "archive-off").await;

    let (status, overview) = call(&resources, "GET", "/api/imports", &athlete.token, None).await;
    assert_eq!(status, StatusCode::OK, "{overview}");
    assert_eq!(overview["enabled"], false, "{overview}");
    let (start, body) = call(
        &resources,
        "POST",
        "/api/imports/strava_archive",
        &athlete.token,
        Some(json!({ "url": LINK })),
    )
    .await;
    assert_eq!(start, StatusCode::SERVICE_UNAVAILABLE, "{body}");
}

/// `needs_provider_connection` as the onboarding gate reads it.
async fn needs_provider_connection(resources: &Arc<ServerContext>, athlete: &Athlete) -> bool {
    let request = Request::builder()
        .uri("/api/me/onboarding-status")
        .header("authorization", format!("Bearer {}", athlete.token))
        .body(Body::empty())
        .expect("a request");
    let response = OnboardingRoutes::routes(Arc::clone(resources))
        .oneshot(request)
        .await
        .expect("the router answers");
    let bytes = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a body");
    let body: Value = serde_json::from_slice(&bytes).expect("a JSON body");
    body["needs_provider_connection"]
        .as_bool()
        .expect("the gate answers")
}

#[tokio::test]
async fn an_athlete_who_imported_their_history_passes_the_connect_step() {
    let (imports, _) = imports_serving(strava_export());
    let resources = common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context");
    let athlete = seed_athlete(&resources, "archive-onboarding").await;
    assert!(
        needs_provider_connection(&resources, &athlete).await,
        "an athlete with nothing connected starts at the connect step"
    );

    let (_, waiting) = call(
        &resources,
        "POST",
        "/api/imports/strava_archive",
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(waiting["status"], "awaiting_link", "{waiting}");
    assert!(
        needs_provider_connection(&resources, &athlete).await,
        "an import still waiting for its link brings nothing in yet"
    );

    import_export(&resources, &athlete).await;
    assert!(
        !needs_provider_connection(&resources, &athlete).await,
        "a completed import answers the connect step as a connection does"
    );
}

#[tokio::test]
async fn the_web_apps_own_session_manages_imports() {
    let (imports, _) = imports_serving(strava_export());
    let resources = common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context");
    let email = format!("archive-web-{}@example.com", Uuid::new_v4());
    let (_, user, tenant) =
        common::create_test_user_with_plan(&resources.agent.database, &email, "starter")
            .await
            .expect("the athlete is created");
    // The session the web app redeems its sign-in code for names its
    // first-party client; it is the athlete's own credential all the same.
    let web_session = resources
        .auth
        .auth_manager
        .generate_client_session_token(
            &user,
            &resources.auth.jwks_manager,
            Some(tenant.to_string()),
            Some("dravr-web"),
        )
        .expect("a web session");

    let (status, overview) = call(&resources, "GET", "/api/imports", &web_session, None).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "the web app's session reads its imports: {overview}"
    );
    assert_eq!(overview["enabled"], true, "{overview}");
}
