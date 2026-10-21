// ABOUTME: The coach-consents migration: each confirmed link's grants carry over to its member's row in the link's group
// ABOUTME: Replays each lane's migrations up to it, plants links in every state, and reads the member rows afterwards

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1055. 20261020120000 moves the calendar-write and wellness-read
//! consents off `delegated_connections` onto `coaching_group_members`, then
//! drops the link's columns. No member may lose a grant they gave, and no
//! grant may appear that they did not give: only a confirmed link's grants
//! carry, onto the member's row in that link's group, with the instant they
//! were given (the latest, when two confirmed links of theirs in the group
//! granted one). A proposal, an ended link and another group carry nothing.
//!
//! Each lane has its own migration set, so the factory's URL decides which
//! one is replayed.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::str::FromStr;

use pierre_test_support::db::create_test_db_url;
use sqlx::migrate::Migrator;
#[cfg(feature = "postgresql")]
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::Row;

/// The migration under test; everything before it builds the legacy shape.
const CONSENTS_MIGRATION: i64 = 20_261_020_120_000;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
#[cfg(feature = "postgresql")]
static PG_MIGRATOR: Migrator = sqlx::migrate!("../../migrations_pg");

const COACH: &str = "aaaaaaaa-0000-4000-8000-000000000001";
/// Two confirmed links in the squad, granting both consents between them.
const BOTH: &str = "aaaaaaaa-0000-4000-8000-000000000002";
/// A confirmed link that granted nothing.
const NONE: &str = "aaaaaaaa-0000-4000-8000-000000000003";
/// A proposal whose consent columns are set: it carries nothing.
const PROPOSED: &str = "aaaaaaaa-0000-4000-8000-000000000004";
/// A link that granted both, then ended: it carries nothing.
const REVOKED: &str = "aaaaaaaa-0000-4000-8000-000000000005";

const SQUAD: &str = "22222222-0000-4000-8000-000000000001";
/// Another group `BOTH` belongs to, with no link there.
const OTHER: &str = "22222222-0000-4000-8000-000000000002";

const FIXTURE: &str = r"
INSERT INTO tenants (id, name, slug, created_at, updated_at) VALUES
    ('11111111-0000-4000-8000-000000000001', 'Coach', 'coach', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00');

INSERT INTO agents (id, user_id, tenant_id, title, system_prompt, created_at, updated_at) VALUES
    ('agent-squad', 'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'Squad Agent', 'You coach.', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00');

INSERT INTO coaching_groups (id, tenant_id, name, agent_id, owner_id, created_at, updated_at) VALUES
    ('22222222-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'Squad', 'agent-squad', 'aaaaaaaa-0000-4000-8000-000000000001', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('22222222-0000-4000-8000-000000000002', '11111111-0000-4000-8000-000000000001', 'Other', 'agent-squad', 'aaaaaaaa-0000-4000-8000-000000000001', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00');

INSERT INTO coaching_group_members (id, group_id, user_id, tenant_id, role, peer_sharing_consent, consent_given_at, joined_at) VALUES
    ('33333333-0000-4000-8000-000000000001', '22222222-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000002', '11111111-0000-4000-8000-000000000001', 'member', FALSE, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('33333333-0000-4000-8000-000000000002', '22222222-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000003', '11111111-0000-4000-8000-000000000001', 'member', FALSE, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('33333333-0000-4000-8000-000000000003', '22222222-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000004', '11111111-0000-4000-8000-000000000001', 'member', FALSE, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('33333333-0000-4000-8000-000000000004', '22222222-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000005', '11111111-0000-4000-8000-000000000001', 'member', FALSE, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('33333333-0000-4000-8000-000000000005', '22222222-0000-4000-8000-000000000002', 'aaaaaaaa-0000-4000-8000-000000000002', '11111111-0000-4000-8000-000000000001', 'member', FALSE, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00');

INSERT INTO delegated_connections (id, provider, group_id, coach_user_id, coach_tenant_id, member_user_id,
        member_tenant_id, provider_athlete_id, status, proposed_at, confirmed_at, revoked_at, revoke_reason,
        calendar_write, calendar_write_at, wellness_read, wellness_read_at) VALUES
    -- BOTH: calendar granted on one link, wellness on that one and a later
    -- calendar grant on the other; the latest instant of each carries.
    ('44444444-0000-4000-8000-000000000001', 'intervals_icu', '22222222-0000-4000-8000-000000000001',
     'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000002',
     '11111111-0000-4000-8000-000000000001', 'i201', 'confirmed', '2026-09-01T00:00:00+00:00', '2026-09-02T00:00:00+00:00', NULL, NULL,
     TRUE, '2026-09-02T08:00:00+00:00', TRUE, '2026-09-03T09:00:00+00:00'),
    ('44444444-0000-4000-8000-000000000002', 'sciotte_trainingpeaks', '22222222-0000-4000-8000-000000000001',
     'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000002',
     '11111111-0000-4000-8000-000000000001', '900001', 'confirmed', '2026-09-01T00:00:00+00:00', '2026-09-02T00:00:00+00:00', NULL, NULL,
     TRUE, '2026-09-04T10:00:00+00:00', FALSE, NULL),
    -- NONE: confirmed, nothing granted.
    ('44444444-0000-4000-8000-000000000003', 'intervals_icu', '22222222-0000-4000-8000-000000000001',
     'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000003',
     '11111111-0000-4000-8000-000000000001', 'i202', 'confirmed', '2026-09-01T00:00:00+00:00', '2026-09-02T00:00:00+00:00', NULL, NULL,
     FALSE, NULL, FALSE, NULL),
    -- PROPOSED: never confirmed, whatever its columns say.
    ('44444444-0000-4000-8000-000000000004', 'intervals_icu', '22222222-0000-4000-8000-000000000001',
     'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000004',
     NULL, 'i203', 'proposed', '2026-09-01T00:00:00+00:00', NULL, NULL, NULL,
     TRUE, '2026-09-02T08:00:00+00:00', TRUE, '2026-09-02T08:00:00+00:00'),
    -- REVOKED: granted both, then ended.
    ('44444444-0000-4000-8000-000000000005', 'intervals_icu', '22222222-0000-4000-8000-000000000001',
     'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000005',
     '11111111-0000-4000-8000-000000000001', 'i204', 'revoked', '2026-09-01T00:00:00+00:00', '2026-09-02T00:00:00+00:00', '2026-09-05T00:00:00+00:00', 'revoked_by_member',
     TRUE, '2026-09-02T08:00:00+00:00', TRUE, '2026-09-02T08:00:00+00:00');
";

/// A member row afterwards: calendar write, when (to the second), wellness
/// read, when.
type Consents = (bool, Option<String>, bool, Option<String>);

/// Every member row, keyed by group then user, and what it must read.
fn expected() -> Vec<(&'static str, &'static str, Consents)> {
    let none: Consents = (false, None, false, None);
    vec![
        (
            SQUAD,
            BOTH,
            (
                true,
                Some("2026-09-04T10:00:00".to_owned()),
                true,
                Some("2026-09-03T09:00:00".to_owned()),
            ),
        ),
        (SQUAD, NONE, none.clone()),
        (SQUAD, PROPOSED, none.clone()),
        (SQUAD, REVOKED, none.clone()),
        (OTHER, BOTH, none),
    ]
}

fn assert_carried(rows: &[(String, String, Consents)]) {
    let got: Vec<(&str, &str, Consents)> = rows
        .iter()
        .map(|(group, user, consents)| (group.as_str(), user.as_str(), consents.clone()))
        .collect();
    let mut want = expected();
    want.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    assert_eq!(
        got, want,
        "only a confirmed link's grants carry, onto its group's member row, latest instant first"
    );
}

/// The first nineteen characters of a stamp: the instant to the second,
/// whatever each engine appends.
fn second(at: &str) -> String {
    at.chars().take(19).collect()
}

#[tokio::test]
async fn a_confirmed_links_grants_carry_to_its_members_row_and_nothing_else_does() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        carries_on_postgres(&database.url).await;
        return;
    }
    carries_on_sqlite(&database.url).await;
}

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

/// A database migrated up to, not including, the migration under test, and
/// that migration's SQL. One connection: every pooled connection to an
/// in-memory database is its own empty database. Foreign keys off, as the
/// migrations themselves run.
async fn sqlite_before_migration(url: &str) -> (sqlx::SqlitePool, String) {
    let options = SqliteConnectOptions::from_str(url)
        .unwrap()
        .foreign_keys(false);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    for migration in MIGRATOR.iter() {
        if migration.version == CONSENTS_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the coach-consents migration is in migrations/");
}

async fn carries_on_sqlite(url: &str) {
    let (pool, sql) = sqlite_before_migration(url).await;
    for user in [COACH, BOTH, NONE, PROPOSED, REVOKED] {
        sqlx::query(
            "INSERT INTO users (id, email, password_hash, created_at, last_active)
             VALUES ($1, $2, 'x', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(user)
        .bind(format!("{user}@consents.test"))
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::raw_sql(FIXTURE).execute(&pool).await.unwrap();

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    let rows: Vec<(String, String, Consents)> = sqlx::query(
        "SELECT group_id, user_id, coach_calendar_write, coach_calendar_write_at,
                coach_wellness_read, coach_wellness_read_at
         FROM coaching_group_members ORDER BY group_id, user_id",
    )
    .fetch_all(&pool)
    .await
    .unwrap()
    .iter()
    .map(|row| {
        (
            row.get("group_id"),
            row.get("user_id"),
            (
                row.get::<i64, _>("coach_calendar_write") != 0,
                row.get::<Option<String>, _>("coach_calendar_write_at")
                    .as_deref()
                    .map(second),
                row.get::<i64, _>("coach_wellness_read") != 0,
                row.get::<Option<String>, _>("coach_wellness_read_at")
                    .as_deref()
                    .map(second),
            ),
        )
    })
    .collect();
    assert_carried(&rows);

    let left: i64 = sqlx::query(
        "SELECT COUNT(*) AS n FROM pragma_table_info('delegated_connections')
         WHERE name IN ('calendar_write', 'calendar_write_at', 'wellness_read', 'wellness_read_at')",
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .get("n");
    assert_eq!(left, 0, "the link's consent columns are dropped");
}

// ---------------------------------------------------------------------------
// PostgreSQL
// ---------------------------------------------------------------------------

/// A database migrated up to, not including, the migration under test, and
/// that migration's SQL. The factory hands out a clone of the migrated
/// template, so start again from an empty schema.
#[cfg(feature = "postgresql")]
async fn postgres_before_migration(url: &str) -> (sqlx::PgPool, String) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .unwrap();
    sqlx::raw_sql("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
        .execute(&pool)
        .await
        .unwrap();
    for migration in PG_MIGRATOR.iter() {
        if migration.version == CONSENTS_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the coach-consents migration is in migrations_pg/");
}

#[cfg(feature = "postgresql")]
async fn carries_on_postgres(url: &str) {
    let (pool, sql) = postgres_before_migration(url).await;
    for user in [COACH, BOTH, NONE, PROPOSED, REVOKED] {
        sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1::uuid, $2, 'x')")
            .bind(user)
            .bind(format!("{user}@consents.test"))
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::raw_sql(FIXTURE).execute(&pool).await.unwrap();

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    let rows = postgres_rows(&pool).await;
    assert_carried(&rows);

    // A second run finds the link's columns gone and changes nothing.
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    assert_eq!(postgres_rows(&pool).await, rows, "a re-run carries nothing");
}

#[cfg(feature = "postgresql")]
async fn postgres_rows(pool: &sqlx::PgPool) -> Vec<(String, String, Consents)> {
    sqlx::query(
        "SELECT group_id::text AS group_id, user_id::text AS user_id,
                coach_calendar_write,
                to_char(coach_calendar_write_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS') AS calendar_at,
                coach_wellness_read,
                to_char(coach_wellness_read_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS') AS wellness_at
         FROM coaching_group_members ORDER BY group_id::text, user_id::text",
    )
    .fetch_all(pool)
    .await
    .unwrap()
    .iter()
    .map(|row| {
        (
            row.get("group_id"),
            row.get("user_id"),
            (
                row.get("coach_calendar_write"),
                row.get("calendar_at"),
                row.get("coach_wellness_read"),
                row.get("wellness_at"),
            ),
        )
    })
    .collect()
}
