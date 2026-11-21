// ABOUTME: carnet#1541 — the originals bundle: asked for, built, downloaded and unzipped, its manifest and checksums verified
// ABOUTME: Only the athlete's own session reaches it; no live Strava record names a file in it; the sweep and a reset take it away

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `/api/user/export/originals`, driven through the real auth router over a
//! memory blob store (which signs no URLs, so a download streams through the
//! API) and a memory record store the test fills.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use std::sync::Arc;
use std::time::Duration as Wait;

use axum::extract::{Path, State};
use axum::{Extension, Json, Router};
use bytes::Bytes;
use chrono::{Duration, Utc};
use dravr_barrage::blob::memory::MemoryBlobStore;
use dravr_barrage::blob::{BlobKey, BlobMeta, BlobStore};
use dravr_barrage::digest::Sha256;
use dravr_barrage::record::{StreamsState, SummaryWindow, TotalsFrom};
use dravr_barrage::store::memory::MemoryActivityRecordStore;
use dravr_barrage::store::ActivityRecordStore;
use dravr_barrage::{ActivityId, ActivityRecord, LiveSource, Provenance, Scope, SourceKey};
use pierre_auth::api_keys::{ApiKeyManager, ApiKeyTier, CreateApiKeyRequest};
use pierre_core::models::{
    ActivityBuilder, CreateUserMcpTokenRequest, SportType, Tenant, TenantId, User, UserStatus,
};
use pierre_core::permissions::impersonation::ImpersonationSession;
use pierre_database::repositories::feedback_messages::{
    FeedbackImage, FeedbackWriter, NewFeedbackMessage,
};
use pierre_database::repositories::{BundleStatus, ExportBundle};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::handlers::types::ResetAccountRequest;
use pierre_routes_admin::handlers::user_removal::handle_reset_user;
use pierre_routes_auth::AuthRoutes;
use pierre_services::barrage_keys::memory_records;
use pierre_services::originals_seal::unseal;
use pierre_test_support::server::create_test_admin_user;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{json, Value};
use tokio::time;
use uuid::Uuid;

use crate::admin_user_removal_test::{admin_context, super_admin_token};
use crate::common::create_test_server_resources_with_records_and_blobs;
use crate::helpers::axum_test::{AxumTestRequest, AxumTestResponse};

/// A FIT file's first bytes: `.FIT` at bytes 8..12.
fn fit(seed: u8) -> Vec<u8> {
    let mut bytes = vec![14, 0x10, 0, 0, 0, 0, 0, 0];
    bytes.extend_from_slice(b".FIT");
    bytes.extend((0..400_u16).map(|i| (i as u8).wrapping_mul(seed)));
    bytes
}

struct Setup {
    resources: Arc<ServerContext>,
    routes: Router,
    records: Arc<MemoryActivityRecordStore>,
    blobs: Arc<MemoryBlobStore>,
}

impl Setup {
    async fn new() -> Self {
        let records = Arc::new(memory_records());
        let blobs = Arc::new(MemoryBlobStore::new());
        let resources =
            create_test_server_resources_with_records_and_blobs(records.clone(), blobs.clone())
                .await
                .unwrap();
        Self::over(resources, records, blobs)
    }

    fn over(
        resources: Arc<ServerContext>,
        records: Arc<MemoryActivityRecordStore>,
        blobs: Arc<MemoryBlobStore>,
    ) -> Self {
        Self {
            routes: AuthRoutes::routes(resources.auth_routes_context()),
            resources,
            records,
            blobs,
        }
    }

    fn repos(&self) -> Arc<RepositoryRegistry> {
        self.resources.common.repos.clone()
    }

    fn jwt(&self, user: &User) -> String {
        self.resources
            .auth
            .auth_manager
            .generate_token(user, &self.resources.auth.jwks_manager)
            .unwrap()
    }

    async fn post(&self, user: &User, body: Option<Value>) -> AxumTestResponse {
        let mut request = AxumTestRequest::post("/api/user/export/originals")
            .header("Authorization", &format!("Bearer {}", self.jwt(user)));
        if let Some(body) = body {
            request = request.json(&body);
        }
        request.send(self.routes.clone()).await
    }

    async fn get(&self, user: &User, path: &str) -> AxumTestResponse {
        // The JSON export runs four at once, process-wide, beside the other
        // suites' exports: a busy refusal is retried, as it asks.
        for _ in 0..200 {
            let response = AxumTestRequest::get(path)
                .header("Authorization", &format!("Bearer {}", self.jwt(user)))
                .send(self.routes.clone())
                .await;
            if !response.body_text().contains("\"export_busy\"") {
                return response;
            }
            time::sleep(Wait::from_millis(50)).await;
        }
        panic!("{path} stayed busy");
    }

    /// Ask for a bundle and wait until it is no longer building: its view.
    async fn built(&self, user: &User, body: Option<Value>) -> Value {
        let response = self.post(user, body).await;
        assert_eq!(response.status(), 202, "{}", response.body_text());
        let id = response.json::<Value>()["bundle_id"]
            .as_str()
            .unwrap()
            .to_owned();
        for _ in 0..200 {
            let view: Value = self
                .get(user, &format!("/api/user/export/originals/{id}"))
                .await
                .json();
            if view["status"] != "building" {
                return view;
            }
            time::sleep(Wait::from_millis(25)).await;
        }
        panic!("bug: the bundle never finished building");
    }

    async fn download(&self, user: &User, view: &Value) -> AxumTestResponse {
        self.get(
            user,
            view["download"]
                .as_str()
                .expect("a ready bundle names its download"),
        )
        .await
    }
}

async fn athlete(repos: &RepositoryRegistry, label: &str) -> (User, TenantId) {
    let mut user = User::new(
        format!("{label}-{}@example.com", Uuid::new_v4()),
        "hash".to_owned(),
        Some(label.to_owned()),
    );
    user.user_status = UserStatus::Active;
    repos.users.create(&user).await.unwrap();
    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: format!("{label} tenant"),
            slug: format!("{label}-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user.id,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    repos
        .users
        .update_tenant_id(user.id, tenant_id)
        .await
        .unwrap();
    (user, tenant_id)
}

fn record(scope: Scope, source: SourceKey, id: &str, original: &[u8]) -> ActivityRecord {
    let start = Utc::now() - Duration::days(2);
    let activity = ActivityBuilder::new(id, "Ride", SportType::Ride, start, 3600, source.as_str())
        .distance_meters(20_000.0)
        .build();
    let sha = Sha256::of(original);
    let provenance = Provenance {
        source,
        import_id: None,
        source_activity_id: id.to_owned(),
        source_athlete_id: None,
        ingested_at: Utc::now(),
        original: Some(BlobKey::original(scope, sha)),
        original_sha256: Some(sha),
        session_index: Some(0),
        streams_state: StreamsState::Decodable,
        source_gear: None,
        importer_version: None,
        summary_window: SummaryWindow::default(),
        totals_from: TotalsFrom::default(),
        legacy_id: None,
        legacy_original: None,
    };
    ActivityRecord::try_new(scope, ActivityId::parse(id).unwrap(), provenance, activity).unwrap()
}

/// Keep `original` as an archive record of `id` in `scope` keeps it.
async fn keep_original(setup: &Setup, scope: Scope, source: SourceKey, id: &str, original: &[u8]) {
    setup
        .records
        .upsert(&scope, &[record(scope, source, id, original)])
        .await
        .unwrap();
    setup
        .blobs
        .put(
            &BlobKey::original(scope, Sha256::of(original)),
            Bytes::copy_from_slice(original),
            &BlobMeta::new("application/fit"),
        )
        .await
        .unwrap();
}

/// The members of a downloaded bundle.
fn members(zip_bytes: &[u8]) -> BTreeMap<String, Vec<u8>> {
    let mut zip = zip::ZipArchive::new(Cursor::new(zip_bytes.to_vec())).unwrap();
    let mut out = BTreeMap::new();
    for i in 0..zip.len() {
        let mut member = zip.by_index(i).unwrap();
        let name = member.name().unwrap().into_owned();
        let mut bytes = Vec::new();
        member.read_to_end(&mut bytes).unwrap();
        out.insert(name, bytes);
    }
    out
}

/// Every `SHA256SUMS` line holds for its file; the manifest.
fn verified(members: &BTreeMap<String, Vec<u8>>) -> Value {
    let sums = String::from_utf8(members["SHA256SUMS"].clone()).unwrap();
    for line in sums.lines() {
        let (digest, path) = line.split_once("  ").unwrap();
        assert_eq!(
            Sha256::of(&members[path]).to_hex(),
            digest,
            "bug: {path} fails its checksum"
        );
    }
    serde_json::from_slice(&members["manifest.json"]).unwrap()
}

fn manifest_paths(manifest: &Value) -> Vec<String> {
    manifest["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn request_build_download_round_trip() {
    let setup = Setup::new().await;
    let (ana, tenant) = athlete(&setup.repos(), "ana").await;
    let scope = Scope::new(tenant.as_uuid(), ana.id);
    let ride = fit(3);
    keep_original(&setup, scope, SourceKey::StravaArchive, "ride-1", &ride).await;

    let view = setup.built(&ana, None).await;
    assert_eq!(view["status"], "ready", "{view}");
    assert_eq!(
        view["sealed"], false,
        "bug: an unsealed request built a sealed bundle"
    );
    assert_eq!(
        view["files"], 1,
        "bug: the one kept original is the one file"
    );
    let response = setup.download(&ana, &view).await;
    assert_eq!(response.status(), 200, "{}", response.body_text());
    assert_eq!(
        response.header("content-type").unwrap(),
        "application/zip",
        "bug: an unsealed bundle downloads as a ZIP"
    );
    assert_eq!(
        response.header("cache-control").unwrap(),
        "no-store",
        "bug: a bundle download may be cached"
    );
    assert!(
        response
            .header("content-disposition")
            .unwrap()
            .starts_with("attachment; filename=\"dravr-originals-"),
        "bug: the download is not named as the athlete's originals"
    );
    let bytes = response.bytes();
    assert_eq!(
        view["sha256"],
        Sha256::of(&bytes).to_hex(),
        "bug: the view names the digest of what downloads"
    );
    let members = members(&bytes);
    let manifest = verified(&members);
    assert_eq!(
        manifest["complete"], true,
        "bug: a whole bundle says it is complete"
    );
    assert_eq!(
        manifest["user_id"],
        ana.id.to_string(),
        "bug: the manifest names its athlete"
    );
    assert_eq!(
        manifest_paths(&manifest),
        vec![format!("original/{}.fit", Sha256::of(&ride))],
        "bug: the manifest names exactly the kept original"
    );
    assert_eq!(
        members[&format!("original/{}.fit", Sha256::of(&ride))],
        ride,
        "bug: the original's bytes are not the ones kept"
    );
    assert_eq!(
        manifest["files"][0]["records"][0]["activity_id"], "ride-1",
        "bug: the original names the record that keeps it"
    );
}

#[tokio::test]
async fn uploads_and_feedback_images_are_in_the_bundle() {
    let setup = Setup::new().await;
    let repos = setup.repos();
    let (ana, tenant) = athlete(&repos, "ana").await;
    let scope = Scope::new(tenant.as_uuid(), ana.id);
    let upload = fit(9);
    let upload_sha = Sha256::of(&upload);
    setup
        .blobs
        .put(
            &BlobKey::upload(scope, upload_sha),
            Bytes::copy_from_slice(&upload),
            &BlobMeta::new("application/fit"),
        )
        .await
        .unwrap();
    repos
        .uploaded_activity_files
        .record_uploaded_file(
            &tenant,
            ana.id,
            &upload_sha.to_hex(),
            upload.len() as u64,
            "upload",
        )
        .await
        .unwrap();
    let png = b"\x89PNG\r\n\x1a\nscreenshot".to_vec();
    let png_sha = Sha256::of(&png);
    setup
        .resources
        .common
        .originals
        .put_feedback_image(
            &tenant,
            ana.id,
            &png_sha.to_hex(),
            "image/png",
            Bytes::copy_from_slice(&png),
        )
        .await
        .unwrap();
    let image = FeedbackImage {
        sha256: png_sha.to_hex(),
        media_type: "image/png".to_owned(),
    };
    repos
        .feedback_messages
        .insert_feedback_message(&NewFeedbackMessage {
            tenant_id: tenant,
            user_id: ana.id,
            writer: FeedbackWriter::Athlete,
            body: "it broke",
            screen: None,
            app_version: None,
            locale: None,
            about: None,
            image: Some(&image),
            at: Utc::now(),
        })
        .await
        .unwrap();

    let view = setup.built(&ana, None).await;
    assert_eq!(view["status"], "ready", "{view}");
    let members = members(&setup.download(&ana, &view).await.bytes());
    let manifest = verified(&members);
    let paths = manifest_paths(&manifest);
    assert!(
        paths.contains(&format!("upload/{upload_sha}.fit")),
        "{paths:?}"
    );
    assert!(
        paths.contains(&format!("feedback/{png_sha}.png")),
        "{paths:?}"
    );
    assert_eq!(
        members[&format!("feedback/{png_sha}.png")],
        png,
        "bug: the screenshot's bytes are not the ones attached"
    );
}

#[tokio::test]
async fn a_live_strava_record_never_reaches_the_manifest() {
    let setup = Setup::new().await;
    let (ana, tenant) = athlete(&setup.repos(), "ana").await;
    let scope = Scope::new(tenant.as_uuid(), ana.id);
    let live = fit(5);
    keep_original(
        &setup,
        scope,
        SourceKey::Live(LiveSource::Strava),
        "live-1",
        &live,
    )
    .await;
    let archived = fit(7);
    keep_original(
        &setup,
        scope,
        SourceKey::GarminArchive,
        "garmin-1",
        &archived,
    )
    .await;

    let view = setup.built(&ana, None).await;
    let members = members(&setup.download(&ana, &view).await.bytes());
    let manifest = verified(&members);
    let body = members["manifest.json"].clone();
    let text = String::from_utf8(body).unwrap();
    assert!(
        !text.contains("live-1") && !text.contains(&Sha256::of(&live).to_hex()),
        "bug: a live Strava record named a file in the bundle"
    );
    assert_eq!(
        manifest_paths(&manifest),
        vec![format!("original/{}.fit", Sha256::of(&archived))],
        "bug: the bundle holds anything but the archived original"
    );
}

#[tokio::test]
async fn a_second_request_while_building_returns_the_same_bundle() {
    let setup = Setup::new().await;
    let repos = setup.repos();
    let (ana, tenant) = athlete(&repos, "ana").await;
    let now = Utc::now();
    let building = repos
        .user_export_bundles
        .insert_building(&ExportBundle {
            id: Uuid::new_v4(),
            tenant_id: tenant,
            user_id: ana.id,
            status: BundleStatus::Building,
            sealed: false,
            files: None,
            missing: None,
            damaged: None,
            byte_size: None,
            sha256: None,
            error: None,
            created_at: now,
            heartbeat_at: now,
            finished_at: None,
            expires_at: now + Duration::days(7),
        })
        .await
        .unwrap();
    let response = setup.post(&ana, None).await;
    assert_eq!(response.status(), 202, "{}", response.body_text());
    assert_eq!(
        response.json::<Value>()["bundle_id"],
        building.id.to_string(),
        "bug: a second build started beside the one building"
    );
    let sealed = setup
        .post(
            &ana,
            Some(json!({ "passphrase": "a long enough passphrase" })),
        )
        .await;
    assert_eq!(sealed.status(), 409, "{}", sealed.body_text());
    assert_eq!(
        sealed.json::<Value>()["error"],
        "bundle_building",
        "bug: the conflict is not named by its code"
    );
    let not_ready = setup
        .get(
            &ana,
            &format!("/api/user/export/originals/{}/download", building.id),
        )
        .await;
    assert_eq!(not_ready.status(), 409, "{}", not_ready.body_text());
}

/// A `building` row for `user` in `tenant`, as a build in flight leaves it.
async fn building_row(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user: &User,
    sealed: bool,
) -> ExportBundle {
    let now = Utc::now();
    repos
        .user_export_bundles
        .insert_building(&ExportBundle {
            id: Uuid::new_v4(),
            tenant_id: tenant,
            user_id: user.id,
            status: BundleStatus::Building,
            sealed,
            files: None,
            missing: None,
            damaged: None,
            byte_size: None,
            sha256: None,
            error: None,
            created_at: now,
            heartbeat_at: now,
            finished_at: None,
            expires_at: now + Duration::days(7),
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn a_request_answered_by_the_build_in_flight_is_not_counted() {
    let setup = Setup::new().await;
    let repos = setup.repos();
    let (ana, tenant) = athlete(&repos, "ana").await;
    let building = building_row(&repos, tenant, &ana, false).await;
    // One more than a day allows: none of them starts a build.
    for attempt in 1..=4 {
        let response = setup.post(&ana, None).await;
        assert_eq!(
            response.status(),
            202,
            "bug: request {attempt}, answered by the build in flight, was counted: {}",
            response.body_text()
        );
        assert_eq!(
            response.json::<Value>()["bundle_id"],
            building.id.to_string(),
            "bug: request {attempt} was not answered with the build in flight"
        );
    }
}

#[tokio::test]
async fn a_sealed_build_in_flight_never_stands_for_another_sealed_request() {
    let setup = Setup::new().await;
    let repos = setup.repos();
    let (ana, tenant) = athlete(&repos, "ana").await;
    building_row(&repos, tenant, &ana, true).await;
    // The server cannot tell whether this passphrase is the one the build in
    // flight was sealed with: handing that build back could hand the athlete
    // a file their passphrase does not open.
    let response = setup
        .post(
            &ana,
            Some(json!({ "passphrase": "another long passphrase" })),
        )
        .await;
    assert_eq!(response.status(), 409, "{}", response.body_text());
    assert_eq!(
        response.json::<Value>()["error"],
        "bundle_building",
        "bug: the conflict is not named by its code"
    );
}

#[tokio::test]
async fn another_athletes_bundle_is_404() {
    let setup = Setup::new().await;
    let repos = setup.repos();
    let (ana, _) = athlete(&repos, "ana").await;
    let (ben, _) = athlete(&repos, "ben").await;
    let view = setup.built(&ana, None).await;
    let id = view["bundle_id"].as_str().unwrap();
    for path in [
        format!("/api/user/export/originals/{id}"),
        format!("/api/user/export/originals/{id}/download"),
    ] {
        let response = setup.get(&ben, &path).await;
        assert_eq!(response.status(), 404, "{path}: {}", response.body_text());
    }
}

#[tokio::test]
async fn an_api_key_mcp_token_or_impersonation_cannot_request_or_download() {
    let setup = Setup::new().await;
    let repos = setup.repos();
    let (ana, _) = athlete(&repos, "ana").await;
    // An impersonation token holds only while its opener is an active admin.
    let admin = create_test_admin_user(
        &format!("admin-{}@example.com", Uuid::new_v4()),
        Some("admin".to_owned()),
    );
    repos.users.create(&admin).await.unwrap();
    let view = setup.built(&ana, None).await;
    let (api_key, full_key) = ApiKeyManager::new()
        .create_api_key(
            ana.id,
            CreateApiKeyRequest {
                name: "bundle probe".to_owned(),
                description: None,
                tier: ApiKeyTier::Starter,
                rate_limit_requests: None,
                expires_in_days: None,
            },
        )
        .unwrap();
    repos.api_keys.create(&api_key).await.unwrap();
    let mcp_token = repos
        .user_mcp_tokens
        .create_token(
            ana.id,
            &CreateUserMcpTokenRequest {
                name: "Claude Desktop".to_owned(),
                expires_in_days: None,
            },
        )
        .await
        .unwrap()
        .token_value;
    let session = ImpersonationSession::new(admin.id, ana.id, Some("support".to_owned()));
    repos.impersonation.create_session(&session).await.unwrap();
    let impersonation = setup
        .resources
        .auth
        .auth_manager
        .generate_impersonation_token(
            &ana,
            admin.id,
            &session.id,
            &setup.resources.auth.jwks_manager,
            None,
        )
        .unwrap();
    let download = view["download"].as_str().unwrap();
    for authorization in [
        full_key,
        format!("Bearer {}", mcp_token.expose_secret()),
        format!("Bearer {impersonation}"),
    ] {
        let asked = AxumTestRequest::post("/api/user/export/originals")
            .header("Authorization", &authorization)
            .send(setup.routes.clone())
            .await;
        assert_eq!(asked.status(), 403, "{}", asked.body_text());
        let fetched = AxumTestRequest::get(download)
            .header("Authorization", &authorization)
            .send(setup.routes.clone())
            .await;
        assert_eq!(fetched.status(), 403, "{}", fetched.body_text());
    }
}

#[tokio::test]
async fn a_sealed_bundle_opens_only_with_its_passphrase() {
    let setup = Setup::new().await;
    let (ana, tenant) = athlete(&setup.repos(), "ana").await;
    let ride = fit(11);
    keep_original(
        &setup,
        Scope::new(tenant.as_uuid(), ana.id),
        SourceKey::CorosArchive,
        "c-1",
        &ride,
    )
    .await;
    let short = setup
        .post(&ana, Some(json!({ "passphrase": "short" })))
        .await;
    assert_eq!(short.status(), 400, "{}", short.body_text());

    let passphrase = "correct horse battery staple";
    let view = setup
        .built(&ana, Some(json!({ "passphrase": passphrase })))
        .await;
    assert_eq!(view["status"], "ready", "{view}");
    assert_eq!(
        view["sealed"], true,
        "bug: a passphrase did not seal the bundle"
    );
    let response = setup.download(&ana, &view).await;
    assert_eq!(
        response.header("content-type").unwrap(),
        "application/octet-stream",
        "bug: a sealed bundle downloads typed as a ZIP"
    );
    assert!(
        response
            .header("content-disposition")
            .unwrap()
            .contains(".zip.age"),
        "bug: a sealed bundle is not named as sealed"
    );
    let sealed = response.bytes();
    assert!(
        zip::ZipArchive::new(Cursor::new(sealed.clone())).is_err(),
        "bug: a sealed bundle reads as a plain archive"
    );
    assert!(
        unseal(&SecretString::from("the wrong passphrase!"), &sealed).is_err(),
        "bug: a wrong passphrase opened the bundle"
    );
    let opened = unseal(&SecretString::from(passphrase.to_owned()), &sealed).unwrap();
    let members = members(&opened);
    verified(&members);
    assert_eq!(
        members[&format!("original/{}.fit", Sha256::of(&ride))],
        ride,
        "bug: the unsealed original's bytes are not the ones kept"
    );
}

#[tokio::test]
async fn the_sweep_fails_a_stale_build_and_deletes_an_expired_bundle() {
    let setup = Setup::new().await;
    let repos = setup.repos();
    let (ana, tenant) = athlete(&repos, "ana").await;
    let (ben, ben_tenant) = athlete(&repos, "ben").await;
    let ready = setup.built(&ana, None).await;
    let ready_id = Uuid::parse_str(ready["bundle_id"].as_str().unwrap()).unwrap();
    let ready_key = BlobKey::export_bundle(Scope::new(tenant.as_uuid(), ana.id), ready_id);
    assert!(
        setup.blobs.size(&ready_key).await.unwrap().is_some(),
        "bug: a ready bundle's blob is not stored"
    );

    let long_ago = Utc::now() - Duration::hours(1);
    let stalled = repos
        .user_export_bundles
        .insert_building(&ExportBundle {
            id: Uuid::new_v4(),
            tenant_id: ben_tenant,
            user_id: ben.id,
            status: BundleStatus::Building,
            sealed: false,
            files: None,
            missing: None,
            damaged: None,
            byte_size: None,
            sha256: None,
            error: None,
            created_at: long_ago,
            heartbeat_at: long_ago,
            finished_at: None,
            expires_at: long_ago + Duration::days(7),
        })
        .await
        .unwrap();
    let stalled_key = BlobKey::export_bundle(Scope::new(ben_tenant.as_uuid(), ben.id), stalled.id);
    setup
        .blobs
        .put(
            &stalled_key,
            Bytes::from_static(b"half a zip"),
            &BlobMeta::expiring("application/zip"),
        )
        .await
        .unwrap();

    let bundles = setup.resources.common.originals_bundles.clone().unwrap();
    let swept = bundles.sweep(Utc::now()).await.unwrap();
    assert_eq!(swept.stalled, 1, "bug: the stopped build is failed");
    assert_eq!(swept.expired, 0, "bug: nothing has expired yet");
    let failed = repos
        .user_export_bundles
        .get(ben_tenant, ben.id, stalled.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        failed.status,
        BundleStatus::Failed,
        "bug: the swept build is not marked failed"
    );
    assert!(
        setup.blobs.size(&stalled_key).await.unwrap().is_none(),
        "bug: a stopped build's blob is deleted"
    );

    let later = Utc::now() + Duration::days(8);
    let swept = bundles.sweep(later).await.unwrap();
    assert_eq!(swept.expired, 2, "bug: both bundles expired");
    assert!(
        repos
            .user_export_bundles
            .get(tenant, ana.id, ready_id)
            .await
            .unwrap()
            .is_none(),
        "bug: an expired bundle's row outlived the sweep"
    );
    assert!(
        setup.blobs.size(&ready_key).await.unwrap().is_none(),
        "bug: an expired bundle's blob is deleted with its row"
    );
}

#[tokio::test]
async fn an_account_reset_takes_the_bundle_row_and_blob() {
    // Over blobs the test reads back, so the blob's absence is the store's.
    let setup = Setup::new().await;
    let repos = setup.repos();
    let (ana, tenant) = athlete(&repos, "ana").await;
    let view = setup.built(&ana, None).await;
    assert_eq!(view["status"], "ready", "{view}");
    let id = Uuid::parse_str(view["bundle_id"].as_str().unwrap()).unwrap();
    let before = setup.download(&ana, &view).await;
    assert_eq!(
        before.status(),
        200,
        "bug: the bundle does not download before the reset: {}",
        before.body_text()
    );
    let key = BlobKey::export_bundle(Scope::new(tenant.as_uuid(), ana.id), id);
    assert!(
        setup.blobs.size(&key).await.unwrap().is_some(),
        "the bundle is stored before the reset"
    );

    let response = handle_reset_user(
        State(admin_context(&setup.resources)),
        Extension(super_admin_token()),
        Path(ana.id.to_string()),
        Json(ResetAccountRequest {
            reason: Some("starting over".to_owned()),
            delete_owned_groups: false,
        }),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 200, "bug: the account reset failed");
    assert!(
        repos
            .user_export_bundles
            .get(tenant, ana.id, id)
            .await
            .unwrap()
            .is_none(),
        "bug: a reset left the bundle's row"
    );
    let after = setup
        .get(&ana, &format!("/api/user/export/originals/{id}/download"))
        .await;
    assert_eq!(
        after.status(),
        404,
        "bug: a reset left the bundle downloadable"
    );
    assert_eq!(
        setup.blobs.size(&key).await.unwrap(),
        None,
        "bug: a reset left the bundle's blob"
    );
}

#[tokio::test]
async fn the_export_references_the_latest_bundle() {
    let setup = Setup::new().await;
    let (ana, _) = athlete(&setup.repos(), "ana").await;
    let before: Value = setup.get(&ana, "/api/user/export").await.json();
    assert_eq!(
        before["originals"],
        Value::Null,
        "bug: no bundle asked for, none named"
    );
    let view = setup.built(&ana, None).await;
    let export: Value = setup.get(&ana, "/api/user/export").await.json();
    assert_eq!(
        export["originals"]["bundle_id"], view["bundle_id"],
        "bug: the export names another bundle than the latest"
    );
    assert_eq!(
        export["originals"]["status"], "ready",
        "bug: the export names the bundle's state wrong"
    );
    assert_eq!(
        export["originals"]["download"], view["download"],
        "bug: the export names another download route"
    );
}

#[tokio::test]
async fn a_reopened_page_finds_the_latest_bundle() {
    let setup = Setup::new().await;
    let (ana, _) = athlete(&setup.repos(), "ana").await;
    let none: Value = setup.get(&ana, "/api/user/export/originals").await.json();
    assert_eq!(none, Value::Null, "bug: no bundle asked for, none answered");
    let view = setup.built(&ana, None).await;
    let latest: Value = setup.get(&ana, "/api/user/export/originals").await.json();
    assert_eq!(
        latest["bundle_id"], view["bundle_id"],
        "bug: the reopened page found another bundle than the latest"
    );
    assert_eq!(
        latest["status"], "ready",
        "bug: the reopened page reads the bundle's state wrong"
    );
}
