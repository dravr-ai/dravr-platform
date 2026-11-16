// ABOUTME: carnet#1589 — a request logged while its admin's account is being deleted keeps no caller, in either order
// ABOUTME: Stages the two overlaps on whichever backend DATABASE_URL names: the log write waits, and the deletion scrubs what got in first

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A device login's request log outlives its admin's account, without the
//! address and the user agent each request came from. Two writes meet on
//! the login's row while that account is being deleted:
//!
//! - the deletion, which retires the login (it acts as no account from then
//!   on) and then clears the caller out of the log rows under it;
//! - a request still in flight, which appends its log row and records the
//!   caller only when the login still takes one.
//!
//! Whichever reaches the login's row first, no row is left with an address
//! under the retired login. A log write that comes second waits for the
//! deletion to end and reads the login as the deletion left it; a log write
//! that came first is committed before the deletion retires the login, and
//! the deletion's scrub, which runs after the retirement, clears it.
//!
//! `SQLite` has one writer, so the two never overlap there and the staging
//! is the write lock. `PostgreSQL` orders them by the login's row lock.
//!
//! **Only the `PostgreSQL` run judges the change.** The row lock is
//! `PostgreSQL` text, and on `SQLite` the second test passes with the
//! deletion's old order too (scrub, then retire): a green local
//! `cargo test -p pierre-database` says nothing about this window. The
//! `PostgreSQL` lane runs this binary; with the old order or without the
//! lock, one of the two tests fails there.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::time::Duration;

use pierre_core::admin::models::{DELETED_ACCOUNT_OPERATOR, DEVICE_CLI_SERVICE_PREFIX};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_test_support::account::delete_account;
use pierre_test_support::db::create_concurrent_test_db;
use sqlx::pool::PoolConnection;
#[cfg(feature = "postgresql")]
use sqlx::Postgres;
use sqlx::Sqlite;
use tokio::time;

use crate::admin_operator_tokens_test::{
    account, log_a_request_with_its_caller, mint, requests_of, token,
};

/// How long a write that must wait is watched not finishing.
const WATCHED_WAITING: Duration = Duration::from_millis(400);

/// A transaction held open on its own connection: the other writer the
/// engine has to order a statement against.
enum HeldTransaction {
    Sqlite(PoolConnection<Sqlite>),
    #[cfg(feature = "postgresql")]
    Postgres(PoolConnection<Postgres>),
}

impl HeldTransaction {
    /// Open one as a writer: `SQLite`'s takes the one write lock at once.
    async fn open(db: &Database) -> Self {
        match db.backend() {
            DatabaseBackend::SQLite(backend) => {
                let mut connection = backend.pool().acquire().await.unwrap();
                sqlx::query("BEGIN IMMEDIATE")
                    .execute(&mut *connection)
                    .await
                    .unwrap();
                Self::Sqlite(connection)
            }
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(backend) => {
                let mut connection = backend.pool().acquire().await.unwrap();
                sqlx::query("BEGIN")
                    .execute(&mut *connection)
                    .await
                    .unwrap();
                Self::Postgres(connection)
            }
        }
    }

    /// Run `statement` inside it, every bind a text.
    async fn run(&mut self, statement: &'static str, binds: &[&str]) {
        match self {
            Self::Sqlite(connection) => {
                let mut query = sqlx::query(statement);
                for bind in binds {
                    query = query.bind(*bind);
                }
                query.execute(&mut **connection).await.unwrap();
            }
            #[cfg(feature = "postgresql")]
            Self::Postgres(connection) => {
                let mut query = sqlx::query(statement);
                for bind in binds {
                    query = query.bind(*bind);
                }
                query.execute(&mut **connection).await.unwrap();
            }
        }
    }

    async fn commit(mut self) {
        self.run("COMMIT", &[]).await;
    }
}

/// What the deletion's retirement writes on a login's row, as far as a log
/// write reads it: inactive, acting as the id that stands for no account.
const RETIRE_THE_LOGIN: &str =
    "UPDATE admin_tokens SET is_active = FALSE, operator_user_id = $1 WHERE id = $2";

/// A request in flight when its admin's account is deleted writes its log
/// row and its use after the deletion retired the login and before the
/// deletion committed. Both writes wait for the deletion to end, and read
/// the login as it left it: the row says what the request did, without the
/// address or the user agent, and the use is not counted.
///
/// On `PostgreSQL` the log write read the login through a plain subquery,
/// which never waits: it saw the login as it was before the uncommitted
/// retirement, and landed a row with the caller's address under a login
/// the deletion had already scrubbed.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_log_write_that_meets_an_open_deletion_waits_and_keeps_no_caller() {
    let db = create_concurrent_test_db().await.unwrap();
    let repos = db.repositories();
    let leaving = account(repos, "leaving@example.com", true).await;
    account(repos, "staying@example.com", true).await;
    let label = format!("{DEVICE_CLI_SERVICE_PREFIX}{}", leaving.id);
    let login = mint(repos, &label, Some(leaving.id)).await;

    let mut deletion = HeldTransaction::open(&db).await;
    deletion
        .run(
            RETIRE_THE_LOGIN,
            &[&DELETED_ACCOUNT_OPERATOR.to_string(), &login],
        )
        .await;

    let logging = {
        let (db, login) = (db.clone(), login.clone());
        tokio::spawn(async move { log_a_request_with_its_caller(db.repositories(), &login).await })
    };
    let counting = {
        let (db, login) = (db.clone(), login.clone());
        tokio::spawn(async move {
            db.repositories()
                .admin
                .record_token_use(&login, chrono::Utc::now(), Some("203.0.113.9"))
                .await
        })
    };
    time::sleep(WATCHED_WAITING).await;
    assert!(
        !logging.is_finished(),
        "the log write waits for the deletion that retired its login to end"
    );
    assert!(
        !counting.is_finished(),
        "and so does the write that counts the use"
    );

    deletion.commit().await;
    logging.await.expect("the log write ran");
    counting
        .await
        .expect("the use write ran")
        .expect("and is answered");

    let requests = requests_of(repos, &login).await;
    assert_eq!(requests.len(), 1, "the request is logged");
    assert_eq!(
        (&requests[0].ip_address, &requests[0].user_agent),
        (&None, &None),
        "without the caller the deletion took out of the log"
    );
    let retired = token(repos, &login).await.expect("the login's row stays");
    assert_eq!(retired.last_used_ip, None, "no address is written back");
    assert_eq!(retired.usage_count, 0, "and no use is counted");
}

/// Hold the lock a log write takes on its login's row while it reads it,
/// where the engine has one to hold: `Some` on `PostgreSQL`, whose log
/// write reads the row `FOR SHARE`, and `None` on `SQLite`, where a writer
/// holds the whole database and nothing can be parked beside it.
async fn hold_a_log_writes_lock(db: &Database, login: &str) -> Option<HeldTransaction> {
    if matches!(db.backend(), DatabaseBackend::SQLite(_)) {
        return None;
    }
    let mut held = HeldTransaction::open(db).await;
    held.run(
        "SELECT id FROM admin_tokens WHERE id = $1 FOR SHARE",
        &[login],
    )
    .await;
    Some(held)
}

/// A request that logged its row just before the deletion retired its login
/// read a login that still took a caller, so its row carries the address.
/// The deletion clears it: the scrub runs after the retirement, on the
/// logins it retired, so it sees every row committed before the login
/// stopped taking a caller.
///
/// On `PostgreSQL` this is staged with the lock a log write holds: the
/// deletion waits at the login's row, a request logs its row meanwhile, and
/// the deletion goes on. Scrubbing first and retiring after, the deletion
/// had already cleared the log when it reached the row, and the request's
/// address stayed under the retired login. On `SQLite` a log write lands
/// wholly before the deletion or wholly after it, and before is the order
/// run here.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_request_logged_before_its_login_is_retired_is_scrubbed_by_the_deletion() {
    let db = create_concurrent_test_db().await.unwrap();
    let repos = db.repositories();
    let leaving = account(repos, "leaving@example.com", true).await;
    account(repos, "staying@example.com", true).await;
    let label = format!("{DEVICE_CLI_SERVICE_PREFIX}{}", leaving.id);
    let login = mint(repos, &label, Some(leaving.id)).await;

    if let Some(log_write) = hold_a_log_writes_lock(&db, &login).await {
        let deleting = {
            let (db, leaving) = (db.clone(), leaving.id);
            tokio::spawn(async move { delete_account(db.repositories(), leaving, None).await })
        };
        time::sleep(WATCHED_WAITING).await;
        assert!(
            !deleting.is_finished(),
            "the deletion waits at the row a log write holds"
        );
        log_a_request_with_its_caller(repos, &login).await;
        log_write.commit().await;
        deleting
            .await
            .expect("the deletion ran")
            .expect("and landed");
    } else {
        log_a_request_with_its_caller(repos, &login).await;
        delete_account(repos, leaving.id, None)
            .await
            .expect("the deletion lands");
    }

    let retired = token(repos, &login).await.expect("the login's row stays");
    assert_eq!(retired.operator_user_id, Some(DELETED_ACCOUNT_OPERATOR));
    let requests = requests_of(repos, &login).await;
    assert_eq!(requests.len(), 1, "the request's row stays");
    assert_eq!(
        (&requests[0].ip_address, &requests[0].user_agent),
        (&None, &None),
        "the deletion scrubbed the row logged just before it retired the login"
    );
}
