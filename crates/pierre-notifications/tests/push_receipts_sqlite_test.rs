// ABOUTME: Runs dravr-commere's push-receipt sweep against the platform's migrated push_receipts_pending table
// ABOUTME: A due ticket Expo kept no receipt for is dropped, a fresh one is kept — proving the schema fits commere's SQL
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The platform owns the schema dravr-commere 0.6.0 records pending push
//! receipts in (carnet#984). commere's own tests run against a schema it
//! declares itself, so only a sweep over the platform's migrated database
//! proves the two agree: the insert's columns, the due-time comparison and
//! the delete.

#![cfg(feature = "sqlite")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use chrono::{Duration, Utc};
use dravr_commere::expo_push::ExpoPushService;
use dravr_commere::service::NotificationService;
use pierre_test_support::db::create_sqlite_test_db;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use uuid::Uuid;

/// An Expo stand-in that answers every request with an empty receipt map:
/// Expo keeps a receipt for 24 hours, so a ticket it no longer knows reads
/// as no receipt at all.
async fn expo_without_receipts() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut request = vec![0_u8; 16 * 1024];
            let _read = socket.read(&mut request).await;
            let body = r#"{"data":{}}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
            let _written = socket.write_all(response.as_bytes()).await;
        }
    });
    format!("http://{addr}")
}

async fn insert_pending(pool: &sqlx::SqlitePool, ticket_id: &str, minutes_ago: i64) {
    sqlx::query(
        "INSERT INTO push_receipts_pending \
         (ticket_id, device_token_id, notification_id, user_id, tenant_id, sent_at) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(ticket_id)
    .bind(Uuid::new_v4().to_string())
    .bind(Uuid::new_v4().to_string())
    .bind(Uuid::new_v4().to_string())
    .bind(Uuid::new_v4().to_string())
    .bind((Utc::now() - Duration::minutes(minutes_ago)).to_rfc3339())
    .execute(pool)
    .await
    .unwrap();
}

async fn pending_tickets(pool: &sqlx::SqlitePool) -> Vec<String> {
    sqlx::query_scalar("SELECT ticket_id FROM push_receipts_pending ORDER BY ticket_id")
        .fetch_all(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn the_sweep_drops_an_expired_ticket_and_keeps_a_fresh_one() {
    let db = create_sqlite_test_db().await.unwrap();
    let pool = db.sqlite_pool().unwrap().clone();

    // Two days old: past Expo's 24-hour retention, so its missing receipt
    // means it never will have one. Five minutes old: not due for 15 minutes.
    insert_pending(&pool, "ticket-expired", 2 * 24 * 60).await;
    insert_pending(&pool, "ticket-fresh", 5).await;

    let service = NotificationService::from_sqlite(pool.clone()).with_expo_push(
        ExpoPushService::with_api_base(expo_without_receipts().await),
    );
    let sweep = service.sweep_push_receipts().await.unwrap();

    assert_eq!(sweep.expired, 1, "{sweep:?}");
    assert_eq!(sweep.answered, 0, "{sweep:?}");
    assert_eq!(sweep.retired, 0, "{sweep:?}");
    assert_eq!(
        pending_tickets(&pool).await,
        vec!["ticket-fresh".to_owned()]
    );
}
