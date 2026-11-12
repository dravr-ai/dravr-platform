// ABOUTME: carnet#1482 — a provider purge deletes the activity store's live records, never an imported archive
// ABOUTME: One attestation per purge; a re-read after it is stored as new, not refused as erased

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Every provider read is kept as a record in the activity store
//! (dravr-barrage), projected into `cached_activities`. A disconnect that
//! deleted only the cache rows would leave the athlete's data held after it,
//! for a later projection to serve again. These tests drive the disconnect
//! chokepoint's purge and the operator's termination purge over a server
//! whose record store they can read, and check that the live source's
//! records go, an imported Strava archive's records and cache rows stay, the
//! purge attests once with the caller's reason, and a reconnect's read is
//! stored as new.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::slice;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{Duration, Utc};
use dravr_barrage::record::{StreamsState, SummaryWindow, TotalsFrom};
use dravr_barrage::store::memory::MemoryActivityRecordStore;
use dravr_barrage::store::ActivityRecordStore;
use dravr_barrage::{ActivityId, ActivityRecord, LiveSource, Provenance, Scope, SourceKey};
use pierre_auth::auth::AuthManager;
use pierre_cache::{Cache, CacheConfig};
use pierre_core::models::{Activity, ActivityBuilder, SportType, TenantId};
use pierre_database::repositories::PurgeReason;
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_services::provider_revocation::purge_provider_data;
use uuid::Uuid;

use crate::common;

const STRAVA: &str = "strava";
const STRAVA_ARCHIVE: &str = "strava_archive";

/// A server whose live records are kept in `records`.
async fn server_over(records: Arc<MemoryActivityRecordStore>) -> Arc<ServerContext> {
    common::init_server_config();
    let database = common::create_test_database().await.unwrap();
    let cache = Cache::new(CacheConfig {
        max_entries: 1000,
        redis_url: None,
        cleanup_interval: StdDuration::from_mins(1),
        enable_background_cleanup: false,
        ..Default::default()
    })
    .await
    .unwrap();
    let live_records: Arc<dyn ActivityRecordStore> = records;
    Arc::new(
        ServerContext::new(
            (*database).clone(),
            AuthManager::new(24),
            "test_admin_secret",
            Arc::new(common::test_server_config()),
            cache,
            ServerContextOptions {
                jwks_manager: Some(common::get_shared_test_jwks()),
                live_records: Some(live_records),
                ..ServerContextOptions::testing()
            },
        )
        .await
        .unwrap(),
    )
}

/// An athlete with a tenant of their own.
async fn athlete(resources: &ServerContext, label: &str) -> (Uuid, TenantId) {
    let (user_id, _user, tenant_id) = common::create_test_user_with_plan(
        &resources.agent.database,
        &format!("{label}-{}@example.com", Uuid::new_v4()),
        "starter",
    )
    .await
    .unwrap();
    (user_id, tenant_id)
}

/// A ride `hours_ago`, as `provider` serves it.
fn ride(id: &str, hours_ago: i64, provider: &str) -> Activity {
    ActivityBuilder::new(
        id,
        "Morning ride",
        SportType::Ride,
        Utc::now() - Duration::hours(hours_ago),
        3_600,
        provider,
    )
    .build()
}

/// Keep `activity` as the athlete's imported Strava archive keeps it: the
/// record in the store, its projection in the cache under the archive key.
async fn import_archive_ride(
    resources: &ServerContext,
    records: &MemoryActivityRecordStore,
    (user_id, tenant_id): (Uuid, TenantId),
    activity: &Activity,
) {
    let scope = Scope::new(tenant_id.as_uuid(), user_id);
    let provenance = Provenance {
        source: SourceKey::StravaArchive,
        import_id: None,
        source_activity_id: activity.id().to_owned(),
        source_athlete_id: None,
        ingested_at: Utc::now(),
        original: None,
        original_sha256: None,
        session_index: None,
        streams_state: StreamsState::None,
        source_gear: None,
        importer_version: None,
        summary_window: SummaryWindow::default(),
        totals_from: TotalsFrom::default(),
    };
    let record = ActivityRecord::try_new(
        scope,
        ActivityId::parse(activity.id()).unwrap(),
        provenance,
        activity.clone(),
    )
    .unwrap();
    records.upsert(&scope, &[record]).await.unwrap();
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(
            user_id,
            &tenant_id,
            STRAVA_ARCHIVE,
            slice::from_ref(activity),
        )
        .await
        .unwrap();
}

/// Whether the store holds `id` of `source` for the athlete.
async fn holds(
    records: &MemoryActivityRecordStore,
    (user_id, tenant_id): (Uuid, TenantId),
    source: SourceKey,
    id: &str,
) -> bool {
    records
        .get(
            &Scope::new(tenant_id.as_uuid(), user_id),
            source,
            &ActivityId::parse(id).unwrap(),
        )
        .await
        .unwrap()
        .is_some()
}

/// Whether the cache serves `id` of `provider` to the athlete.
async fn served(
    resources: &ServerContext,
    (user_id, tenant_id): (Uuid, TenantId),
    provider: &str,
    id: &str,
) -> bool {
    resources
        .common
        .repos
        .activity_cache
        .get_cached_activity(user_id, &tenant_id, provider, id)
        .await
        .unwrap()
        .is_some()
}

#[tokio::test]
async fn a_disconnect_deletes_the_live_records_and_leaves_the_imported_archive() {
    let records = Arc::new(MemoryActivityRecordStore::new());
    let resources = server_over(Arc::clone(&records)).await;
    let who = athlete(&resources, "disconnect").await;
    let (user_id, tenant_id) = who;
    let live = &resources.common.live_activities;
    let read = live
        .ingest(
            user_id,
            tenant_id,
            STRAVA,
            &[ride("9001", 5, STRAVA)],
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(read.stored, 1, "bug: the live read was not kept: {read:?}");
    import_archive_ride(
        &resources,
        &records,
        who,
        &ride("7001", 400, STRAVA_ARCHIVE),
    )
    .await;

    purge_provider_data(
        &resources.data(),
        user_id,
        tenant_id,
        STRAVA,
        PurgeReason::AthleteDisconnect,
    )
    .await
    .unwrap();

    assert!(
        !holds(&records, who, SourceKey::Live(LiveSource::Strava), "9001").await,
        "bug: the live record outlived the disconnect"
    );
    assert!(
        !served(&resources, who, STRAVA, "9001").await,
        "bug: the cache still serves the disconnected ride"
    );
    assert!(
        holds(&records, who, SourceKey::StravaArchive, "7001").await,
        "bug: a live disconnect deleted the imported archive's record"
    );
    assert!(
        served(&resources, who, STRAVA_ARCHIVE, "7001").await,
        "bug: a live disconnect deleted the imported archive's cache row"
    );

    let attested = resources
        .common
        .repos
        .provider_data
        .list_provider_data_purges(STRAVA, 10)
        .await
        .unwrap();
    assert_eq!(
        attested.len(),
        1,
        "bug: the purge is attested once, got {attested:?}"
    );
    assert_eq!(
        attested[0].reason,
        PurgeReason::AthleteDisconnect.as_str(),
        "bug: the purge is not attested as a disconnect"
    );
    assert!(
        resources
            .common
            .repos
            .provider_data
            .list_provider_data_purges(STRAVA_ARCHIVE, 10)
            .await
            .unwrap()
            .is_empty(),
        "bug: the untouched archive was attested as purged"
    );

    // A reconnect reads the ride again: it is stored as new, not refused as
    // a workout the athlete deleted.
    let again = live
        .ingest(
            user_id,
            tenant_id,
            STRAVA,
            &[ride("9001", 5, STRAVA)],
            Utc::now(),
        )
        .await
        .unwrap();
    assert_eq!(again.stored, 1, "bug: the re-read was refused: {again:?}");
    assert!(
        again.newer_session,
        "bug: the store still held a ride the purge deleted"
    );
    assert!(
        served(&resources, who, STRAVA, "9001").await,
        "bug: the re-read ride is not served"
    );
}

#[tokio::test]
async fn the_termination_purge_deletes_every_athletes_live_records() {
    let records = Arc::new(MemoryActivityRecordStore::new());
    let resources = server_over(Arc::clone(&records)).await;
    let first = athlete(&resources, "first").await;
    let second = athlete(&resources, "second").await;
    let live = &resources.common.live_activities;
    for (user_id, tenant_id) in [first, second] {
        live.ingest(
            user_id,
            tenant_id,
            STRAVA,
            &[ride("9001", 5, STRAVA), ride("9002", 30, STRAVA)],
            Utc::now(),
        )
        .await
        .unwrap();
    }
    import_archive_ride(
        &resources,
        &records,
        first,
        &ride("7001", 400, STRAVA_ARCHIVE),
    )
    .await;

    let purge = resources
        .common
        .provider_purge
        .purge_provider_data(STRAVA)
        .await
        .unwrap();
    assert_eq!(
        purge.removed_from("cached_activities"),
        4,
        "bug: the purge did not remove both athletes' cached rides"
    );

    for who in [first, second] {
        for id in ["9001", "9002"] {
            assert!(
                !holds(&records, who, SourceKey::Live(LiveSource::Strava), id).await,
                "bug: a live record outlived the termination purge"
            );
        }
    }
    assert!(
        holds(&records, first, SourceKey::StravaArchive, "7001").await,
        "bug: the termination purge deleted an imported archive's record"
    );
    assert!(
        served(&resources, first, STRAVA_ARCHIVE, "7001").await,
        "bug: the imported archive ride is no longer served"
    );
}

#[tokio::test]
async fn a_provider_whose_reads_the_store_does_not_keep_is_purged_from_the_tables() {
    let records = Arc::new(MemoryActivityRecordStore::new());
    let resources = server_over(Arc::clone(&records)).await;
    let who = athlete(&resources, "unkept").await;
    let (user_id, tenant_id) = who;
    // A provider key the store has no live source for: its cached rows are
    // the only copy, and the purge must not fail asking the store for one.
    let provider = "provider_the_store_does_not_know";
    resources
        .common
        .repos
        .activity_cache
        .upsert_activities(user_id, &tenant_id, provider, &[ride("5001", 5, provider)])
        .await
        .unwrap();

    let purge = resources
        .common
        .provider_purge
        .purge_user_provider_data(
            user_id,
            &tenant_id,
            provider,
            PurgeReason::AthleteDisconnect,
        )
        .await
        .unwrap();
    assert_eq!(
        purge.removed_from("cached_activities"),
        1,
        "bug: the unkept provider's cached row was not purged"
    );
    assert!(
        !served(&resources, who, provider, "5001").await,
        "bug: the purged ride is still served"
    );
}
