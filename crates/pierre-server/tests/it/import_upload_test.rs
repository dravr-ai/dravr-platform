// ABOUTME: The upload intake's routes: open, read back, write through the API, finish, cancel, and the sweeps behind them
// ABOUTME: Runs on barrage's memory stores, with a scripted store standing in for GCS where a session or a size matters

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! An Apple Health export is uploaded rather than linked. These tests drive
//! `/api/imports/apple_health_archive` as the clients do. No test here reaches
//! real GCS: the scripted store only answers what a session would.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::convert::Infallible;
use std::io::{Cursor, Write};
use std::iter;
use std::ops::Range;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use bytes::Bytes;
use chrono::{Duration, Utc};
use dravr_barrage::blob::memory::MemoryBlobStore;
use dravr_barrage::blob::{
    BlobKey, BlobMeta, BlobPrefix, BlobStat, BlobStore, DeletePage, PageToken, UploadTarget,
};
use dravr_barrage::store::memory::{MemoryActivityRecordStore, MemoryImportJobStore};
use dravr_barrage::store::{BarrageStores, NewImportJob};
use dravr_barrage::{BarrageError, FailureCode, ImportId, JobStatus, Scope, SourceKey};
use futures_util::stream::{self, BoxStream};
use futures_util::StreamExt;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::imports::import_routes;
use pierre_mcp_server::services::archive_import::runner::{
    ImportRunner, ImportTasks, ImportTasksConfig,
};
use pierre_mcp_server::services::archive_import::sweeps::expire_awaiting_links;
use pierre_mcp_server::services::archive_import::{ArchiveImports, ImportWiring};
use pierre_middleware::provider_link_token::{
    mint_connect_link_token, mint_import_link_token, verify_link_token, CONNECT_PROVIDER,
};
use serde_json::{json, Value};
use tokio::time::sleep;
use tower::ServiceExt;
use uuid::Uuid;
use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::archive_import_test::{
    call, imports_serving, seed_athlete, strava_export, Athlete, LINK,
};
use crate::common;
use crate::helpers::cloud_tasks_stub::{
    NamedQueueStub, StaticToken, QUEUE, SERVICE_ACCOUNT, TARGET,
};

const SOURCE: &str = "apple_health_archive";
const START: &str = "/api/imports/apple_health_archive";
const SESSION_URL: &str = "https://storage.example.com/upload/session-secret-token";
/// One byte past barrage's archive cap.
const OVER_CAP: u64 = (16 << 30) + 1;

/// A store that answers as GCS does: a session, and any size a test scripts.
struct SessionBlobs {
    inner: MemoryBlobStore,
    scripted_size: AtomicU64,
    opened: Mutex<Vec<(BlobKey, u64, Option<String>)>>,
}

impl SessionBlobs {
    fn new() -> Self {
        Self {
            inner: MemoryBlobStore::new(),
            scripted_size: AtomicU64::new(0),
            opened: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl BlobStore for SessionBlobs {
    async fn put_stream(
        &self,
        key: &BlobKey,
        body: BoxStream<'_, Result<Bytes, BarrageError>>,
        meta: &BlobMeta,
    ) -> Result<BlobStat, BarrageError> {
        self.inner.put_stream(key, body, meta).await
    }

    async fn put(
        &self,
        key: &BlobKey,
        bytes: Bytes,
        meta: &BlobMeta,
    ) -> Result<BlobStat, BarrageError> {
        self.inner.put(key, bytes, meta).await
    }

    async fn open_upload(
        &self,
        key: &BlobKey,
        _meta: &BlobMeta,
        length: u64,
        origin: Option<&str>,
    ) -> Result<UploadTarget, BarrageError> {
        self.opened
            .lock()
            .unwrap()
            .push((*key, length, origin.map(str::to_owned)));
        Ok(UploadTarget::Session(SESSION_URL.to_owned()))
    }

    async fn get(&self, key: &BlobKey) -> Result<Option<Bytes>, BarrageError> {
        self.inner.get(key).await
    }

    async fn get_range(&self, key: &BlobKey, range: Range<u64>) -> Result<Bytes, BarrageError> {
        self.inner.get_range(key, range).await
    }

    async fn size(&self, key: &BlobKey) -> Result<Option<u64>, BarrageError> {
        match self.scripted_size.load(Ordering::SeqCst) {
            0 => self.inner.size(key).await,
            size => Ok(Some(size)),
        }
    }

    async fn delete(&self, key: &BlobKey) -> Result<(), BarrageError> {
        self.inner.delete(key).await
    }

    async fn delete_prefix_page(
        &self,
        prefix: &BlobPrefix,
        token: Option<PageToken>,
    ) -> Result<DeletePage, BarrageError> {
        self.inner.delete_prefix_page(prefix, token).await
    }
}

/// A server on barrage's memory stores: its store takes no direct writes.
async fn through_api_server() -> (Arc<ServerContext>, BarrageStores) {
    let (imports, stores) = imports_serving(Vec::new());
    let resources = common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context");
    (resources, stores)
}

/// A server whose store hands out sessions, as GCS does.
async fn session_server() -> (Arc<ServerContext>, BarrageStores, Arc<SessionBlobs>) {
    let blobs = Arc::new(SessionBlobs::new());
    let (imports, stores) = imports_over(blobs.clone());
    let resources = common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context");
    (resources, stores, blobs)
}

fn imports_over(blobs: Arc<dyn BlobStore>) -> (ArchiveImports, BarrageStores) {
    imports_running(blobs, ImportRunner::InProcess)
}

fn imports_running(
    blobs: Arc<dyn BlobStore>,
    runner: ImportRunner,
) -> (ArchiveImports, BarrageStores) {
    let stores = BarrageStores {
        blobs,
        records: Arc::new(MemoryActivityRecordStore::new()),
        jobs: Arc::new(MemoryImportJobStore::new()),
    };
    let (seed, _) = imports_serving(Vec::new());
    let fetcher = seed.wiring().expect("wired").fetcher.clone();
    let imports = ArchiveImports::new(ImportWiring {
        stores: stores.clone(),
        fetcher,
        runner,
    });
    (imports, stores)
}

/// A server whose store hands out sessions, delivering steps through `queue`,
/// with an object already landed for every upload.
async fn queued_server(queue: &Arc<NamedQueueStub>) -> Arc<ServerContext> {
    let tasks = ImportTasks::new(
        ImportTasksConfig {
            queue: QUEUE.to_owned(),
            target_url: TARGET.to_owned(),
            service_account: SERVICE_ACCOUNT.to_owned(),
            api_base: queue.serve().await,
        },
        Arc::new(StaticToken),
    );
    let blobs = Arc::new(SessionBlobs::new());
    blobs.scripted_size.store(1024, Ordering::SeqCst);
    let (imports, _stores) = imports_running(blobs, ImportRunner::CloudTasks(Box::new(tasks)));
    common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context")
}

fn scope_of(athlete: &Athlete) -> Scope {
    Scope::new(athlete.tenant.as_uuid(), athlete.user_id)
}

fn uri(id: &str, tail: &str) -> String {
    format!("{START}/{id}/{tail}")
}

/// A request's status and JSON body.
async fn send(resources: &Arc<ServerContext>, request: Request<Body>) -> (StatusCode, Value) {
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

/// A `POST` start carrying `origin`.
fn start_with_origin(token: &str, origin: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(START)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .header("origin", origin)
        .body(Body::from(json!({ "length": 1024 }).to_string()))
        .expect("a request")
}

/// `bytes` written through the API.
async fn write(
    resources: &Arc<ServerContext>,
    token: &str,
    id: &str,
    bytes: Vec<u8>,
) -> StatusCode {
    let request = Request::builder()
        .method("PUT")
        .uri(uri(id, "upload"))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::from(bytes))
        .expect("a request");
    send(resources, request).await.0
}

/// Open an upload; its id and the full body.
async fn open(resources: &Arc<ServerContext>, token: &str, length: u64) -> (String, Value) {
    let (status, body) = call(
        resources,
        "POST",
        START,
        token,
        Some(json!({ "length": length })),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    (body["id"].as_str().expect("an id").to_owned(), body)
}

/// A one-workout Apple Health export, synthetic and hand-reduced.
fn apple_export() -> Vec<u8> {
    let start = (Utc::now() - Duration::days(30)).format("%Y-%m-%d %H:%M:%S +0200");
    let end =
        (Utc::now() - Duration::days(30) + Duration::minutes(30)).format("%Y-%m-%d %H:%M:%S +0200");
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE HealthData [\n<!ELEMENT HealthData (Workout)*>\n]>\n\
         <HealthData locale=\"en_US\">\n\
         <Workout workoutActivityType=\"HKWorkoutActivityTypeRunning\" duration=\"30\" \
         durationUnit=\"min\" sourceName=\"Synthetic Watch\" startDate=\"{start}\" endDate=\"{end}\"/>\n\
         </HealthData>\n"
    );
    let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(
        "apple_health_export/export.xml",
        SimpleFileOptions::default(),
    )
    .expect("a member");
    zip.write_all(xml.as_bytes()).expect("written");
    zip.finish().expect("a zip").into_inner()
}

async fn follow_until_final(resources: &Arc<ServerContext>, token: &str) -> Value {
    for _ in 0..200 {
        let (status, body) = call(resources, "GET", &format!("{START}/current"), token, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let state = body["import"]["status"].as_str().unwrap_or_default();
        if ["completed", "failed", "cancelled"].contains(&state) {
            return body["import"].clone();
        }
        sleep(StdDuration::from_millis(25)).await;
    }
    panic!("the import did not finish");
}

#[tokio::test]
async fn an_upload_through_the_api_imports_end_to_end() {
    let (resources, _stores) = through_api_server().await;
    let athlete = seed_athlete(&resources, "upload-e2e").await;
    let export = apple_export();

    let (id, started) = open(&resources, &athlete.token, export.len() as u64).await;
    assert_eq!(started["status"], "awaiting_link", "{started}");
    assert_eq!(started["upload"]["kind"], "through_api", "{started}");

    let (status, target) = call(
        &resources,
        "GET",
        &uri(&id, "upload-session"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{target}");
    assert_eq!(target["kind"], "through_api", "{target}");

    assert_eq!(
        write(&resources, &athlete.token, &id, export).await,
        StatusCode::NO_CONTENT
    );
    let (status, accepted) = call(
        &resources,
        "POST",
        &uri(&id, "uploaded"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{accepted}");

    let finished = follow_until_final(&resources, &athlete.token).await;
    assert_eq!(finished["status"], "completed", "{finished}");
    assert_eq!(finished["imported"], 1, "{finished}");
    assert_eq!(finished["source"], SOURCE, "{finished}");
}

#[tokio::test]
async fn a_session_is_opened_for_its_declared_length_sealed_and_read_back_by_its_owner() {
    let (resources, stores, blobs) = session_server().await;
    let athlete = seed_athlete(&resources, "upload-session").await;
    let (status, started) = send(
        &resources,
        start_with_origin(&athlete.token, "https://app.example.com"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{started}");
    assert_eq!(started["upload"]["kind"], "session", "{started}");
    assert_eq!(started["upload"]["url"], SESSION_URL, "{started}");
    let id = started["id"].as_str().unwrap().to_owned();

    {
        let opened = blobs.opened.lock().unwrap();
        assert_eq!(opened.len(), 1, "one session opened");
        assert_eq!(opened[0].1, 1024, "the declared length");
        assert_eq!(opened[0].2.as_deref(), Some("https://app.example.com"));
    }

    let job = stores
        .jobs
        .get(&scope_of(&athlete), ImportId(Uuid::parse_str(&id).unwrap()))
        .await
        .unwrap()
        .expect("the job");
    let sealed = job.url_ciphertext.expect("the URL is sealed");
    assert!(
        !String::from_utf8_lossy(&sealed).contains("session-secret-token"),
        "the job row holds a ciphertext"
    );

    let (status, target) = call(
        &resources,
        "GET",
        &uri(&id, "upload-session"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{target}");
    assert_eq!(target["url"], SESSION_URL, "its owner resumes with it");

    let (_, current) = call(
        &resources,
        "GET",
        &format!("{START}/current"),
        &athlete.token,
        None,
    )
    .await;
    assert!(
        !current.to_string().contains("session-secret-token"),
        "no other route serves the URL: {current}"
    );
    let (_, overview) = call(&resources, "GET", "/api/imports", &athlete.token, None).await;
    assert!(
        !overview.to_string().contains("session-secret-token"),
        "{overview}"
    );
}

#[tokio::test]
async fn the_session_answers_are_kept_out_of_caches() {
    let (resources, _stores, _blobs) = session_server().await;
    let athlete = seed_athlete(&resources, "upload-no-store").await;
    let response = import_routes()
        .with_state(Arc::clone(&resources))
        .oneshot(start_with_origin(&athlete.token, "https://app.example.com"))
        .await
        .unwrap();
    assert_eq!(
        response.headers().get("cache-control").unwrap(),
        "no-store",
        "bug: a session URL must stay out of every cache"
    );
}

#[tokio::test]
async fn another_athlete_cannot_reach_an_upload_in_any_way() {
    let (resources, stores, _blobs) = session_server().await;
    let owner = seed_athlete(&resources, "upload-owner").await;
    let other = seed_athlete(&resources, "upload-other").await;
    let (id, _) = open(&resources, &owner.token, 1024).await;

    let (status, body) = call(
        &resources,
        "GET",
        &uri(&id, "upload-session"),
        &other.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert!(!body.to_string().contains("session-secret-token"), "{body}");

    let (status, _) = call(
        &resources,
        "POST",
        &uri(&id, "uploaded"),
        &other.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "finishing");
    assert_eq!(
        write(&resources, &other.token, &id, b"PK".to_vec()).await,
        StatusCode::NOT_FOUND,
        "writing"
    );
    let (status, _) = call(&resources, "POST", &uri(&id, "cancel"), &other.token, None).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "cancelling");

    let job = stores
        .jobs
        .get(&scope_of(&owner), ImportId(Uuid::parse_str(&id).unwrap()))
        .await
        .unwrap()
        .expect("the job");
    assert_eq!(
        job.status.as_str(),
        "awaiting_link",
        "the owner's job is untouched"
    );
}

#[tokio::test]
async fn an_upload_is_not_reached_through_another_sources_routes() {
    let (resources, stores, _blobs) = session_server().await;
    let athlete = seed_athlete(&resources, "upload-other-source").await;
    let (id, _) = open(&resources, &athlete.token, 1024).await;
    let elsewhere = |tail: &str| format!("/api/imports/garmin_archive/{id}/{tail}");

    let (status, body) = call(
        &resources,
        "GET",
        &elsewhere("upload-session"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "reading back: {body}");
    let (status, body) = call(
        &resources,
        "POST",
        &elsewhere("uploaded"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "finishing: {body}");
    let (status, body) = call(
        &resources,
        "POST",
        &elsewhere("cancel"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "cancelling: {body}");

    let job = stores
        .jobs
        .get(&scope_of(&athlete), ImportId(Uuid::parse_str(&id).unwrap()))
        .await
        .unwrap()
        .expect("the job");
    assert_eq!(job.status.as_str(), "awaiting_link", "the job is untouched");
}

#[tokio::test]
async fn the_same_account_reaches_no_upload_it_holds_in_another_tenant() {
    let (resources, stores, _blobs) = session_server().await;
    let athlete = seed_athlete(&resources, "upload-tenant").await;
    let elsewhere = Scope::new(Uuid::new_v4(), athlete.user_id);
    let job = stores
        .jobs
        .create(
            &elsewhere,
            NewImportJob {
                source: SourceKey::AppleHealthArchive,
                status: JobStatus::AwaitingLink,
                url_sha256: None,
                url_ciphertext: None,
                spool: Some(BlobKey::spool(elsewhere)),
                expires_at: Some(Utc::now() + Duration::days(1)),
            },
        )
        .await
        .expect("a job in another tenant");
    let id = job.id.0.to_string();

    let (status, body) = call(
        &resources,
        "GET",
        &uri(&id, "upload-session"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    let (status, _) = call(
        &resources,
        "POST",
        &uri(&id, "uploaded"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "finishing");
    assert_eq!(
        write(&resources, &athlete.token, &id, b"PK".to_vec()).await,
        StatusCode::NOT_FOUND,
        "writing"
    );
}

#[tokio::test]
async fn a_session_is_served_only_while_the_import_waits_for_it() {
    let (resources, _stores) = through_api_server().await;
    let athlete = seed_athlete(&resources, "upload-status").await;
    let (id, _) = open(&resources, &athlete.token, 1024).await;

    let (status, _) = call(
        &resources,
        "POST",
        &uri(&id, "cancel"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the cancel is taken");

    let (status, body) = call(
        &resources,
        "GET",
        &uri(&id, "upload-session"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    let (status, body) = call(
        &resources,
        "POST",
        &uri(&id, "uploaded"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        write(&resources, &athlete.token, &id, b"PK".to_vec()).await,
        StatusCode::CONFLICT,
        "a cancelled upload takes no bytes"
    );
}

#[tokio::test]
async fn finishing_before_the_object_landed_keeps_the_session_alive() {
    let (resources, _stores, _blobs) = session_server().await;
    let athlete = seed_athlete(&resources, "upload-early").await;
    let (id, _) = open(&resources, &athlete.token, 1024).await;

    let (status, body) = call(
        &resources,
        "POST",
        &uri(&id, "uploaded"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");

    let (status, target) = call(
        &resources,
        "GET",
        &uri(&id, "upload-session"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "still resumable: {target}");
}

/// Cloud Tasks keeps a deleted task's name reserved for up to a day, so a create
/// under it answers 409 with nothing queued. The upload still waits and its
/// object has landed, so the athlete's retry is the only thing that can start it.
#[tokio::test]
async fn finishing_again_after_the_queue_spent_its_retries_schedules_the_read_again() {
    let queue = NamedQueueStub::new();
    let resources = queued_server(&queue).await;
    let athlete = seed_athlete(&resources, "upload-spent").await;
    let (id, _) = open(&resources, &athlete.token, 1024).await;
    let uploaded = uri(&id, "uploaded");
    let finish = || call(&resources, "POST", &uploaded, &athlete.token, None);

    let (status, body) = finish().await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let first = queue.created();
    assert_eq!(first.len(), 1, "the read is scheduled once: {first:?}");

    let (status, body) = finish().await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(
        queue.created(),
        first,
        "a second finish while the task lives adds nothing: a pass is costly"
    );

    queue.spend_all();
    assert_eq!(queue.live().len(), 0);
    let (status, body) = finish().await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let live = queue.live();
    assert_eq!(
        live.len(),
        1,
        "bug: answered 202 with nothing queued, so the job waits until it expires"
    );
    assert_ne!(live, first, "the spent task's name stays reserved");

    let (status, body) = finish().await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(queue.live(), live, "the new task is deduplicated as well");

    queue.spend_all();
    let (status, body) = finish().await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    assert_eq!(
        queue.live().len(),
        1,
        "the retry works every time it is needed"
    );
    assert_eq!(queue.created().len(), 3);
}

/// A queue that will not say whether the task lives must not read as started.
#[tokio::test]
async fn finishing_again_when_the_queue_will_not_answer_is_not_called_started() {
    let queue = NamedQueueStub::new();
    let resources = queued_server(&queue).await;
    let athlete = seed_athlete(&resources, "upload-unread").await;
    let (id, _) = open(&resources, &athlete.token, 1024).await;
    let (status, body) = call(
        &resources,
        "POST",
        &uri(&id, "uploaded"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");

    queue.spend_all();
    queue.refuse_reads(StatusCode::FORBIDDEN);
    let (status, body) = call(
        &resources,
        "POST",
        &uri(&id, "uploaded"),
        &athlete.token,
        None,
    )
    .await;
    assert!(
        status.is_server_error(),
        "bug: {status} for a read nothing confirmed was scheduled: {body}"
    );
}

#[tokio::test]
async fn a_declared_length_past_the_cap_is_refused_before_a_session_opens() {
    let (resources, _stores, blobs) = session_server().await;
    let athlete = seed_athlete(&resources, "upload-oversize").await;
    let (status, body) = call(
        &resources,
        "POST",
        START,
        &athlete.token,
        Some(json!({ "length": OVER_CAP })),
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");
    assert_eq!(body["error_code"], "archive_too_large", "{body}");
    assert!(blobs.opened.lock().unwrap().is_empty(), "no session opened");
}

#[tokio::test]
async fn an_object_past_the_cap_is_refused_deleted_and_fails_the_import() {
    let (resources, stores, blobs) = session_server().await;
    let athlete = seed_athlete(&resources, "upload-landed-oversize").await;
    let (id, _) = open(&resources, &athlete.token, 1024).await;
    let key = blobs.opened.lock().unwrap()[0].0;
    blobs
        .inner
        .put(
            &key,
            Bytes::from_static(b"PK"),
            &BlobMeta::new("application/zip"),
        )
        .await
        .unwrap();
    blobs.scripted_size.store(OVER_CAP, Ordering::SeqCst);

    let (status, body) = call(
        &resources,
        "POST",
        &uri(&id, "uploaded"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body}");

    let job = stores
        .jobs
        .get(&scope_of(&athlete), ImportId(Uuid::parse_str(&id).unwrap()))
        .await
        .unwrap()
        .expect("the job");
    assert_eq!(job.status.as_str(), "failed");
    assert_eq!(
        job.error_code.map(FailureCode::as_str),
        Some("archive_too_large")
    );
    blobs.scripted_size.store(0, Ordering::SeqCst);
    assert_eq!(
        blobs.inner.size(&key).await.unwrap(),
        None,
        "the object went"
    );
}

#[tokio::test]
async fn a_write_declaring_more_than_the_cap_is_refused_before_it_is_read() {
    let (resources, _stores) = through_api_server().await;
    let athlete = seed_athlete(&resources, "upload-put-oversize").await;
    let (id, _) = open(&resources, &athlete.token, 1024).await;
    let request = Request::builder()
        .method("PUT")
        .uri(uri(&id, "upload"))
        .header("authorization", format!("Bearer {}", athlete.token))
        .header("content-length", OVER_CAP.to_string())
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        send(&resources, request).await.0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
}

/// A store that takes no direct writes and reads a stream without keeping it,
/// so a body past the 16 GiB cap costs no memory.
struct DrainingBlobs {
    inner: MemoryBlobStore,
    drained: AtomicU64,
}

#[async_trait]
impl BlobStore for DrainingBlobs {
    async fn put_stream(
        &self,
        key: &BlobKey,
        mut body: BoxStream<'_, Result<Bytes, BarrageError>>,
        meta: &BlobMeta,
    ) -> Result<BlobStat, BarrageError> {
        while let Some(chunk) = body.next().await {
            self.drained
                .fetch_add(chunk?.len() as u64, Ordering::SeqCst);
        }
        self.inner.put(key, Bytes::new(), meta).await
    }

    async fn put(
        &self,
        key: &BlobKey,
        bytes: Bytes,
        meta: &BlobMeta,
    ) -> Result<BlobStat, BarrageError> {
        self.inner.put(key, bytes, meta).await
    }

    async fn open_upload(
        &self,
        key: &BlobKey,
        meta: &BlobMeta,
        length: u64,
        origin: Option<&str>,
    ) -> Result<UploadTarget, BarrageError> {
        self.inner.open_upload(key, meta, length, origin).await
    }

    async fn get(&self, key: &BlobKey) -> Result<Option<Bytes>, BarrageError> {
        self.inner.get(key).await
    }

    async fn get_range(&self, key: &BlobKey, range: Range<u64>) -> Result<Bytes, BarrageError> {
        self.inner.get_range(key, range).await
    }

    async fn size(&self, key: &BlobKey) -> Result<Option<u64>, BarrageError> {
        self.inner.size(key).await
    }

    async fn delete(&self, key: &BlobKey) -> Result<(), BarrageError> {
        self.inner.delete(key).await
    }

    async fn delete_prefix_page(
        &self,
        prefix: &BlobPrefix,
        token: Option<PageToken>,
    ) -> Result<DeletePage, BarrageError> {
        self.inner.delete_prefix_page(prefix, token).await
    }
}

#[tokio::test]
async fn a_write_past_the_cap_is_stopped_whatever_length_it_declares() {
    const GIB: usize = 1 << 30;
    let chunk = Bytes::from(vec![0_u8; GIB]);
    for declared in [None, Some("2")] {
        let blobs = Arc::new(DrainingBlobs {
            inner: MemoryBlobStore::new(),
            drained: AtomicU64::new(0),
        });
        let (imports, stores) = imports_over(blobs.clone());
        let resources = common::create_test_server_resources_with_archive_imports(imports)
            .await
            .expect("a server context");
        let athlete = seed_athlete(&resources, "upload-put-lying").await;
        let (id, _) = open(&resources, &athlete.token, 2).await;
        let chunks = iter::repeat_n(chunk.clone(), 32).map(Ok::<_, Infallible>);
        let mut request = Request::builder()
            .method("PUT")
            .uri(uri(&id, "upload"))
            .header("authorization", format!("Bearer {}", athlete.token));
        if let Some(declared) = declared {
            request = request.header("content-length", declared);
        }
        let request = request
            .body(Body::from_stream(stream::iter(chunks)))
            .unwrap();

        assert_eq!(
            send(&resources, request).await.0,
            StatusCode::PAYLOAD_TOO_LARGE,
            "bug: a body past the cap is taken when it declares {declared:?}"
        );
        assert!(
            blobs.drained.load(Ordering::SeqCst) <= 16 << 30,
            "bug: the store is handed bytes past the cap when the body declares {declared:?}"
        );
        let job = stores
            .jobs
            .get(&scope_of(&athlete), ImportId(Uuid::parse_str(&id).unwrap()))
            .await
            .unwrap()
            .expect("the job");
        let spool = job.spool.expect("the job holds its spool");
        assert_eq!(
            blobs.inner.size(&spool).await.unwrap(),
            None,
            "bug: a refused body leaves an object"
        );
        assert_eq!(
            job.status.as_str(),
            "awaiting_link",
            "the upload still waits"
        );
    }
}

#[tokio::test]
async fn a_session_import_takes_no_write_through_the_api() {
    let (resources, _stores, _blobs) = session_server().await;
    let athlete = seed_athlete(&resources, "upload-put-session").await;
    let (id, _) = open(&resources, &athlete.token, 1024).await;
    assert_eq!(
        write(&resources, &athlete.token, &id, b"PK".to_vec()).await,
        StatusCode::CONFLICT,
    );
}

#[tokio::test]
async fn a_second_write_over_a_landed_file_is_refused() {
    let (resources, _stores) = through_api_server().await;
    let athlete = seed_athlete(&resources, "upload-put-twice").await;
    let (id, _) = open(&resources, &athlete.token, 2).await;
    assert_eq!(
        write(&resources, &athlete.token, &id, b"PK".to_vec()).await,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        write(&resources, &athlete.token, &id, b"PK".to_vec()).await,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn a_second_upload_while_one_waits_opens_no_session() {
    let (resources, _stores, blobs) = session_server().await;
    let athlete = seed_athlete(&resources, "upload-conflict").await;
    open(&resources, &athlete.token, 1024).await;
    let (status, body) = call(
        &resources,
        "POST",
        START,
        &athlete.token,
        Some(json!({ "length": 1024 })),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(blobs.opened.lock().unwrap().len(), 1, "no second session");
}

#[tokio::test]
async fn a_malformed_origin_is_refused_and_a_native_client_names_none() {
    let (resources, _stores, blobs) = session_server().await;
    let athlete = seed_athlete(&resources, "upload-origin").await;
    let (status, body) = send(
        &resources,
        start_with_origin(&athlete.token, "not an origin"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(blobs.opened.lock().unwrap().is_empty(), "no session opened");

    open(&resources, &athlete.token, 1024).await;
    assert_eq!(blobs.opened.lock().unwrap()[0].2, None, "a native client");
}

#[tokio::test]
async fn an_upload_takes_no_link_and_needs_its_length() {
    let (resources, _stores) = through_api_server().await;
    let athlete = seed_athlete(&resources, "upload-body").await;
    for body in [
        json!({ "url": LINK }),
        json!({ "url": LINK, "length": 10 }),
        json!({}),
        json!({ "length": 0 }),
    ] {
        let (status, answer) = call(
            &resources,
            "POST",
            START,
            &athlete.token,
            Some(body.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {answer}");
    }
    let (status, _) = call(
        &resources,
        "POST",
        &format!("{START}/check"),
        &athlete.token,
        Some(json!({ "url": LINK })),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "no link to check");
}

#[tokio::test]
async fn every_upload_route_answers_not_found_for_a_source_not_offered() {
    let (resources, _stores) = through_api_server().await;
    let athlete = seed_athlete(&resources, "upload-unoffered").await;
    let id = Uuid::new_v4();
    let source = "fitbit_archive";
    let (status, _) = call(
        &resources,
        "POST",
        &format!("/api/imports/{source}"),
        &athlete.token,
        Some(json!({ "length": 10 })),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "start");
    let (status, _) = call(
        &resources,
        "GET",
        &format!("/api/imports/{source}/{id}/upload-session"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "session");
    let (status, _) = call(
        &resources,
        "POST",
        &format!("/api/imports/{source}/{id}/uploaded"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "finish");
    let (status, _) = call(
        &resources,
        "PUT",
        &format!("/api/imports/{source}/{id}/upload"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "write");
}

#[tokio::test]
async fn a_link_import_is_no_upload() {
    let (imports, _stores) = imports_serving(strava_export());
    let resources = common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context");
    let athlete = seed_athlete(&resources, "upload-link-job").await;
    let (status, waiting) = call(
        &resources,
        "POST",
        "/api/imports/strava_archive",
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{waiting}");
    let id = waiting["id"].as_str().unwrap();
    let base = format!("/api/imports/strava_archive/{id}");
    let (status, _) = call(
        &resources,
        "GET",
        &format!("{base}/upload-session"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "session");
    let (status, _) = call(
        &resources,
        "POST",
        &format!("{base}/uploaded"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "finish");
}

#[tokio::test]
async fn cancelling_removes_the_object_and_the_sweep_catches_a_late_completion() {
    let (resources, _stores, blobs) = session_server().await;
    let athlete = seed_athlete(&resources, "upload-cancel").await;
    let (id, _) = open(&resources, &athlete.token, 2).await;
    let key = blobs.opened.lock().unwrap()[0].0;
    blobs
        .inner
        .put(
            &key,
            Bytes::from_static(b"PK"),
            &BlobMeta::new("application/zip"),
        )
        .await
        .unwrap();

    let (status, _) = call(
        &resources,
        "POST",
        &uri(&id, "cancel"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the cancel is taken");
    assert_eq!(
        blobs.inner.size(&key).await.unwrap(),
        None,
        "cancel deleted it"
    );

    // The live session completes after the cancel: only the sweep sees that.
    blobs
        .inner
        .put(
            &key,
            Bytes::from_static(b"PK"),
            &BlobMeta::new("application/zip"),
        )
        .await
        .unwrap();
    expire_awaiting_links(&resources).await.expect("the sweep");
    assert_eq!(
        blobs.inner.size(&key).await.unwrap(),
        None,
        "the sweep deleted it"
    );
}

/// A store whose reads fail until told otherwise, so an adoption is retryable.
struct FlakyReads {
    inner: MemoryBlobStore,
    failing: AtomicU64,
}

#[async_trait]
impl BlobStore for FlakyReads {
    async fn put_stream(
        &self,
        key: &BlobKey,
        body: BoxStream<'_, Result<Bytes, BarrageError>>,
        meta: &BlobMeta,
    ) -> Result<BlobStat, BarrageError> {
        self.inner.put_stream(key, body, meta).await
    }

    async fn put(
        &self,
        key: &BlobKey,
        bytes: Bytes,
        meta: &BlobMeta,
    ) -> Result<BlobStat, BarrageError> {
        self.inner.put(key, bytes, meta).await
    }

    async fn open_upload(
        &self,
        key: &BlobKey,
        meta: &BlobMeta,
        length: u64,
        origin: Option<&str>,
    ) -> Result<UploadTarget, BarrageError> {
        self.inner.open_upload(key, meta, length, origin).await
    }

    async fn get(&self, key: &BlobKey) -> Result<Option<Bytes>, BarrageError> {
        self.inner.get(key).await
    }

    async fn get_range(&self, key: &BlobKey, range: Range<u64>) -> Result<Bytes, BarrageError> {
        if self.failing.load(Ordering::SeqCst) > 0 {
            self.failing.fetch_sub(1, Ordering::SeqCst);
            return Err(BarrageError::Blob("scripted read failure".to_owned()));
        }
        self.inner.get_range(key, range).await
    }

    async fn size(&self, key: &BlobKey) -> Result<Option<u64>, BarrageError> {
        self.inner.size(key).await
    }

    async fn delete(&self, key: &BlobKey) -> Result<(), BarrageError> {
        self.inner.delete(key).await
    }

    async fn delete_prefix_page(
        &self,
        prefix: &BlobPrefix,
        token: Option<PageToken>,
    ) -> Result<DeletePage, BarrageError> {
        self.inner.delete_prefix_page(prefix, token).await
    }
}

#[tokio::test]
async fn a_retryable_failure_reading_the_upload_keeps_it_and_the_step_finishes_it() {
    let blobs = Arc::new(FlakyReads {
        inner: MemoryBlobStore::new(),
        failing: AtomicU64::new(1),
    });
    let (imports, _stores) = imports_over(blobs);
    let resources = common::create_test_server_resources_with_archive_imports(imports)
        .await
        .expect("a server context");
    let athlete = seed_athlete(&resources, "upload-retry").await;
    let export = apple_export();
    let (id, _) = open(&resources, &athlete.token, export.len() as u64).await;
    assert_eq!(
        write(&resources, &athlete.token, &id, export).await,
        StatusCode::NO_CONTENT
    );

    let (status, body) = call(
        &resources,
        "POST",
        &uri(&id, "uploaded"),
        &athlete.token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");

    // The in-process runner tries the step again after its first failure; the
    // spool has to survive that for the import to finish at all.
    let finished = follow_until_final(&resources, &athlete.token).await;
    assert_eq!(finished["status"], "completed", "{finished}");
    assert_eq!(finished["imported"], 1, "{finished}");
}

#[tokio::test]
async fn the_hosted_chat_path_starts_no_upload() {
    let (resources, _stores, blobs) = session_server().await;
    let athlete = seed_athlete(&resources, "upload-hosted").await;
    let connect = mint_connect_link_token(
        athlete.user_id,
        athlete.tenant.as_uuid(),
        "telegram",
        None,
        &resources.auth.admin_jwt_secret,
    )
    .expect("a connect token");
    let claims = verify_link_token(&connect, &resources.auth.admin_jwt_secret, CONNECT_PROVIDER)
        .expect("the connect token verifies");
    let token = mint_import_link_token(&claims, SOURCE, &resources.auth.admin_jwt_secret)
        .expect("an import token");

    let (status, body) = call(
        &resources,
        "POST",
        "/providers/connect/import/apple_health_archive",
        &token,
        Some(json!({ "length": 1024 })),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "chat athletes do this in the app: {body}"
    );
    assert!(blobs.opened.lock().unwrap().is_empty(), "no session opened");
}
