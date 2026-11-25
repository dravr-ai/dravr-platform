// ABOUTME: carnet#1550 — the record_log worker re-keys legacy apple- records each tick and projects them as an import would
// ABOUTME: Over the server's own projection sink: the keyed activity is served, the legacy one retracted, and remaining logged

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::slice;
use std::sync::Arc;

use dravr_barrage::blob::memory::MemoryBlobStore;
use dravr_barrage::blob::BlobStore;
use dravr_barrage::record::{StreamsState, SummaryWindow, TotalsFrom};
use dravr_barrage::store::memory::MemoryImportJobStore;
use dravr_barrage::store::{ActivityRecordStore, BarrageStores, PageReq, RecordQuery};
use dravr_barrage::{ActivityId, ActivityRecord, Provenance, Scope, SourceKey};
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::models::{ActivityBuilder, SportType, Tenant, TenantId, User, UserStatus};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::services::archive_import::ops::importer_over;
use pierre_providers::originals::OriginalsStore;
use pierre_services::barrage_keys::memory_records;
use pierre_services::record_log_worker::{
    GrantReceiptOutbox, KeyedNameStores, LegacyApplePage, RecordLogWorker,
};
use uuid::Uuid;

use crate::common::create_test_server_resources;

const APPLE: SourceKey = SourceKey::AppleHealthArchive;

async fn seed_athlete(repos: &RepositoryRegistry) -> (Uuid, TenantId) {
    let mut user = User::new(
        format!("legacy-apple-{}@example.com", Uuid::new_v4()),
        "hash".to_owned(),
        Some("Legacy Apple".to_owned()),
    );
    user.user_status = UserStatus::Active;
    let user_id = user.id;
    repos.users.create(&user).await.unwrap();
    let tenant_id = TenantId::generate();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: "Legacy Apple tenant".to_owned(),
            slug: format!("legacy-apple-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
        })
        .await
        .unwrap();
    (user_id, tenant_id)
}

/// An Apple workout as a v0.27 import stored it: under its legacy
/// `apple-<hex16>` id, no route kept.
fn legacy_workout(scope: Scope, id: &ActivityId) -> ActivityRecord {
    let start = Timestamp::now() - Elapsed::days(2);
    let activity = ActivityBuilder::new(
        id.as_str(),
        "Morning run",
        SportType::Run,
        start,
        1800,
        APPLE.as_str(),
    )
    .distance_meters(5_000.0)
    .build();
    let provenance = Provenance {
        source: APPLE,
        import_id: None,
        source_activity_id: id.as_str().to_owned(),
        source_athlete_id: None,
        ingested_at: start,
        original: None,
        original_sha256: None,
        session_index: None,
        streams_state: StreamsState::None,
        source_gear: None,
        importer_version: None,
        summary_window: SummaryWindow::default(),
        totals_from: TotalsFrom::default(),
        legacy_id: None,
        legacy_original: None,
    };
    ActivityRecord::try_new(scope, id.clone(), provenance, activity).unwrap()
}

/// A worker whose renames carry an importer over `records`, projecting
/// into `resources` as the server does.
fn rekeying_worker(
    resources: &Arc<ServerContext>,
    records: &Arc<dyn ActivityRecordStore>,
) -> RecordLogWorker {
    let repos = &resources.common.repos;
    let blobs: Arc<dyn BlobStore> = Arc::new(MemoryBlobStore::new());
    let originals = BarrageStores {
        blobs: Arc::clone(&blobs),
        records: Arc::clone(records),
        jobs: Arc::new(MemoryImportJobStore::new()),
    };
    RecordLogWorker::new(
        Arc::clone(records),
        Arc::clone(&repos.import_tombstones),
        Arc::clone(&repos.account_deletion),
        Arc::clone(&repos.users),
        GrantReceiptOutbox {
            events: Arc::clone(&repos.oauth_grant_events),
            issuer_url: "https://api.dravr.test".to_owned(),
        },
        None,
        Some(KeyedNameStores {
            apple_rekey: Some(importer_over(resources, originals.clone())),
            originals,
            uploads: Arc::new(OriginalsStore::new(
                Ok(blobs),
                Arc::clone(records),
                Arc::clone(&repos.uploaded_activity_files),
            )),
        }),
    )
}

/// One tick moves a workout stored under its legacy Apple id to its keyed
/// id: the record keeps the legacy id as its alias, the server serves the
/// keyed activity and no longer the legacy one, and the tick reports none
/// remaining. The next tick finds nothing to move.
#[tokio::test]
async fn a_tick_re_keys_a_legacy_apple_workout_and_serves_it_under_the_keyed_id() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let (user_id, tenant_id) = seed_athlete(repos).await;
    let scope = Scope::new(Uuid::from(tenant_id), user_id);
    let records: Arc<dyn ActivityRecordStore> = Arc::new(memory_records());
    let legacy = ActivityId::parse("apple-0123456789abcdef").unwrap();
    let workout = legacy_workout(scope, &legacy);
    records
        .upsert(&scope, slice::from_ref(&workout))
        .await
        .unwrap();
    // Served under its legacy id, as the import that stored it projected it.
    repos
        .activity_cache
        .upsert_activities(
            user_id,
            &tenant_id,
            APPLE.as_str(),
            slice::from_ref(&workout.activity),
        )
        .await
        .unwrap();
    let worker = rekeying_worker(&resources, &records);

    let tick = worker.run().await.unwrap();

    assert_eq!(
        tick.legacy_apple,
        Some(LegacyApplePage {
            rekeyed: 1,
            refused: 0,
            remaining: 0,
            sweep_done: true,
        }),
        "bug: the tick does not re-key the one legacy workout"
    );
    assert!(
        records.get(&scope, APPLE, &legacy).await.unwrap().is_none(),
        "bug: the legacy record outlives the tick"
    );
    let held = records
        .list(&scope, &RecordQuery::all(PageReq::first(10)))
        .await
        .unwrap()
        .items;
    assert_eq!(held.len(), 1, "premise: one workout, one record");
    let keyed = &held[0];
    assert!(
        keyed.activity_id.as_str().starts_with("apple-k"),
        "bug: the record is not under a keyed id: {}",
        keyed.activity_id
    );
    assert_eq!(
        keyed.provenance.legacy_id.as_ref(),
        Some(&legacy),
        "bug: the keyed record does not keep the legacy id as its alias"
    );
    let served = |id: String| async move {
        repos
            .activity_cache
            .get_cached_activity(user_id, &tenant_id, APPLE.as_str(), &id)
            .await
            .unwrap()
    };
    assert!(
        served(keyed.activity_id.to_string()).await.is_some(),
        "bug: the keyed workout is not served"
    );
    assert!(
        served(legacy.to_string()).await.is_none(),
        "bug: the legacy workout is still served"
    );

    let next = worker.run().await.unwrap();
    assert_eq!(
        next.legacy_apple,
        Some(LegacyApplePage {
            sweep_done: true,
            ..LegacyApplePage::default()
        }),
        "bug: a tick after the move does more than nothing"
    );
}

/// Without an importer, no tick re-keys.
#[tokio::test]
async fn a_worker_without_an_importer_re_keys_nothing() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let records: Arc<dyn ActivityRecordStore> = Arc::new(memory_records());
    let blobs: Arc<dyn BlobStore> = Arc::new(MemoryBlobStore::new());
    let renames = KeyedNameStores {
        originals: BarrageStores {
            blobs: Arc::clone(&blobs),
            records: Arc::clone(&records),
            jobs: Arc::new(MemoryImportJobStore::new()),
        },
        uploads: Arc::new(OriginalsStore::new(
            Ok(blobs),
            Arc::clone(&records),
            Arc::clone(&repos.uploaded_activity_files),
        )),
        apple_rekey: None,
    };
    let worker = RecordLogWorker::new(
        Arc::clone(&records),
        Arc::clone(&repos.import_tombstones),
        Arc::clone(&repos.account_deletion),
        Arc::clone(&repos.users),
        GrantReceiptOutbox {
            events: Arc::clone(&repos.oauth_grant_events),
            issuer_url: "https://api.dravr.test".to_owned(),
        },
        None,
        Some(renames),
    );
    assert_eq!(
        worker.run().await.unwrap().legacy_apple,
        None,
        "bug: a worker without an importer re-keys"
    );
}

/// A workout the athlete deleted under its legacy id stays where it is: the
/// tick counts it refused, and what remains is what was refused, tick after
/// tick.
#[tokio::test]
async fn a_tick_counts_an_erased_legacy_workout_refused() {
    let resources = create_test_server_resources().await.unwrap();
    let repos = &resources.common.repos;
    let (user_id, tenant_id) = seed_athlete(repos).await;
    let scope = Scope::new(Uuid::from(tenant_id), user_id);
    let records: Arc<dyn ActivityRecordStore> = Arc::new(memory_records());
    let legacy = ActivityId::parse("apple-fedcba9876543210").unwrap();
    let workout = legacy_workout(scope, &legacy);
    records
        .upsert(&scope, slice::from_ref(&workout))
        .await
        .unwrap();
    records
        .remember_erased(&scope, &workout, &[])
        .await
        .unwrap();
    let worker = rekeying_worker(&resources, &records);

    for tick in ["first", "second"] {
        assert_eq!(
            worker.run().await.unwrap().legacy_apple,
            Some(LegacyApplePage {
                rekeyed: 0,
                refused: 1,
                remaining: 1,
                sweep_done: true,
            }),
            "bug: {tick} tick: an erased legacy workout is not refused, or remaining is not what was refused"
        );
    }
    assert!(
        records.get(&scope, APPLE, &legacy).await.unwrap().is_some(),
        "bug: the tick deleted the refused legacy record"
    );
}
