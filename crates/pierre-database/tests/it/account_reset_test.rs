// ABOUTME: Holds the account reset's wipe and keep lists against the live schema: every user-keyed table is one or the other
// ABOUTME: And proves the wipe order: no reference into a wiped row is left without being cleared first or refused

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! An account reset keeps the `users` row, so no cascade from it fires and
//! every table the account owns rows in must be named (carnet#1463). These
//! tests read the catalog of whichever backend `DATABASE_URL` names and hold
//! the reset's lists to it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};

use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::repositories::{
    AccountResetWipe, POSTGRES_ACCOUNT_RESET, POSTGRES_ONLY_RESET_TABLES, RESET_BLOCKERS_SQL,
    RESET_KEEP, SQLITE_ACCOUNT_RESET,
};
use pierre_test_support::db::create_test_db;
use sqlx::Row;

/// Every table of the live schema with its column names.
async fn catalog(db: &Database) -> BTreeMap<String, Vec<String>> {
    const SQLITE: &str = "SELECT m.name AS tbl, c.name AS col \
        FROM sqlite_master m, pragma_table_info(m.name) c WHERE m.type = 'table'";
    #[cfg(feature = "postgresql")]
    const POSTGRES: &str = "SELECT CAST(c.relname AS TEXT) AS tbl, CAST(a.attname AS TEXT) AS col \
        FROM pg_catalog.pg_attribute a JOIN pg_catalog.pg_class c ON c.oid = a.attrelid \
        WHERE a.attnum > 0 AND NOT a.attisdropped AND c.relkind = 'r' \
          AND pg_catalog.pg_table_is_visible(c.oid)";
    let rows: Vec<(String, String)> = match db.backend() {
        DatabaseBackend::SQLite(sqlite) => sqlx::query(SQLITE)
            .fetch_all(sqlite.pool())
            .await
            .unwrap()
            .iter()
            .map(|r| (r.get("tbl"), r.get("col")))
            .collect(),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => sqlx::query(POSTGRES)
            .fetch_all(pg.pool())
            .await
            .unwrap()
            .iter()
            .map(|r| (r.get("tbl"), r.get("col")))
            .collect(),
    };
    let mut tables: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (table, column) in rows {
        tables.entry(table).or_default().push(column);
    }
    tables
}

/// A foreign key as the catalog reports it: `(child, child column, parent,
/// on-delete action)`.
type ForeignKey = (String, String, String, String);

/// Every foreign key of the live schema, and the engine's wipe list.
async fn foreign_keys(db: &Database) -> (Vec<ForeignKey>, AccountResetWipe) {
    const SQLITE: &str = r#"
        SELECT m.name AS child, f."from" AS child_column, f."table" AS parent,
               UPPER(f.on_delete) AS on_delete
        FROM sqlite_master m, pragma_foreign_key_list(m.name) f
        WHERE m.type = 'table'"#;
    #[cfg(feature = "postgresql")]
    const POSTGRES: &str = r"
        SELECT CAST(c.relname AS TEXT) AS child, CAST(a.attname AS TEXT) AS child_column,
               CAST(p.relname AS TEXT) AS parent,
               CASE k.confdeltype WHEN 'c' THEN 'CASCADE' WHEN 'n' THEN 'SET NULL'
                    WHEN 'd' THEN 'SET DEFAULT' ELSE 'NO ACTION' END AS on_delete
        FROM pg_catalog.pg_constraint k
        JOIN pg_catalog.pg_class c ON c.oid = k.conrelid
        JOIN pg_catalog.pg_class p ON p.oid = k.confrelid
        CROSS JOIN LATERAL pg_catalog.unnest(k.conkey) AS u(attnum)
        JOIN pg_catalog.pg_attribute a ON a.attrelid = k.conrelid AND a.attnum = u.attnum
        WHERE k.contype = 'f' AND pg_catalog.pg_table_is_visible(c.oid)";
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => (
            sqlx::query_as(SQLITE)
                .fetch_all(sqlite.pool())
                .await
                .unwrap(),
            SQLITE_ACCOUNT_RESET,
        ),
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => (
            sqlx::query_as(POSTGRES).fetch_all(pg.pool()).await.unwrap(),
            POSTGRES_ACCOUNT_RESET,
        ),
    }
}

/// A column naming a person: `user_id`, a role-prefixed `*_user_id`, a
/// group's `owner_id`, or an attribution `*_by`.
fn is_user_keyed(column: &str) -> bool {
    column == "user_id"
        || column.ends_with("_user_id")
        || column == "owner_id"
        || column.ends_with("_by")
}

#[tokio::test]
async fn every_user_keyed_table_is_wiped_or_kept_with_a_reason() {
    let db = create_test_db().await.unwrap();
    let tables = catalog(&db).await;
    let (keys, wipe) = foreign_keys(&db).await;
    let wiped: BTreeSet<&str> = wipe.tables.iter().copied().collect();
    let kept: BTreeSet<&str> = RESET_KEEP.iter().map(|(t, _)| *t).collect();

    assert_eq!(
        wiped.len(),
        wipe.tables.len(),
        "a table is wiped twice: {:?}",
        wipe.tables
    );
    assert_eq!(
        kept.len(),
        RESET_KEEP.len(),
        "a table is kept twice: {RESET_KEEP:?}"
    );
    assert!(
        wiped.is_disjoint(&kept),
        "a table is both wiped and kept: {:?}",
        wiped.intersection(&kept).collect::<Vec<_>>()
    );

    // A column bound to users(id) names a person whatever it is called.
    let referencing_users: BTreeSet<&str> = keys
        .iter()
        .filter(|(_, _, parent, _)| parent == "users")
        .map(|(child, _, _, _)| child.as_str())
        .collect();
    let unclassified: Vec<&String> = tables
        .iter()
        .filter(|(table, columns)| {
            (columns.iter().any(|c| is_user_keyed(c)) || referencing_users.contains(table.as_str()))
                && !wiped.contains(table.as_str())
                && !kept.contains(table.as_str())
        })
        .map(|(table, _)| table)
        .collect();
    assert!(
        unclassified.is_empty(),
        "these tables hold a user-keyed column and are neither wiped by the account reset nor in \
         RESET_KEEP with a reason: {unclassified:?}"
    );

    let stale: Vec<&&str> = wiped
        .iter()
        .chain(kept.iter())
        .filter(|t| !tables.contains_key(**t))
        .collect();
    assert!(
        stale.is_empty(),
        "the reset names tables absent from the schema: {stale:?}"
    );
    for (table, reason) in RESET_KEEP {
        assert!(
            !reason.trim().is_empty(),
            "{table} is kept without a reason"
        );
    }
    assert!(
        wiped.contains("chat_conversations") && kept.contains("users"),
        "the lists must be the reset's: {wiped:?}"
    );
}

/// The `PostgreSQL` list is the `SQLite` one followed by the tables only that
/// schema carries, so the two engines cannot wipe different things.
#[test]
fn the_postgres_wipe_is_the_sqlite_wipe_then_its_own_tables() {
    let expected: Vec<&str> = SQLITE_ACCOUNT_RESET
        .tables
        .iter()
        .chain(POSTGRES_ONLY_RESET_TABLES)
        .copied()
        .collect();
    assert_eq!(
        POSTGRES_ACCOUNT_RESET.tables,
        expected.as_slice(),
        "bug: the PostgreSQL wipe is not the SQLite wipe followed by its own tables"
    );
    for wipe in [SQLITE_ACCOUNT_RESET, POSTGRES_ACCOUNT_RESET] {
        assert_eq!(
            wipe.tables.len(),
            wipe.statements.len(),
            "bug: a wiped table has no statement: {:?}",
            wipe.tables
        );
    }
    for (table, statement) in POSTGRES_ACCOUNT_RESET
        .tables
        .iter()
        .zip(POSTGRES_ACCOUNT_RESET.statements)
    {
        assert!(
            statement.starts_with(&format!("DELETE FROM {table} WHERE ")),
            "{statement}"
        );
        assert!(statement.contains("$1"), "{statement} binds no account");
        assert!(
            !statement.contains("tenant_id"),
            "the reset is cross-tenant by design: {statement}"
        );
    }
}

/// The aliases `table` is read under in `sql`, after a `FROM` or a `JOIN`.
fn aliases_in<'a>(sql: &'a str, table: &str) -> Vec<&'a str> {
    sql.split_whitespace()
        .collect::<Vec<_>>()
        .windows(3)
        .filter(|w| matches!(w[0], "FROM" | "JOIN") && w[1] == table)
        .map(|w| w[2])
        .collect()
}

/// Whether `sql` reads `alias.column` as a whole qualified name.
fn reads_column(sql: &str, alias: &str, column: &str) -> bool {
    let wanted = format!("{alias}.{column}");
    sql.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
        .any(|token| token == wanted)
}

/// Every non-cascading foreign key into a row the reset removes that nothing
/// handles, as `child.column -> parent`.
///
/// The reset removes a table's rows at a step: a table `wipe` lists at its
/// position, and a table a `CASCADE` reaches at the earliest step of a parent
/// it cascades from. `users` is never removed, so nothing cascades from it. A
/// reference is handled when its child's rows go at a strictly earlier step
/// than its parent's.
///
/// A blocker handles it only for a child the reset removes no rows of: the
/// reset's blockers refuse over the rows it keeps (other users'), never the
/// account's own, so reading the column says nothing about a child the reset
/// wipes. Such a child must be cleared first whatever a blocker reads; one it
/// never touches is handled when `blockers` reads the referencing column, so
/// the refusal names the row before anything is disconnected.
fn unhandled_references(keys: &[ForeignKey], wipe: &[&str], blockers: &str) -> Vec<String> {
    let mut steps: BTreeMap<&str, usize> = wipe
        .iter()
        .enumerate()
        .rev()
        .map(|(at, table)| (*table, at))
        .collect();
    loop {
        let mut changed = false;
        for (child, _, parent, on_delete) in keys {
            if on_delete != "CASCADE" || parent == "users" {
                continue;
            }
            let Some(&parent_at) = steps.get(parent.as_str()) else {
                continue;
            };
            let child_at = steps.entry(child.as_str()).or_insert(usize::MAX);
            if parent_at < *child_at {
                *child_at = parent_at;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    keys.iter()
        .filter(|(child, _, parent, on_delete)| {
            !matches!(on_delete.as_str(), "CASCADE" | "SET NULL" | "SET DEFAULT")
                && parent != "users"
                && child != parent
        })
        .filter_map(|(child, column, parent, _)| {
            let parent_at = *steps.get(parent.as_str())?;
            let handled = steps.get(child.as_str()).map_or_else(
                || {
                    aliases_in(blockers, child)
                        .into_iter()
                        .any(|alias| reads_column(blockers, alias, column))
                },
                |child_at| *child_at < parent_at,
            );
            (!handled).then(|| format!("{child}.{column} -> {parent}"))
        })
        .collect()
}

/// The gate below follows a cascade from a wiped table, never one from
/// `users` (which a reset keeps), and holds a blocker to the column it reads.
#[test]
fn the_reset_reference_gate_ignores_the_users_cascade_and_reads_columns() {
    let key = |child: &str, column: &str, parent: &str, on_delete: &str| {
        (
            child.to_owned(),
            column.to_owned(),
            parent.to_owned(),
            on_delete.to_owned(),
        )
    };
    // A cascade from users removes nothing in a reset: the agent stays, so a
    // reference into it is no concern.
    let from_users = [
        key("agents", "user_id", "users", "CASCADE"),
        key("chat_conversations", "agent_id", "agents", "NO ACTION"),
    ];
    assert_eq!(
        unhandled_references(&from_users, &[], RESET_BLOCKERS_SQL),
        Vec::<String>::new()
    );
    // A wiped agent still referenced by a kept row fails the reset, unless a
    // blocker reads the column.
    assert_eq!(
        unhandled_references(
            &[key(
                "coaching_groups",
                "pinned_agent_id",
                "agents",
                "NO ACTION"
            )],
            &["agents"],
            RESET_BLOCKERS_SQL
        ),
        vec!["coaching_groups.pinned_agent_id -> agents".to_owned()]
    );
    assert_eq!(
        unhandled_references(
            &[key("coaching_groups", "agent_id", "agents", "NO ACTION")],
            &["agents"],
            RESET_BLOCKERS_SQL
        ),
        Vec::<String>::new()
    );
    // A blocker refuses only the rows the reset keeps (other users'), so a
    // child the reset wipes must still go before its parent, whatever column
    // a blocker reads: the account's own conversation names its agent.
    let own_rows = [key("chat_conversations", "agent_id", "agents", "NO ACTION")];
    assert_eq!(
        unhandled_references(
            &own_rows,
            &["agents", "chat_conversations"],
            RESET_BLOCKERS_SQL
        ),
        vec!["chat_conversations.agent_id -> agents".to_owned()]
    );
    assert_eq!(
        unhandled_references(
            &own_rows,
            &["chat_conversations", "agents"],
            RESET_BLOCKERS_SQL
        ),
        Vec::<String>::new()
    );
    // A cleared parent takes its children at its own step.
    let behind_a_cascade = [
        key(
            "chat_messages",
            "conversation_id",
            "chat_conversations",
            "CASCADE",
        ),
        key("message_marks", "message_id", "chat_messages", "NO ACTION"),
    ];
    assert_eq!(
        unhandled_references(
            &behind_a_cascade,
            &["chat_conversations", "message_marks"],
            RESET_BLOCKERS_SQL
        ),
        vec!["message_marks.message_id -> chat_messages".to_owned()]
    );
}

/// Every foreign key that does not cascade into a table the reset removes rows
/// from comes from a table the reset clears first, or from one it never
/// touches whose column the reset's blockers read, so no such reference can
/// fail the reset after the providers were already disconnected.
#[tokio::test]
async fn every_reference_into_what_the_reset_removes_is_cleared_first_or_refused() {
    let db = create_test_db().await.unwrap();
    let (keys, wipe) = foreign_keys(&db).await;
    assert!(
        keys.len() > 50,
        "the catalog read must see the schema's foreign keys: {}",
        keys.len()
    );
    assert!(
        keys.iter().any(
            |(child, column, parent, on_delete)| child == "messaging_messages"
                && column == "session_id"
                && parent == "messaging_sessions"
                && on_delete == "NO ACTION"
        ),
        "the catalog read must name each reference's column: {keys:?}"
    );

    let unhandled = unhandled_references(&keys, wipe.tables, RESET_BLOCKERS_SQL);
    assert!(
        unhandled.is_empty(),
        "these references fail the account reset with nothing to name them: {unhandled:?}"
    );
}
