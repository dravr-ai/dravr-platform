// ABOUTME: Response date goldens for the admin console routes: accounts, the audit trail and its page cursor
// ABOUTME: Pins account stamps written with an offset, audit instants, and how the `before` cursor is read
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Response date goldens: `/api/admin/users`, `/api/admin/audit-events`,
//! `/api/admin/analytics/new-users`, `/api/dashboard/analytics`
//! (carnet#1620, gate 3).
//!
//! Accounts are created with the stamps the suite chooses. The audit trail
//! stamps each row with the wall clock, so two rows are recorded through the
//! repository and moved to chosen instants with one `UPDATE` each on the test
//! database. Every read goes through the server's whole HTTP app, as an
//! admin's console session.
//!
//! Stamped with the wall clock, and so compared by shape, which keeps the
//! clock's fraction: `generated_at` on the new-users analytics. The
//! signed-in admin's own `last_active`, which signing in moves with the
//! database's clock, is compared by its whole shape (whole seconds on
//! SQLite).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use axum::http::StatusCode;
use axum::Router;
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_database::repositories::{AuditOutcome, AuditSource, NewAuditEvent};
use pierre_database::seed_models::SeedLlmUsageRecord;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::{json, Value};
use serial_test::serial;
use uuid::Uuid;

use crate::common;
use crate::helpers::date_golden::{
    dates_in, dates_in_stamped, get, instant, on_database_clock, on_the_clock, restamp,
    seed_account, send, shifted, table, with_values, Athlete, Today,
};

const MOVE_AUDIT_EVENT: &str =
    "UPDATE audit_events SET timestamp = $1 WHERE id = $2 AND CAST(tenant_id AS TEXT) = $3";

struct Console {
    resources: Arc<ServerContext>,
    app: Router,
    admin: Athlete,
}

impl Console {
    /// An admin created at 09:30:00.250125 on 2026-08-01, signed in.
    async fn new(label: &str) -> Self {
        let resources = common::create_test_server_resources().await.unwrap();
        let app = ProviderToolRouter::build_http_app(&resources);
        let admin = seed_account(
            &resources,
            label,
            UserRole::Admin,
            instant("2026-08-01T09:30:00.250125"),
            instant("2026-08-01T09:30:00.250125"),
        )
        .await;
        Self {
            resources,
            app,
            admin,
        }
    }

    async fn read(&self, uri: &str) -> (StatusCode, Value) {
        send(self.app.clone(), get(uri, &self.admin.token)).await
    }

    async fn read_ok(&self, uri: &str) -> Value {
        let (status, body) = self.read(uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        body
    }

    /// One audit row by this admin, moved to `at`; its id.
    async fn audit_row(&self, action: &str, at: &str) -> String {
        let id = self
            .resources
            .common
            .repos
            .admin_audit
            .record(&NewAuditEvent {
                action: action.to_owned(),
                summary: format!("{action} for the golden"),
                outcome: AuditOutcome::Success,
                source: AuditSource::Console,
                actor_user_id: Some(self.admin.user_id),
                tenant_id: Some(self.admin.tenant.to_string()),
                ip_address: None,
                user_agent: None,
                metadata: json!({}),
            })
            .await
            .unwrap();
        restamp(
            &self.resources.agent.database,
            MOVE_AUDIT_EVENT,
            instant(at),
            &id,
            Some(self.admin.tenant),
        )
        .await;
        id
    }
}

/// The row of `users` (or `admins`) that is `account`'s.
fn row_of<'a>(rows: &'a Value, account: &Athlete) -> &'a Value {
    let id = account.user_id.to_string();
    rows.as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id.as_str())
        .unwrap_or_else(|| panic!("{id} is listed: {rows}"))
}

#[tokio::test]
#[serial]
async fn the_account_list_writes_each_stamp_in_utc_with_an_offset() {
    on_the_clock(|| async {
        let console = Console::new("golden-admin-users").await;
        let athlete = seed_account(
            &console.resources,
            "golden-admin-athlete",
            UserRole::User,
            instant("2026-08-02T10:00:00"),
            instant("2026-08-29T10:15:30.250125"),
        )
        .await;

        let body = console.read_ok("/api/admin/users").await;
        let users = &body["data"]["users"];

        // Whole seconds, milliseconds and microseconds each keep their own width.
        assert_eq!(
            dates_in(row_of(users, &athlete)),
            table(&[
                ("/approved_at", "2026-08-02T10:00:00+00:00"),
                ("/created_at", "2026-08-02T10:00:00+00:00"),
                ("/last_active", "2026-08-29T10:15:30.250125+00:00"),
            ]),
            "{users}"
        );
        // Signing in moved `last_active` with the database's clock: whole
        // seconds on SQLite, microseconds on PostgreSQL.
        let database = &console.resources.agent.database;
        let last_active = match database.backend() {
            DatabaseBackend::SQLite(_) => "dddd-dd-ddTdd:dd:dd+dd:dd",
            #[cfg(feature = "postgresql")]
            DatabaseBackend::PostgreSQL(_) => "dddd-dd-ddTdd:dd:dd.f+dd:dd",
        };
        assert_eq!(
            on_database_clock(
                database,
                dates_in(row_of(users, &console.admin)),
                &["/last_active"]
            ),
            table(&[
                ("/approved_at", "2026-08-01T09:30:00.250125+00:00"),
                ("/created_at", "2026-08-01T09:30:00.250125+00:00"),
                ("/last_active", last_active),
            ]),
            "{users}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn the_new_users_analytics_are_stamped_with_z() {
    on_the_clock(|| async {
        let console = Console::new("golden-admin-analytics").await;

        let body = console.read_ok("/api/admin/analytics/new-users").await;

        assert_eq!(
            dates_in_stamped(&body["data"]["generated_at"], &[""]),
            table(&[("", "dddd-dd-ddTdd:dd:dd.fZ")]),
            "{body}"
        );
    })
    .await;
}

#[tokio::test]
#[serial]
async fn the_audit_trail_answers_each_rows_instant_and_the_cursor_of_the_next_page() {
    let console = Console::new("golden-admin-audit").await;
    console
        .audit_row("golden.older", "2026-08-28T02:59:03.500")
        .await;
    console
        .audit_row("golden.newer", "2026-08-29T10:15:30")
        .await;

    let body = console
        .read_ok("/api/admin/audit-events?action=golden.")
        .await;
    assert_eq!(
        dates_in(&body["data"]),
        table(&[
            ("/events/0/occurred_at", "2026-08-29T10:15:30Z"),
            ("/events/1/occurred_at", "2026-08-28T02:59:03.500Z"),
        ]),
        "{body}"
    );

    // A full page names where the next one starts.
    let first = console
        .read_ok("/api/admin/audit-events?action=golden.&limit=1")
        .await;
    assert_eq!(
        dates_in(&first["data"]),
        table(&[
            ("/events/0/occurred_at", "2026-08-29T10:15:30Z"),
            ("/next_before", "2026-08-29T10:15:30Z"),
        ]),
        "{first}"
    );
}

#[tokio::test]
#[serial]
async fn the_audit_cursor_is_read_as_an_instant_in_any_offset() {
    let console = Console::new("golden-admin-cursor").await;
    console
        .audit_row("golden.older", "2026-08-28T02:59:03.500")
        .await;
    console
        .audit_row("golden.newer", "2026-08-29T10:15:30")
        .await;

    // (before, status, the actions strictly older than it).
    let read: [(&str, u16, &[&str]); 9] = [
        // The newer row's own instant, in UTC, Kolkata and Toronto.
        ("2026-08-29T10:15:30Z", 200, &["golden.older"]),
        ("2026-08-29T15:45:30+05:30", 200, &["golden.older"]),
        ("2026-08-29T06:15:30-04:00", 200, &["golden.older"]),
        // One millisecond later holds both; the older row's own instant none.
        (
            "2026-08-29T10:15:30.001Z",
            200,
            &["golden.newer", "golden.older"],
        ),
        ("2026-08-28T02:59:03.500Z", 200, &[]),
        ("2026-08-28T02:59:03.501Z", 200, &["golden.older"]),
        // A space for the `T` is read.
        ("2026-08-29 10:15:30Z", 200, &["golden.older"]),
        // Not instants: a date alone, no zone.
        ("2026-08-29", 400, &[]),
        ("2026-08-29T10:15:30", 400, &[]),
    ];
    for (before, status, expected) in read {
        let uri = format!(
            "/api/admin/audit-events?action=golden.&before={}",
            urlencoding::encode(before)
        );
        let (answered, body) = console.read(&uri).await;
        assert_eq!(answered.as_u16(), status, "{before}: {body}");
        if status == 200 {
            let actions: Vec<&str> = body["data"]["events"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|event| event["action"].as_str())
                .collect();
            assert_eq!(actions, expected, "{before}: {body}");
        }
    }
}

/// The dashboard's series is one point a UTC day, each named by hand
/// (`%Y-%m-%d`) and joined to the calls counted under the same key: a call
/// in the last half second of a UTC day and one in the first second of the
/// next fall on two points.
#[tokio::test]
#[serial]
async fn the_dashboard_series_counts_each_call_on_its_utc_day() {
    let (today, body) = loop {
        let today = Today::read();
        // A fresh console each time round: a midnight between two rounds
        // would otherwise count the first round's calls on other days.
        let console = Console::new("golden-admin-dashboard").await;
        let yesterday = shifted(today.utc, -1);
        for at in [
            format!("{yesterday}T23:59:59.500"),
            format!("{yesterday}T12:00:00"),
            format!("{}T00:00:00", shifted(today.utc, 0)),
        ] {
            console
                .resources
                .common
                .repos
                .seeder
                .seed_insert_llm_usage(&SeedLlmUsageRecord {
                    id: Uuid::new_v4(),
                    tenant_id: console.admin.tenant.as_uuid(),
                    user_id: console.admin.user_id,
                    conversation_id: None,
                    provider: "golden".to_owned(),
                    model: "golden-model".to_owned(),
                    prompt_tokens: 100,
                    completion_tokens: 50,
                    total_tokens: 150,
                    call_type: "chat".to_owned(),
                    tool_calls_count: 0,
                    execution_time_ms: 100,
                    created_at: instant(&at),
                })
                .await
                .unwrap();
        }
        let body = console.read_ok("/api/dashboard/analytics?days=3").await;
        if today.holds() {
            break (today, body);
        }
    };

    let counts = [
        "/time_series/0/request_count",
        "/time_series/1/request_count",
        "/time_series/2/request_count",
    ];
    assert_eq!(
        with_values(dates_in(&body), &body, &counts),
        table(&[
            ("/time_series/0/date", shifted(today.utc, -2).as_str()),
            ("/time_series/0/request_count", "0"),
            ("/time_series/1/date", shifted(today.utc, -1).as_str()),
            ("/time_series/1/request_count", "2"),
            ("/time_series/2/date", shifted(today.utc, 0).as_str()),
            ("/time_series/2/request_count", "1"),
        ]),
        "{body}"
    );
}
