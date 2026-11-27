// ABOUTME: Covers the one stored route track read: a drawn track decodes, an unavailable answer keeps its reason and expiry
// ABOUTME: A row whose track no longer decodes reads as no read, so its caller reads the route again

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_arpenteur::RouteTrack;
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::models::{TenantId, User};
use pierre_database::backends::factory::Database;
use pierre_database::repositories::StoredRouteTrack;
use pierre_services::activity_track::{stored_track, StoredTrack};
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

struct World {
    db: Database,
    user: Uuid,
    tenant: TenantId,
}

impl World {
    async fn new() -> Self {
        let db = create_test_db().await.unwrap();
        let user = User::new(
            format!("stored-track-{}@example.com", Uuid::new_v4()),
            "argon2-hash-placeholder".to_owned(),
            None,
        );
        let user = db.repositories().users.create(&user).await.unwrap();
        Self {
            db,
            user,
            tenant: TenantId::generate(),
        }
    }

    async fn store(&self, activity_id: &str, stored: &StoredRouteTrack) {
        self.db
            .repositories()
            .activity_route_tracks
            .upsert_route_track(&self.tenant, self.user, "strava", activity_id, stored)
            .await
            .unwrap();
    }

    async fn read(&self, activity_id: &str) -> Option<StoredTrack> {
        stored_track(
            self.db.repositories(),
            &self.tenant,
            self.user,
            "strava",
            activity_id,
        )
        .await
        .unwrap()
    }
}

fn track() -> RouteTrack {
    let points: Vec<(f64, f64)> = (0..40)
        .map(|i| (f64::from(i).mul_add(0.0009, 45.5), -73.6))
        .collect();
    RouteTrack::from_overview(&points).unwrap()
}

#[tokio::test]
async fn a_drawn_row_decodes_into_its_track() {
    let world = World::new().await;
    let drawn = track();
    world
        .store(
            "a1",
            &StoredRouteTrack::Drawn {
                source: "summary_polyline".to_owned(),
                track_json: serde_json::to_string(&drawn).unwrap(),
            },
        )
        .await;
    assert_eq!(
        world.read("a1").await,
        Some(StoredTrack::Drawn { track: drawn }),
        "bug: a stored drawn track must read back as its track"
    );
}

#[tokio::test]
async fn an_unavailable_row_keeps_its_reason_and_expiry() {
    let world = World::new().await;
    world
        .store(
            "settled",
            &StoredRouteTrack::Unavailable {
                source: "streams".to_owned(),
                reason: "no_gps".to_owned(),
                expires_at: None,
            },
        )
        .await;
    assert_eq!(
        world.read("settled").await,
        Some(StoredTrack::Unavailable {
            reason: "no_gps".to_owned(),
            expires_at: None,
        }),
        "bug: a settled absence must keep its reason and stand"
    );

    let expires_at = Timestamp::now() + Elapsed::minutes(10);
    world
        .store(
            "unread",
            &StoredRouteTrack::Unavailable {
                source: "streams".to_owned(),
                reason: "unavailable".to_owned(),
                expires_at: Some(expires_at),
            },
        )
        .await;
    match world.read("unread").await {
        Some(StoredTrack::Unavailable {
            reason,
            expires_at: Some(read),
        }) => {
            assert_eq!(
                reason, "unavailable",
                "bug: the unread reason must survive the decode"
            );
            assert!(
                (read - expires_at).num_seconds().abs() <= 1,
                "bug: Home's retry rule dates the read by its expiry, which must survive the decode"
            );
        }
        other => panic!("bug: an unread answer must keep its expiry, read {other:?}"),
    }
}

#[tokio::test]
async fn an_expired_or_undecodable_row_reads_as_no_read() {
    let world = World::new().await;
    world
        .store(
            "expired",
            &StoredRouteTrack::Unavailable {
                source: "streams".to_owned(),
                reason: "unavailable".to_owned(),
                expires_at: Some(Timestamp::now() - Elapsed::minutes(1)),
            },
        )
        .await;
    assert_eq!(
        world.read("expired").await,
        None,
        "bug: an expired answer must read as no read"
    );

    world
        .store(
            "evolved",
            &StoredRouteTrack::Drawn {
                source: "streams".to_owned(),
                track_json: r#"{"points":[]}"#.to_owned(),
            },
        )
        .await;
    assert_eq!(
        world.read("evolved").await,
        None,
        "bug: a track that no longer decodes must read as no read, so the caller reads again"
    );

    assert_eq!(
        world.read("never").await,
        None,
        "bug: an activity never read has no stored track"
    );
}
