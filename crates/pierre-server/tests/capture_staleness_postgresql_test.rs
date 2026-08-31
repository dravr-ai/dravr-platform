// ABOUTME: PostgreSQL-lane test for the capture-staleness snapshot join across a UUID/TEXT type split
// ABOUTME: provider_connections types ids as TEXT, activity_fetch_freshness as UUID — SQLite cannot catch this
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `PostgreSQL` capture-staleness snapshot tests.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![cfg(feature = "postgresql")]

use chrono::{Duration, Utc};
use pierre_core::models::{CoachingPersona, ConnectionType, TenantId, User, UserStatus, UserTier};
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::Database;
use pierre_database::database::test_utils::create_test_db;
use uuid::Uuid;

async fn seed_pg_user(db: &Database) -> Uuid {
    let user_id = Uuid::new_v4();
    let user = User {
        id: user_id,
        email: format!("capture-{user_id}@test.local"),
        display_name: Some("Capture Staleness Test".to_owned()),
        password_hash: "hash_not_verified".to_owned(),
        tier: UserTier::Starter,
        is_active: true,
        user_status: UserStatus::Active,
        is_admin: false,
        role: UserRole::User,
        approved_by: None,
        approved_at: Some(Utc::now()),
        created_at: Utc::now(),
        last_active: Utc::now(),
        strava_token: None,
        fitbit_token: None,
        firebase_uid: None,
        auth_provider: String::new(),
        analytics_consent: false,
        analytics_consent_at: None,
        locale: "fr".to_owned(),
        coaching_persona: CoachingPersona::Casual,
        manages_roster: false,
        timezone: None,
        theme: None,
    };
    db.repositories().users.create(&user).await.unwrap();
    user_id
}

/// The join this test exists for. `provider_connections.user_id` / `.tenant_id`
/// are TEXT; `activity_fetch_freshness.user_id` / `.tenant_id` are UUID. On
/// `SQLite` every one of those columns is TEXT, so the join matches there whatever
/// the SQL says — this is the only lane that can prove the Postgres statement
/// actually pairs a fetch mark with its connection.
#[tokio::test]
async fn test_pg_snapshot_joins_across_the_uuid_text_split() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let user_id = seed_pg_user(&db).await;
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    repos
        .provider_connections
        .register_connection(user_id, tenant, "sciotte", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    repos
        .provider_connections
        .touch_last_used(user_id, tenant, "sciotte")
        .await
        .unwrap();

    let fetched_at = Utc::now() - Duration::hours(50);
    repos
        .activity_cache
        .record_activity_fetch(user_id, &tenant, "sciotte", fetched_at)
        .await
        .unwrap();

    let snapshot = repos
        .activity_cache
        .capture_freshness_snapshot(500)
        .await
        .unwrap();

    let row = snapshot
        .iter()
        .find(|c| c.user_id == user_id.to_string() && c.provider == "sciotte")
        .expect("the connection and its fetch mark join on Postgres");
    assert_eq!(row.tenant_id, tenant.to_string());
    let mark = row
        .last_fetch_at
        .expect("the UUID-keyed fetch mark reaches the TEXT-keyed connection");
    assert!(
        (mark - fetched_at).num_seconds().abs() <= 1,
        "expected the mark at {fetched_at}, got {mark}"
    );
    assert!(
        row.last_used_at.is_some(),
        "TIMESTAMPTZ last_used_at decodes into the snapshot"
    );
}

/// A connection with no fetch mark must survive the LEFT JOIN as never-fetched.
/// An INNER JOIN would silently delete the single most alarming state this
/// endpoint can report, and on Postgres the NULL side decodes through a
/// different path than `SQLite`'s.
#[tokio::test]
async fn test_pg_never_fetched_connection_survives_as_null() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let user_id = seed_pg_user(&db).await;
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    repos
        .provider_connections
        .register_connection(user_id, tenant, "whoop", &ConnectionType::OAuth, None)
        .await
        .unwrap();

    let snapshot = repos
        .activity_cache
        .capture_freshness_snapshot(500)
        .await
        .unwrap();

    let row = snapshot
        .iter()
        .find(|c| c.user_id == user_id.to_string() && c.provider == "whoop")
        .expect("a connection with no fetch mark is still reported");
    assert!(
        row.last_fetch_at.is_none(),
        "no fetch mark must read as None, not be dropped by the join"
    );
    assert!(
        row.last_used_at.is_none(),
        "a NULL TIMESTAMPTZ last_used_at decodes as None"
    );
}

/// The re-auth filter has to hold on Postgres too — it is a plain string
/// comparison, but a status written by one backend and filtered by the other is
/// exactly the shape of this repo's PG regressions.
#[tokio::test]
async fn test_pg_reauth_connection_is_excluded() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let user_id = seed_pg_user(&db).await;
    let tenant = TenantId::from_uuid(Uuid::new_v4());

    repos
        .provider_connections
        .register_connection(user_id, tenant, "strava", &ConnectionType::OAuth, None)
        .await
        .unwrap();
    repos
        .provider_connections
        .mark_needs_reauth(user_id, tenant, "strava", Some("invalid_grant"))
        .await
        .unwrap();

    let snapshot = repos
        .activity_cache
        .capture_freshness_snapshot(500)
        .await
        .unwrap();

    assert!(
        !snapshot
            .iter()
            .any(|c| c.user_id == user_id.to_string() && c.provider == "strava"),
        "a connection needing re-auth must not appear in the snapshot"
    );
}

/// The cast direction is load-bearing, and this is the test that proves it.
///
/// Casting the TEXT side UP (`pc.user_id::uuid`) is the obvious way to write this
/// join and it throws `invalid input syntax for type uuid` on the first row whose
/// TEXT id is not UUID-shaped — taking down the whole report, for every athlete,
/// because of one malformed row. Casting the UUID side DOWN to text cannot fail,
/// because every UUID renders as text.
///
/// `provider_connections` types both ids as TEXT and nothing at the database
/// level constrains them, so this inserts the malformed row directly rather than
/// through the repository, whose `TenantId` argument could never produce one.
#[tokio::test]
async fn test_pg_snapshot_survives_a_non_uuid_tenant_id() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();

    let healthy_user = seed_pg_user(&db).await;
    let healthy_tenant = TenantId::from_uuid(Uuid::new_v4());
    repos
        .provider_connections
        .register_connection(
            healthy_user,
            healthy_tenant,
            "sciotte",
            &ConnectionType::OAuth,
            None,
        )
        .await
        .unwrap();

    let pool = db
        .postgres_pool()
        .expect("the postgresql feature gives a Postgres pool");
    sqlx::query(
        r"
        INSERT INTO provider_connections
            (id, user_id, tenant_id, provider, connection_type, connected_at, status)
        VALUES ($1, $2, $3, $4, $5, $6, 'active')
        ",
    )
    .bind(Uuid::new_v4().to_string())
    .bind("not-a-uuid-either")
    .bind("acme-corp")
    .bind("garmin")
    .bind("oauth")
    .bind(Utc::now())
    .execute(pool)
    .await
    .expect("a TEXT id column accepts a non-UUID value");

    // The whole report must still come back rather than erroring out.
    let snapshot = repos
        .activity_cache
        .capture_freshness_snapshot(500)
        .await
        .expect("a non-UUID tenant must not throw the snapshot query");

    assert!(
        snapshot
            .iter()
            .any(|c| c.user_id == healthy_user.to_string() && c.provider == "sciotte"),
        "the healthy connection is still reported alongside the malformed row"
    );
    let malformed = snapshot
        .iter()
        .find(|c| c.tenant_id == "acme-corp")
        .expect("the malformed row is reported, not silently skipped");
    assert!(
        malformed.last_fetch_at.is_none(),
        "a tenant the UUID column could never have held reads as never-fetched"
    );
}
