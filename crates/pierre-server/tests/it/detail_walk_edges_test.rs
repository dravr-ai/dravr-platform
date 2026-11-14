// ABOUTME: The detail walk at its edges: a ledger that refuses its row, a budget with no room, a credential gone dead
// ABOUTME: A second cut short, a sport widened, a purge mid-read, a fresh walk beside one that ended, a ride stored late

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Detail walks at their edges (carnet#1483).
//
// This `//!` precedes the module-level `#![cfg]` so the docs survive a build
// without the features.
//
// The walk's main path is `detail_walk_test`; this suite drives what it
// meets at the edges, over the same loopback Strava stand-in.
#![cfg(all(
    feature = "tools-data",
    feature = "provider-strava",
    feature = "provider-garmin",
    feature = "health-sync"
))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::detail_walk_test::{mock_strava, since, store_rides, walk_row, DAY, SESSION};
use crate::history_walk_api_test::{open_window, server, strava_athlete};

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use pierre_core::constants::oauth::providers as oauth_providers;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::activity_backfill::detail_walk::{ask_detail_walk, DetailWalkAsked};
use pierre_tool_runtime::runtime::ToolRuntime;
use tokio::time::sleep;

/// The trigger, and on `PostgreSQL` the function behind it, that refuses
/// every detail walk row the ledger is asked to record.
const REFUSE_WALK_ROWS: &str = "refuse_detail_walk_rows";

/// Make the ledger refuse to record any detail walk row from now on.
async fn refuse_walk_rows(resources: &Arc<ServerContext>) {
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(&format!(
                "CREATE TRIGGER {REFUSE_WALK_ROWS} BEFORE INSERT ON activity_backfill_jobs \
                 WHEN NEW.kind = 'detail_walk' \
                 BEGIN SELECT RAISE(ABORT, 'detail walk row refused by the test'); END"
            ))
            .execute(db.pool())
            .await
            .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(&format!(
                "CREATE FUNCTION {REFUSE_WALK_ROWS}() RETURNS trigger LANGUAGE plpgsql \
                 AS $$ BEGIN RAISE EXCEPTION 'detail walk row refused by the test'; END $$"
            ))
            .execute(db.pool())
            .await
            .unwrap();
            sqlx::query(&format!(
                "CREATE TRIGGER {REFUSE_WALK_ROWS} BEFORE INSERT ON activity_backfill_jobs \
                 FOR EACH ROW WHEN (NEW.kind = 'detail_walk') \
                 EXECUTE FUNCTION {REFUSE_WALK_ROWS}()"
            ))
            .execute(db.pool())
            .await
            .unwrap();
        }
    }
}

/// Let the ledger record detail walk rows again.
async fn accept_walk_rows(resources: &Arc<ServerContext>) {
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(&format!("DROP TRIGGER {REFUSE_WALK_ROWS}"))
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(&format!(
                "DROP TRIGGER {REFUSE_WALK_ROWS} ON activity_backfill_jobs"
            ))
            .execute(db.pool())
            .await
            .unwrap();
        }
    }
}

/// A walk the ledger refuses to record is not started: the ask says it was
/// not recorded, and no run reads a ride for a walk with no row. Once the
/// ledger records again, the next ask starts the walk: the refused one held
/// nothing back.
#[tokio::test]
async fn a_walk_the_ledger_cannot_record_is_not_started() {
    let (api_base, mock) = mock_strava().await;
    let resources = server(&api_base, 400).await;
    let (user_id, tenant) = strava_athlete(&resources).await;
    let now = Utc::now().timestamp();
    store_rides(&resources, user_id, tenant, SESSION, &[now - 5 * DAY]).await;
    open_window(&resources).await;
    refuse_walk_rows(&resources).await;

    let runtime: Arc<dyn ToolRuntime> = resources.clone();
    let asked = ask_detail_walk(
        &runtime,
        user_id,
        tenant,
        oauth_providers::STRAVA,
        since(now - 10 * DAY),
    )
    .await;
    assert_eq!(
        asked,
        DetailWalkAsked::NotRecorded,
        "bug: a walk the ledger refused to record reads as started"
    );
    sleep(Duration::from_millis(200)).await;
    assert!(
        mock.detail_reads().is_empty(),
        "bug: a run was spawned for a walk with no row: {:?}",
        mock.detail_reads()
    );
    assert!(
        walk_row(&resources, user_id, tenant).await.is_none(),
        "the refused walk left no row"
    );

    accept_walk_rows(&resources).await;
    assert_eq!(
        ask_detail_walk(
            &runtime,
            user_id,
            tenant,
            oauth_providers::STRAVA,
            since(now - 10 * DAY),
        )
        .await,
        DetailWalkAsked::Started { units_total: 1 },
        "bug: the refused ask still holds the pair, so the next one never starts"
    );
}
