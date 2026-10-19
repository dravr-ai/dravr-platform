// ABOUTME: The room-facts migration: facts a group room filed under its channel tenant move to each athlete's own workspace
// ABOUTME: Replays each lane's migrations up to it, plants rooms, homes and restatements, and reads where every row landed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#926. A messaging group room stamped every ordinary turn's facts
//! under its conversation tenant — the channel's — while every reader of an
//! athlete's facts reads their own workspace. 20261019120000 moves those
//! rows home. It is proven on a real database of each lane, because what it
//! must NOT move matters as much as what it moves: a fact under another
//! tenant the athlete belongs to, filed outside any room, is read there by
//! their in-app turns, and a user with no workspace has nowhere to go. A
//! claim must come out once per workspace: a restatement folds into its home
//! copy, and copies said in the rooms of several channel tenants merge into
//! one survivor — the earliest created, the lowest id breaking a tie.
//!
//! "Home" is the runtime's tool tenant of a messaging turn: the earliest
//! membership in an active, owner-backed tenant (`authenticate_channel` →
//! `tenants.list_for_user().first()`).
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
const ROOM_FACTS_MIGRATION: i64 = 20_261_019_120_000;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
#[cfg(feature = "postgresql")]
static PG_MIGRATOR: Migrator = sqlx::migrate!("../../migrations_pg");

/// The athlete who spoke in the room.
const ATHLETE: &str = "aaaaaaaa-0000-4000-8000-000000000001";
/// An athlete who also belongs to a second, later tenant.
const MEMBER: &str = "aaaaaaaa-0000-4000-8000-000000000002";
/// The coach who owns the channel tenant, which is their own home.
const COACH: &str = "aaaaaaaa-0000-4000-8000-000000000003";
/// A linked user with no workspace at all.
const STRAY: &str = "aaaaaaaa-0000-4000-8000-000000000004";

const ATHLETE_HOME: &str = "11111111-0000-4000-8000-000000000001";
const MEMBER_HOME: &str = "11111111-0000-4000-8000-000000000002";
/// The channel tenant the room's conversations sit under.
const CHANNEL: &str = "11111111-0000-4000-8000-000000000003";
/// The coach's second tenant, which the member also joined, later.
const SECOND: &str = "11111111-0000-4000-8000-000000000004";

/// What the two backends share once the users exist: the tenants and their
/// memberships, two rooms and an in-app conversation, one message each, and
/// the facts. Literals only, so the same text plants either lane.
const FIXTURE: &str = r"
INSERT INTO tenants (id, name, slug, created_at, updated_at) VALUES
    ('11111111-0000-4000-8000-000000000001', 'Athlete', 'athlete-home', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('11111111-0000-4000-8000-000000000002', 'Member', 'member-home', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('11111111-0000-4000-8000-000000000003', 'Channel', 'channel', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('11111111-0000-4000-8000-000000000004', 'Second', 'second', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00');

INSERT INTO tenant_users (id, tenant_id, user_id, role, invited_at, joined_at) VALUES
    ('33333333-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000001', 'owner', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('33333333-0000-4000-8000-000000000002', '11111111-0000-4000-8000-000000000002', 'aaaaaaaa-0000-4000-8000-000000000002', 'owner', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('33333333-0000-4000-8000-000000000003', '11111111-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000003', 'owner', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('33333333-0000-4000-8000-000000000004', '11111111-0000-4000-8000-000000000004', 'aaaaaaaa-0000-4000-8000-000000000003', 'owner', '2026-01-02T00:00:00+00:00', '2026-01-02T00:00:00+00:00'),
    ('33333333-0000-4000-8000-000000000005', '11111111-0000-4000-8000-000000000004', 'aaaaaaaa-0000-4000-8000-000000000002', 'member', '2026-03-01T00:00:00+00:00', '2026-03-01T00:00:00+00:00');

INSERT INTO agents (id, user_id, tenant_id, title, system_prompt, created_at, updated_at) VALUES
    ('agent-room', 'aaaaaaaa-0000-4000-8000-000000000003', '11111111-0000-4000-8000-000000000003', 'Room Agent', 'You coach.', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00');

INSERT INTO coaching_groups (id, tenant_id, name, agent_id, owner_id, created_at, updated_at) VALUES
    ('22222222-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000003', 'Channel Room', 'agent-room', 'aaaaaaaa-0000-4000-8000-000000000003', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('22222222-0000-4000-8000-000000000002', '11111111-0000-4000-8000-000000000004', 'Second Room', 'agent-room', 'aaaaaaaa-0000-4000-8000-000000000003', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00');

INSERT INTO chat_conversations (id, user_id, tenant_id, title, group_id, created_at, updated_at) VALUES
    ('conv-athlete-room', 'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000003', 'Channel Room', '22222222-0000-4000-8000-000000000001', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('conv-member-room', 'aaaaaaaa-0000-4000-8000-000000000002', '11111111-0000-4000-8000-000000000004', 'Second Room', '22222222-0000-4000-8000-000000000002', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('conv-member-app', 'aaaaaaaa-0000-4000-8000-000000000002', '11111111-0000-4000-8000-000000000004', 'My training', NULL, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('conv-coach-room', 'aaaaaaaa-0000-4000-8000-000000000003', '11111111-0000-4000-8000-000000000003', 'Channel Room', '22222222-0000-4000-8000-000000000001', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00');

INSERT INTO chat_messages (id, conversation_id, role, content, created_at) VALUES
    ('msg-athlete-room', 'conv-athlete-room', 'assistant', 'Noted.', '2026-04-01T00:00:00+00:00'),
    ('msg-member-room', 'conv-member-room', 'assistant', 'Noted.', '2026-04-01T00:00:00+00:00'),
    ('msg-member-app', 'conv-member-app', 'assistant', 'Noted.', '2026-04-01T00:00:00+00:00'),
    ('msg-coach-room', 'conv-coach-room', 'assistant', 'Noted.', '2026-04-01T00:00:00+00:00');

INSERT INTO user_facts (id, tenant_id, user_id, scope, kind, predicate_code, object, confidence,
                        source, valid_until, source_msg_id, created_at, updated_at, first_party_only) VALUES
    -- Said in the room, cited by its message: moves home.
    ('fact-knee', '11111111-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000001', 'user', 'injury', 'have',
     'knee injury', 0.8, 'conversation', NULL, 'msg-athlete-room', '2026-04-01T00:00:00+00:00', '2026-04-01T00:00:00+00:00', FALSE),
    -- Its room is gone, but the athlete never belonged to the channel tenant: moves home.
    ('fact-orphan', '11111111-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000001', 'user', 'injury', 'recovering_from',
     'sprained ankle', 0.8, 'conversation', NULL, NULL, '2026-04-01T00:00:00+00:00', '2026-04-01T00:00:00+00:00', FALSE),
    -- A restatement of a claim in force at home: folded into it, then gone.
    ('fact-goal-room', '11111111-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000001', 'user', 'goal', 'training_for',
     'the mountain ultra', 0.9, 'conversation', NULL, 'msg-athlete-room', '2026-05-01T00:00:00+00:00', '2026-05-01T00:00:00+00:00', TRUE),
    ('fact-goal-home', '11111111-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000001', 'user', 'goal', 'training_for',
     'the mountain ultra', 0.5, 'conversation', NULL, NULL, '2026-04-01T00:00:00+00:00', '2026-04-01T00:00:00+00:00', FALSE),
    -- The same claim at home has expired: the live room row is not a restatement, it moves.
    ('fact-sleep-room', '11111111-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000001', 'user', 'schedule', 'prefer',
     'eight hours of sleep', 0.7, 'conversation', NULL, 'msg-athlete-room', '2026-05-01T00:00:00+00:00', '2026-05-01T00:00:00+00:00', FALSE),
    ('fact-sleep-home', '11111111-0000-4000-8000-000000000001', 'aaaaaaaa-0000-4000-8000-000000000001', 'user', 'schedule', 'prefer',
     'eight hours of sleep', 0.7, 'conversation', '2026-02-01T00:00:00+00:00', NULL, '2026-01-15T00:00:00+00:00', '2026-01-15T00:00:00+00:00', FALSE),
    -- The member's room under a tenant they belong to: moves home.
    ('fact-member-room', '11111111-0000-4000-8000-000000000004', 'aaaaaaaa-0000-4000-8000-000000000002', 'user', 'equipment', 'own',
     'a gravel bike', 0.8, 'conversation', NULL, 'msg-member-room', '2026-04-01T00:00:00+00:00', '2026-04-01T00:00:00+00:00', FALSE),
    -- The member's in-app turn under that same tenant: stays, its turns read it there.
    ('fact-member-app', '11111111-0000-4000-8000-000000000004', 'aaaaaaaa-0000-4000-8000-000000000002', 'user', 'equipment', 'own',
     'a track bike', 0.8, 'conversation', NULL, 'msg-member-app', '2026-04-01T00:00:00+00:00', '2026-04-01T00:00:00+00:00', FALSE),
    -- The coach's walk in their own room: the channel tenant is their home, it stays.
    ('fact-coach', '11111111-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000003', 'user', 'goal', 'training_for',
     'a spring marathon', 0.8, 'onboarding', NULL, 'msg-coach-room', '2026-04-01T00:00:00+00:00', '2026-04-01T00:00:00+00:00', FALSE),
    -- No workspace to move to: stays, as the runtime keeps them on the webhook tenant.
    ('fact-stray', '11111111-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000004', 'user', 'goal', 'training_for',
     'a first 10k', 0.8, 'conversation', NULL, NULL, '2026-04-01T00:00:00+00:00', '2026-04-01T00:00:00+00:00', FALSE);
";

/// One claim said in the rooms of several channel tenants, with no copy at
/// home: the copies must merge into one row there, not land home side by
/// side. A third channel tenant, the coach's, hosts the third copy; the
/// athlete belongs to none of the three.
const MERGE_FIXTURE: &str = r"
INSERT INTO tenants (id, name, slug, created_at, updated_at) VALUES
    ('11111111-0000-4000-8000-000000000005', 'Third', 'third', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00');

INSERT INTO tenant_users (id, tenant_id, user_id, role, invited_at, joined_at) VALUES
    ('33333333-0000-4000-8000-000000000006', '11111111-0000-4000-8000-000000000005', 'aaaaaaaa-0000-4000-8000-000000000003', 'owner', '2026-01-03T00:00:00+00:00', '2026-01-03T00:00:00+00:00');

INSERT INTO user_facts (id, tenant_id, user_id, scope, kind, predicate_code, object, confidence,
                        source, valid_until, source_msg_id, created_at, updated_at, first_party_only) VALUES
    -- Two copies. The earlier one survives; it takes the later copy's higher
    -- confidence, first-party stamp, updated_at and open-ended validity.
    ('fact-wheels-a', '11111111-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000001', 'user', 'equipment', 'own',
     'a carbon wheelset', 0.6, 'conversation', '2026-12-01T00:00:00+00:00', NULL, '2026-04-01T00:00:00+00:00', '2026-04-01T00:00:00+00:00', FALSE),
    ('fact-wheels-b', '11111111-0000-4000-8000-000000000005', 'aaaaaaaa-0000-4000-8000-000000000001', 'user', 'equipment', 'own',
     'a carbon wheelset', 0.85, 'conversation', NULL, NULL, '2026-06-01T00:00:00+00:00', '2026-06-01T00:00:00+00:00', TRUE),
    -- Three copies, two created at the same instant: the lower id survives.
    ('fact-strap-c', '11111111-0000-4000-8000-000000000003', 'aaaaaaaa-0000-4000-8000-000000000001', 'user', 'equipment', 'own',
     'a heart-rate strap', 0.7, 'conversation', NULL, NULL, '2026-03-01T00:00:00+00:00', '2026-07-01T00:00:00+00:00', FALSE),
    ('fact-strap-b', '11111111-0000-4000-8000-000000000004', 'aaaaaaaa-0000-4000-8000-000000000001', 'user', 'equipment', 'own',
     'a heart-rate strap', 0.5, 'conversation', NULL, NULL, '2026-03-01T00:00:00+00:00', '2026-03-01T00:00:00+00:00', FALSE),
    ('fact-strap-d', '11111111-0000-4000-8000-000000000005', 'aaaaaaaa-0000-4000-8000-000000000001', 'user', 'equipment', 'own',
     'a heart-rate strap', 0.95, 'conversation', NULL, NULL, '2026-05-01T00:00:00+00:00', '2026-05-01T00:00:00+00:00', TRUE);
";

/// Every fact's id and the tenant it sits under afterwards.
const EXPECTED_TENANTS: [(&str, &str); 11] = [
    ("fact-coach", CHANNEL),
    ("fact-goal-home", ATHLETE_HOME),
    ("fact-knee", ATHLETE_HOME),
    ("fact-member-app", SECOND),
    ("fact-member-room", MEMBER_HOME),
    ("fact-orphan", ATHLETE_HOME),
    ("fact-sleep-home", ATHLETE_HOME),
    ("fact-sleep-room", ATHLETE_HOME),
    ("fact-strap-b", ATHLETE_HOME),
    ("fact-stray", CHANNEL),
    ("fact-wheels-a", ATHLETE_HOME),
];

/// The rows a merge leaves behind, in this order: the home row the
/// restatement folded into, the survivor of two room copies, the survivor of
/// three.
const MERGED_ROWS: [&str; 3] = ["fact-goal-home", "fact-wheels-a", "fact-strap-b"];

/// What each of [`MERGED_ROWS`] must read afterwards: confidence, first-party
/// stamp, `updated_at`'s date, open-ended.
const EXPECTED_MERGED: [(f64, bool, &str, bool); 3] = [
    (0.9, true, "2026-05-01", true),
    (0.85, true, "2026-06-01", true),
    (0.95, true, "2026-07-01", true),
];

/// The rows merged away, which must be gone.
const MERGED_AWAY: [&str; 4] = [
    "fact-goal-room",
    "fact-wheels-b",
    "fact-strap-c",
    "fact-strap-d",
];

#[tokio::test]
async fn room_facts_move_to_each_athletes_own_workspace_and_nothing_else_moves() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        moves_on_postgres(&database.url).await;
        return;
    }
    moves_on_sqlite(&database.url).await;
}

/// What a merged row reads: confidence, first-party stamp, `updated_at`'s
/// date, and whether it is open-ended (no `valid_until`).
type Merged = (f64, bool, String, bool);

fn assert_landed(landed: &[(String, String)], merged: &[Merged]) {
    let landed: Vec<(&str, &str)> = landed
        .iter()
        .map(|(id, tenant)| (id.as_str(), tenant.as_str()))
        .collect();
    assert_eq!(
        landed,
        EXPECTED_TENANTS.to_vec(),
        "every room row is home, every duplicate is merged away, and nothing else moved"
    );
    for gone in MERGED_AWAY {
        assert!(
            !landed.iter().any(|(id, _)| *id == gone),
            "{gone} is merged, not duplicated at home"
        );
    }
    for (id, (row, want)) in MERGED_ROWS.iter().zip(merged.iter().zip(EXPECTED_MERGED)) {
        let (confidence, first_party_only, updated, open_ended) = row;
        let (want_confidence, want_first_party, want_updated, want_open) = want;
        assert!(
            (confidence - want_confidence).abs() < 1e-6,
            "{id} takes the highest confidence of its copies: {confidence}"
        );
        assert_eq!(
            *first_party_only, want_first_party,
            "{id}: the first-party stamp only tightens"
        );
        assert_eq!(updated, want_updated, "{id}: the latest updated_at stands");
        assert_eq!(
            *open_ended, want_open,
            "{id}: a merge never ends a claim earlier than one of its copies"
        );
    }
}

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

/// A database migrated up to, not including, the migration under test, and
/// that migration's SQL. One connection: every pooled connection to an
/// in-memory database is its own empty database. Foreign keys off, as the
/// migrations themselves run: the SQLite rebuilds left some references naming
/// the agents table by its old name.
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
        if migration.version == ROOM_FACTS_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the room-facts migration is in migrations/");
}

async fn moves_on_sqlite(url: &str) {
    let (pool, sql) = sqlite_before_migration(url).await;
    for user in [ATHLETE, MEMBER, COACH, STRAY] {
        sqlx::query(
            "INSERT INTO users (id, email, password_hash, created_at, last_active)
             VALUES ($1, $2, 'x', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
        )
        .bind(user)
        .bind(format!("{user}@room-facts.test"))
        .execute(&pool)
        .await
        .unwrap();
    }
    sqlx::raw_sql(FIXTURE).execute(&pool).await.unwrap();
    sqlx::raw_sql(MERGE_FIXTURE).execute(&pool).await.unwrap();

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    let (landed, merged) = sqlite_landed(&pool).await;
    assert_landed(&landed, &merged);

    // A second run finds every room row already home and moves nothing.
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    let (again, remerged) = sqlite_landed(&pool).await;
    assert_eq!(again, landed, "a re-run moves nothing");
    assert_eq!(remerged, merged, "a re-run merges nothing");
}

/// Where every fact sits, and what each merged row reads.
async fn sqlite_landed(pool: &sqlx::SqlitePool) -> (Vec<(String, String)>, Vec<Merged>) {
    let landed: Vec<(String, String)> =
        sqlx::query("SELECT id, tenant_id FROM user_facts ORDER BY id")
            .fetch_all(pool)
            .await
            .unwrap()
            .iter()
            .map(|row| (row.get("id"), row.get("tenant_id")))
            .collect();
    let mut merged = Vec::new();
    for id in MERGED_ROWS {
        let row = sqlx::query(
            "SELECT confidence, first_party_only, substr(updated_at, 1, 10) AS updated_on,
                    valid_until IS NULL AS open_ended
             FROM user_facts WHERE id = $1",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
        merged.push((
            row.get::<f64, _>("confidence"),
            row.get::<i64, _>("first_party_only") != 0,
            row.get::<String, _>("updated_on"),
            row.get::<i64, _>("open_ended") != 0,
        ));
    }
    (landed, merged)
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
        if migration.version == ROOM_FACTS_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the room-facts migration is in migrations_pg/");
}

#[cfg(feature = "postgresql")]
async fn moves_on_postgres(url: &str) {
    let (pool, sql) = postgres_before_migration(url).await;
    for user in [ATHLETE, MEMBER, COACH, STRAY] {
        sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1::uuid, $2, 'x')")
            .bind(user)
            .bind(format!("{user}@room-facts.test"))
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::raw_sql(FIXTURE).execute(&pool).await.unwrap();
    sqlx::raw_sql(MERGE_FIXTURE).execute(&pool).await.unwrap();

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    let (landed, merged) = postgres_landed(&pool).await;
    assert_landed(&landed, &merged);

    // A second run finds every room row already home and moves nothing.
    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    let (again, remerged) = postgres_landed(&pool).await;
    assert_eq!(again, landed, "a re-run moves nothing");
    assert_eq!(remerged, merged, "a re-run merges nothing");
}

/// Where every fact sits, and what each merged row reads.
#[cfg(feature = "postgresql")]
async fn postgres_landed(pool: &sqlx::PgPool) -> (Vec<(String, String)>, Vec<Merged>) {
    let landed: Vec<(String, String)> =
        sqlx::query("SELECT id, tenant_id FROM user_facts ORDER BY id COLLATE \"C\"")
            .fetch_all(pool)
            .await
            .unwrap()
            .iter()
            .map(|row| (row.get("id"), row.get("tenant_id")))
            .collect();
    let mut merged = Vec::new();
    for id in MERGED_ROWS {
        let row = sqlx::query(
            "SELECT confidence::float8 AS confidence, first_party_only,
                    to_char(updated_at AT TIME ZONE 'UTC', 'YYYY-MM-DD') AS updated_on,
                    valid_until IS NULL AS open_ended
             FROM user_facts WHERE id = $1",
        )
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap();
        merged.push((
            row.get::<f64, _>("confidence"),
            row.get::<bool, _>("first_party_only"),
            row.get::<String, _>("updated_on"),
            row.get::<bool, _>("open_ended"),
        ));
    }
    (landed, merged)
}
