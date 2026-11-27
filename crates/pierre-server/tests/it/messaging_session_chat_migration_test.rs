// ABOUTME: The session-chat backfill: which stored messaging sessions 20261122153000 records as a DM's or a room's
// ABOUTME: Replays each lane's migrations up to it, plants one session per adapter rule, and reads what each was stamped

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1422. A session whose chat is not on record is read as a room on
//! every thread, so every session the stored ids can decide is decided by
//! the backfill, by the pinned messaging adapter's own rules (dravr-canot
//! v0.14.0): a Telegram chat is a DM only when private (a positive id equal
//! to its user's), a Slack channel only when its id starts with `D`;
//! `WhatsApp` and Messenger deliver only one-to-one messages; a session
//! writing into a group-bound row is a group chat's whatever its channel.
//! Discord keeps no stored trace of a DM, so its sessions wait for their next
//! turn. The ungrouped Telegram group chat whose row was designated personal
//! before the branch is the case that mattered: left unrecorded, it read as
//! the owner's direct message.

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
const SESSION_CHAT_MIGRATION: i64 = 20_261_122_153_000;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");
#[cfg(feature = "postgresql")]
static PG_MIGRATOR: Migrator = sqlx::migrate!("../../migrations_pg");

/// The coach every session belongs to.
const COACH: &str = "aaaaaaaa-0000-4000-8000-000000000001";

/// The coach's workspace, its group's agent and group, the group's room, and
/// the coach's ungrouped row a Telegram group chat writes into, designated
/// personal before carnet#1422. Literals only, so the same text plants
/// either lane.
const FIXTURE: &str = r"
INSERT INTO tenants (id, name, slug, created_at, updated_at) VALUES
    ('11111111-0000-4000-8000-000000000001', 'Coach', 'coach-home', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00');

INSERT INTO agents (id, user_id, tenant_id, title, system_prompt, created_at, updated_at) VALUES
    ('agent-room', 'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'Room Agent', 'You coach.', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00');

INSERT INTO coaching_groups (id, tenant_id, name, agent_id, owner_id, created_at, updated_at) VALUES
    ('22222222-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'Squad', 'agent-room', 'aaaaaaaa-0000-4000-8000-000000000001', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00');

INSERT INTO chat_conversations (id, user_id, tenant_id, title, group_id, created_at, updated_at) VALUES
    ('conv-room', 'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'Squad', '22222222-0000-4000-8000-000000000001', '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00'),
    ('conv-personal', 'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'Home', NULL, '2026-01-01T00:00:00+00:00', '2026-01-01T00:00:00+00:00');

INSERT INTO messaging_sessions (id, user_id, tenant_id, channel_type, channel_user_id, channel_conversation_id, pierre_conversation_id) VALUES
    ('s-wa-room', 'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'whatsapp', 'wa-room', NULL, 'conv-room'),
    ('s-tg-group', 'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'telegram', '42', '-100123', 'conv-personal'),
    ('s-tg-private', 'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'telegram', '43', '43', 'conv-personal'),
    ('s-slack-dm', 'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'slack', 'U1', 'D123', NULL),
    ('s-slack-public', 'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'slack', 'U2', 'C123', NULL),
    ('s-slack-private', 'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'slack', 'U3', 'G123', NULL),
    ('s-whatsapp', 'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'whatsapp', 'wa-1', NULL, 'conv-personal'),
    ('s-messenger', 'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'messenger', 'm-1', NULL, NULL),
    ('s-discord', 'aaaaaaaa-0000-4000-8000-000000000001', '11111111-0000-4000-8000-000000000001', 'discord', 'd-1', '987654321', 'conv-personal');
";

/// What each session is recorded as afterwards, by id.
const EXPECTED: [(&str, Option<bool>); 9] = [
    ("s-discord", None),
    ("s-messenger", Some(true)),
    ("s-slack-dm", Some(true)),
    ("s-slack-private", Some(false)),
    ("s-slack-public", Some(false)),
    ("s-tg-group", Some(false)),
    ("s-tg-private", Some(true)),
    ("s-wa-room", Some(false)),
    ("s-whatsapp", Some(true)),
];

#[tokio::test]
async fn every_session_the_stored_ids_decide_is_recorded_by_the_adapter_rules() {
    let database = create_test_db_url().await.unwrap();
    #[cfg(feature = "postgresql")]
    if database.url.starts_with("postgres") {
        stamps_on_postgres(&database.url).await;
        return;
    }
    stamps_on_sqlite(&database.url).await;
}

fn assert_stamped(stamped: &[(String, Option<bool>)]) {
    let stamped: Vec<(&str, Option<bool>)> = stamped
        .iter()
        .map(|(id, record)| (id.as_str(), *record))
        .collect();
    assert_eq!(
        stamped,
        EXPECTED.to_vec(),
        "every session the adapter rules decide is recorded, a Telegram group chat a room, \
         and a Discord session waits for its next turn"
    );
}

// ---------------------------------------------------------------------------
// SQLite
// ---------------------------------------------------------------------------

/// A database migrated up to, not including, the migration under test, and
/// that migration's SQL. One connection: every pooled connection to an
/// in-memory database is its own empty database.
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
        if migration.version == SESSION_CHAT_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the session-chat migration is in migrations/");
}

async fn stamps_on_sqlite(url: &str) {
    let (pool, sql) = sqlite_before_migration(url).await;
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, created_at, last_active)
         VALUES ($1, 'coach@session-chat.test', 'x', '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z')",
    )
    .bind(COACH)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(FIXTURE).execute(&pool).await.unwrap();

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    let stamped = sqlx::query("SELECT id, is_direct_message FROM messaging_sessions ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap()
        .iter()
        .map(|row| {
            (
                row.get::<String, _>("id"),
                row.get::<Option<bool>, _>("is_direct_message"),
            )
        })
        .collect::<Vec<_>>();
    assert_stamped(&stamped);
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
        if migration.version == SESSION_CHAT_MIGRATION {
            return (pool, migration.sql.to_string());
        }
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    panic!("the session-chat migration is in migrations_pg/");
}

#[cfg(feature = "postgresql")]
async fn stamps_on_postgres(url: &str) {
    let (pool, sql) = postgres_before_migration(url).await;
    sqlx::query(
        "INSERT INTO users (id, email, password_hash) VALUES ($1::uuid, 'coach@session-chat.test', 'x')",
    )
    .bind(COACH)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(FIXTURE).execute(&pool).await.unwrap();

    sqlx::raw_sql(&sql).execute(&pool).await.unwrap();
    let stamped = sqlx::query("SELECT id, is_direct_message FROM messaging_sessions ORDER BY id")
        .fetch_all(&pool)
        .await
        .unwrap()
        .iter()
        .map(|row| {
            (
                row.get::<String, _>("id"),
                row.get::<Option<bool>, _>("is_direct_message"),
            )
        })
        .collect::<Vec<_>>();
    assert_stamped(&stamped);
}
