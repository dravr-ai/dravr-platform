// ABOUTME: carnet#682 — the migration that deletes every stored intervals.icu route track, drawn from a misread latlng stream
// ABOUTME: Runs the shipped SQL of the test backend against seeded tracks: intervals.icu rows go, Strava and sciotte rows stay

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use pierre_core::models::TenantId;
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::repositories::StoredRouteTrack;
use uuid::Uuid;

use crate::common::{create_test_database, create_test_user_with_plan};

/// The shipped migration, per backend, so the test runs the exact statements
/// a deploy runs rather than a paraphrase of them.
const SQLITE_MIGRATION_SQL: &str = include_str!(
    "../../../migrations/20260930210000_activity_route_tracks_intervals_icu_reread.sql"
);
#[cfg(feature = "postgresql")]
const POSTGRES_MIGRATION_SQL: &str = include_str!(
    "../../../migrations_pg/20260930210000_activity_route_tracks_intervals_icu_reread.sql"
);

/// Every provider a route track is filed under, with the activity it is for.
const SEEDED: [(&str, &str); 4] = [
    ("intervals_icu", "i1001"),
    ("strava", "s2002"),
    ("sciotte_garmin", "g3003"),
    ("sciotte", "t4004"),
];

async fn run_migration(db: &Database) {
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::raw_sql(SQLITE_MIGRATION_SQL)
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => {
            sqlx::raw_sql(POSTGRES_MIGRATION_SQL)
                .execute(postgres.pool())
                .await
                .unwrap();
        }
    }
}

async fn seed(db: &Database, tenant: &TenantId, user_id: Uuid) {
    let repos = db.repositories();
    for (provider, activity_id) in SEEDED {
        repos
            .activity_route_tracks
            .upsert_route_track(
                tenant,
                user_id,
                provider,
                activity_id,
                &StoredRouteTrack::Drawn {
                    source: "streams".to_owned(),
                    track_json: format!(r#"{{"provider":"{provider}"}}"#),
                },
            )
            .await
            .unwrap();
    }
    // A settled no-GPS answer for intervals.icu is read again too.
    repos
        .activity_route_tracks
        .upsert_route_track(
            tenant,
            user_id,
            "intervals_icu",
            "i1002",
            &StoredRouteTrack::Unavailable {
                source: "streams".to_owned(),
                reason: "no_gps".to_owned(),
                expires_at: None,
            },
        )
        .await
        .unwrap();
}

async fn stored(
    db: &Database,
    tenant: &TenantId,
    user_id: Uuid,
    provider: &str,
    activity_id: &str,
) -> Option<StoredRouteTrack> {
    db.repositories()
        .activity_route_tracks
        .get_route_track(tenant, user_id, provider, activity_id)
        .await
        .unwrap()
}

#[tokio::test]
async fn the_migration_deletes_intervals_icu_tracks_and_keeps_every_other_provider() {
    let db = create_test_database().await.unwrap();
    let (user_id, _user, tenant) =
        create_test_user_with_plan(&db, "icu-route-purge@example.com", "starter")
            .await
            .unwrap();
    seed(&db, &tenant, user_id).await;
    assert!(
        stored(&db, &tenant, user_id, "intervals_icu", "i1001")
            .await
            .is_some(),
        "the intervals.icu track is seeded before the migration runs"
    );

    run_migration(&db).await;

    assert_eq!(
        stored(&db, &tenant, user_id, "intervals_icu", "i1001").await,
        None,
        "the intervals.icu track drawn from the misread stream is deleted"
    );
    assert_eq!(
        stored(&db, &tenant, user_id, "intervals_icu", "i1002").await,
        None,
        "the intervals.icu no_gps answer is deleted and read again"
    );
    for (provider, activity_id) in SEEDED.iter().skip(1) {
        assert_eq!(
            stored(&db, &tenant, user_id, provider, activity_id).await,
            Some(StoredRouteTrack::Drawn {
                source: "streams".to_owned(),
                track_json: format!(r#"{{"provider":"{provider}"}}"#),
            }),
            "the {provider} track is untouched"
        );
    }

    // Idempotent: a second run deletes nothing more and does not fail.
    run_migration(&db).await;
    for (provider, activity_id) in SEEDED.iter().skip(1) {
        assert!(
            stored(&db, &tenant, user_id, provider, activity_id)
                .await
                .is_some(),
            "the {provider} track survives a second run"
        );
    }
}
