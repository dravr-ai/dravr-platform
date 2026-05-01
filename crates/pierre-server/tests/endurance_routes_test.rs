// ABOUTME: Endurance Phase 3 — RouteSummaryRepository multi-tenant isolation + cache hash semantics
// ABOUTME: Validates upsert/get round trip + tenant scoping + hash-mismatch returns None
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

#[cfg(feature = "postgresql")]
use pierre_core::config::database::PostgresPoolConfig;
use pierre_core::models::TenantId;
use pierre_database::backends::factory::Database;
use pierre_database::DatabaseProvider;
use uuid::Uuid;

async fn make_test_db() -> Database {
    let encryption_key = b"test_encryption_key_32_bytes_long".to_vec();
    #[cfg(feature = "postgresql")]
    let db = Database::new(
        "sqlite::memory:",
        encryption_key,
        &PostgresPoolConfig::default(),
    )
    .await
    .expect("create db");
    #[cfg(not(feature = "postgresql"))]
    let db = Database::new("sqlite::memory:", encryption_key)
        .await
        .expect("create db");
    db.migrate().await.expect("migrate");
    db
}

const TERRAIN_JSON: &str = r#"{"total_distance_meters":5000.0,"flat_meters":1000.0,"rolling_meters":2000.0,"climb_meters":1500.0,"steep_meters":500.0,"elevation_gain_meters":250.0,"elevation_loss_meters":200.0}"#;
const CLIMBS_JSON: &str = r#"[{"start_index":0,"end_index":50,"length_meters":1500.0,"elevation_gain_meters":150.0,"avg_gradient":0.10,"category":"cat1"}]"#;

#[tokio::test]
async fn upsert_and_get_route_summary_round_trips() {
    let db = make_test_db().await;
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4();
    let activity_id = "act-42";
    let hash = "deadbeef";

    db.repositories()
        .route_summaries
        .upsert_route_summary(
            tenant_id,
            user_id,
            activity_id,
            hash,
            TERRAIN_JSON,
            CLIMBS_JSON,
        )
        .await
        .expect("upsert");

    let read = db
        .repositories()
        .route_summaries
        .get_route_summary(tenant_id, user_id, activity_id, hash)
        .await
        .expect("get")
        .expect("row present");
    assert_eq!(read.0, TERRAIN_JSON);
    assert_eq!(read.1, CLIMBS_JSON);
}

#[tokio::test]
async fn get_returns_none_when_hash_mismatch() {
    let db = make_test_db().await;
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4();
    let activity_id = "act-7";
    db.repositories()
        .route_summaries
        .upsert_route_summary(
            tenant_id,
            user_id,
            activity_id,
            "abcdef",
            TERRAIN_JSON,
            CLIMBS_JSON,
        )
        .await
        .expect("upsert");
    let read = db
        .repositories()
        .route_summaries
        .get_route_summary(tenant_id, user_id, activity_id, "different_hash")
        .await
        .expect("get");
    assert!(read.is_none(), "stale hash must miss the cache");
}

#[tokio::test]
async fn route_summary_is_tenant_scoped() {
    let db = make_test_db().await;
    let tenant_a = TenantId::new();
    let tenant_b = TenantId::new();
    let user_id = Uuid::new_v4();
    let activity_id = "act-xx";
    let repos = db.repositories();
    repos
        .route_summaries
        .upsert_route_summary(
            tenant_a,
            user_id,
            activity_id,
            "h-a",
            TERRAIN_JSON,
            CLIMBS_JSON,
        )
        .await
        .expect("upsert A");
    repos
        .route_summaries
        .upsert_route_summary(
            tenant_b,
            user_id,
            activity_id,
            "h-b",
            TERRAIN_JSON,
            CLIMBS_JSON,
        )
        .await
        .expect("upsert B");
    let read_a = repos
        .route_summaries
        .get_route_summary(tenant_a, user_id, activity_id, "h-a")
        .await
        .expect("get A")
        .expect("row A");
    let read_b = repos
        .route_summaries
        .get_route_summary(tenant_b, user_id, activity_id, "h-b")
        .await
        .expect("get B")
        .expect("row B");
    assert_eq!(read_a.0, TERRAIN_JSON);
    assert_eq!(read_b.0, TERRAIN_JSON);
    // Tenant A's hash must NOT match tenant B's row.
    let cross = repos
        .route_summaries
        .get_route_summary(tenant_a, user_id, activity_id, "h-b")
        .await
        .expect("cross-hash get");
    assert!(
        cross.is_none(),
        "tenant A row has hash h-a; querying with h-b must miss"
    );
}

#[tokio::test]
async fn upsert_overwrites_existing_row() {
    let db = make_test_db().await;
    let tenant_id = TenantId::new();
    let user_id = Uuid::new_v4();
    let activity_id = "act-9";
    let repos = db.repositories();
    repos
        .route_summaries
        .upsert_route_summary(
            tenant_id,
            user_id,
            activity_id,
            "h1",
            TERRAIN_JSON,
            CLIMBS_JSON,
        )
        .await
        .expect("upsert v1");
    let new_terrain = r#"{"total_distance_meters":7000.0,"flat_meters":7000.0,"rolling_meters":0.0,"climb_meters":0.0,"steep_meters":0.0,"elevation_gain_meters":0.0,"elevation_loss_meters":0.0}"#;
    repos
        .route_summaries
        .upsert_route_summary(tenant_id, user_id, activity_id, "h2", new_terrain, "[]")
        .await
        .expect("upsert v2");
    let read = repos
        .route_summaries
        .get_route_summary(tenant_id, user_id, activity_id, "h2")
        .await
        .expect("get v2")
        .expect("row v2");
    assert_eq!(read.0, new_terrain);
    assert_eq!(read.1, "[]");
}
