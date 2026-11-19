// ABOUTME: Covers the declared home store: a set, read and forget round trip, scoped to one user in one tenant
// ABOUTME: And the table's CHECK: an `at` row always carries its whole seed, any other state none of it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1593. `create_test_db` opens whichever `DATABASE_URL` names, so
//! these run on `SQLite` and `PostgreSQL` alike.

use chrono::{Duration, Utc};
use pierre_core::models::{HomeDeclaration, StoredHome, TenantId, User};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::RepositoryRegistry;
use pierre_test_support::db::create_test_db;
use uuid::Uuid;

const TENANT: &str = "00000000-0000-4000-8000-000000000001";
const OTHER_TENANT: &str = "00000000-0000-4000-8000-000000000002";

async fn user(repos: &RepositoryRegistry) -> Uuid {
    let user = User::new(
        format!("declared-home-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Declared Home Tester".to_owned()),
    );
    repos.users.create(&user).await.unwrap()
}

async fn read(repos: &RepositoryRegistry, tenant: &TenantId, user: Uuid) -> StoredHome {
    repos
        .athlete_places
        .home_declaration(tenant, user)
        .await
        .unwrap()
}

/// Run one statement with text binds; `Err` carries the database's refusal.
async fn execute(db: &Database, sql: &str, binds: &[&str]) -> Result<(), sqlx::Error> {
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            let mut query = sqlx::query(sql);
            for value in binds {
                query = query.bind(*value);
            }
            query.execute(sqlite.pool()).await.map(|_| ())
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            let mut query = sqlx::query(sql);
            for value in binds {
                query = query.bind(*value);
            }
            query.execute(pg.pool()).await.map(|_| ())
        }
    }
}

#[tokio::test]
async fn a_declared_home_round_trips_and_stays_with_its_user_and_tenant() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let tenant = TenantId::parse_str(TENANT).unwrap();
    let other_tenant = TenantId::parse_str(OTHER_TENANT).unwrap();
    let (who, neighbour) = (user(repos).await, user(repos).await);
    let places = &repos.athlete_places;
    let undeclared = StoredHome {
        home: HomeDeclaration::Inferred,
        declared_at: None,
    };
    assert_eq!(
        read(repos, &tenant, who).await,
        undeclared,
        "bug: a user who never declared reads a declaration"
    );

    let now = Utc::now();
    let seed = HomeDeclaration::At {
        lat: 46.812_345,
        lon: -71.208_765,
    };
    places
        .store_home_declaration(&tenant, who, seed, now)
        .await
        .unwrap();
    let stored = read(repos, &tenant, who).await;
    assert_eq!(stored.home, seed, "bug: the seed did not round-trip");
    let declared_at = stored.declared_at.expect("bug: a declaration has no date");
    assert!(
        (declared_at - now).num_milliseconds().abs() < 1,
        "bug: the declaration date moved: {declared_at} vs {now}"
    );
    assert_eq!(
        read(repos, &other_tenant, who).await,
        undeclared,
        "bug: a declaration leaked into another tenant"
    );
    assert_eq!(
        read(repos, &tenant, neighbour).await,
        undeclared,
        "bug: a declaration leaked to another user"
    );

    places
        .store_home_declaration(
            &tenant,
            who,
            HomeDeclaration::Elsewhere,
            now + Duration::seconds(1),
        )
        .await
        .unwrap();
    assert_eq!(
        read(repos, &tenant, who).await.home,
        HomeDeclaration::Elsewhere,
        "bug: elsewhere did not replace the seed"
    );

    places
        .store_home_declaration(
            &tenant,
            who,
            HomeDeclaration::Inferred,
            now + Duration::seconds(2),
        )
        .await
        .unwrap();
    let forgotten = read(repos, &tenant, who).await;
    assert_eq!(
        forgotten.home,
        HomeDeclaration::Inferred,
        "bug: forget did not bring back the inferred home"
    );
    assert!(
        forgotten.declared_at.is_some_and(|at| at > declared_at),
        "bug: forget dropped or did not move the row's date, which makes the user due"
    );
}

/// A whole insert into the declared home table, with `$1` the user and the
/// state and seed columns as written.
macro_rules! insert {
    ($state_and_seed:literal) => {
        concat!(
            "INSERT INTO athlete_declared_home (tenant_id, user_id, state, seed_lat, seed_lon, declared_at) ",
            "VALUES ('00000000-0000-4000-8000-000000000001', $1, ",
            $state_and_seed,
            ", CURRENT_TIMESTAMP)"
        )
    };
}

#[tokio::test]
async fn an_at_row_carries_its_whole_seed_and_no_other_state_carries_any() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let user = user(repos).await.to_string();
    for (sql, why) in [
        (insert!("'at', NULL, NULL"), "an at row without a seed"),
        (insert!("'at', 46.8, NULL"), "an at row with half a seed"),
        (
            insert!("'elsewhere', 46.8, NULL"),
            "an elsewhere row with half a seed",
        ),
        (
            insert!("'inferred', 46.8, -71.2"),
            "an inferred row with a seed",
        ),
        (insert!("'home', NULL, NULL"), "an unknown state"),
    ] {
        assert!(
            execute(&db, sql, &[user.as_str()]).await.is_err(),
            "bug: the table accepted {why}"
        );
    }
    execute(&db, insert!("'elsewhere', NULL, NULL"), &[user.as_str()])
        .await
        .expect("bug: the table refused a well-formed elsewhere row");
}
