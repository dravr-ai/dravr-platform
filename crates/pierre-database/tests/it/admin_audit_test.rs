// ABOUTME: Pages the admin audit trail across rows that share one instant, on whichever backend DATABASE_URL names
// ABOUTME: The cursor is (timestamp, id), so a row tied with the page boundary is read on the next page, never skipped
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Several operator writes can land in one instant (a bulk console action, a
//! coarse clock). A cursor on the timestamp alone asked for rows strictly older
//! than the boundary, so the tied rows past it were never shown.

use chrono::{TimeZone, Utc};
use pierre_database::backends::factory::{Database, DatabaseBackend};
use pierre_database::repositories::{AuditEventFilter, AuditOutcome, AuditSource, NewAuditEvent};
use pierre_test_support::db::create_test_db;
use serde_json::json;
use uuid::Uuid;

/// Move every row of `action` to one instant: the repository stamps `now`,
/// which no test can make two rows share.
async fn stamp_all(db: &Database, action: &str) {
    let tied = Utc.with_ymd_and_hms(2026, 11, 4, 20, 31, 45).unwrap();
    let sql = "UPDATE audit_events SET timestamp = $1 WHERE event_type = $2";
    match db.backend() {
        DatabaseBackend::SQLite(sqlite) => {
            sqlx::query(sql)
                .bind(tied)
                .bind(action)
                .execute(sqlite.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(pg) => {
            sqlx::query(sql)
                .bind(tied)
                .bind(action)
                .execute(pg.pool())
                .await
                .unwrap();
        }
    }
}

#[tokio::test]
async fn the_trail_pages_through_rows_that_share_the_boundary_instant() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let action = format!("test.tied.{}", Uuid::new_v4().simple());
    let mut written = Vec::new();
    for n in 0..3 {
        let id = repos
            .admin_audit
            .record(&NewAuditEvent {
                action: action.clone(),
                summary: format!("tied row {n}"),
                outcome: AuditOutcome::Success,
                source: AuditSource::Console,
                actor_user_id: None,
                tenant_id: None,
                ip_address: None,
                user_agent: None,
                metadata: json!({}),
            })
            .await
            .unwrap();
        written.push(id);
    }
    stamp_all(&db, &action).await;

    let mut filter = AuditEventFilter {
        action_prefix: Some(action.clone()),
        ..AuditEventFilter::default()
    };
    let first = repos.admin_audit.list(&filter, 2).await.unwrap();
    assert_eq!(first.len(), 2);
    let last = first.last().unwrap();
    filter.before = Some(last.occurred_at);
    filter.before_id = Some(last.id.clone());
    let second = repos.admin_audit.list(&filter, 2).await.unwrap();

    let mut seen: Vec<String> = first.iter().chain(&second).map(|e| e.id.clone()).collect();
    seen.sort_unstable();
    written.sort_unstable();
    assert_eq!(
        seen, written,
        "bug: the row sharing the boundary instant must be on the next page"
    );
}
