// ABOUTME: Response date goldens for more admin console routes: a user's usage, engagement tiers, pending users, tokens, flags
// ABOUTME: Pins the month a usage read opens on, the day keys it counts under, tier edges in days, and each stored instant
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Response date goldens: `/api/admin/users/{id}/usage`,
//! `/api/admin/engagement`, `/api/admin/pending-users`, `/api/admin/tokens`
//! and `/api/admin/users/{id}/features` (carnet#1620, gate 3).
//!
//! - A user's usage opens on the first instant of the current month in UTC
//!   and counts each call under its UTC day.
//! - Engagement buckets accounts by how long ago they were last active:
//!   within one day, seven days, thirty days, or dormant. Each account is
//!   last active ten minutes inside or outside an edge, measured from one
//!   reading of the clock.
//! - The pending queue answers the account's stored instants. An admin
//!   token an admin mints through `POST /api/admin/tokens` expires the days
//!   it was asked for after the request, held between two clock readings:
//!   the create answer writes that instant with `+00:00`, the list with `Z`.
//! - A flag set for a user is stamped with the clock, with an offset, and
//!   held between two clock readings.
//!
//! Every read goes through the server's whole HTTP app, as an admin's
//! console session.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use axum::http::StatusCode;
use axum::Router;
use pierre_core::models::UserStatus;
use pierre_core::permissions::UserRole;
use pierre_database::seed_models::SeedLlmUsageRecord;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::{json, Value};
use serial_test::serial;
use uuid::Uuid;

use crate::common;
use crate::helpers::date_golden::{
    as_stored, dates_in, dates_in_stamped, first_of_month, get, instant, lies_between,
    lies_days_after, minutes_before_now, on_the_clock, seed_account, send, shifted, table,
    with_json, with_values, Athlete, Instant, Today,
};

/// How long the admin token the suite issues is asked to last.
const TOKEN_DAYS: i64 = 30;

struct Console {
    resources: Arc<ServerContext>,
    app: Router,
    admin: Athlete,
}

impl Console {
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

    async fn read(&self, uri: &str) -> Value {
        let (status, body) = send(self.app.clone(), get(uri, &self.admin.token)).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        body
    }

    async fn athlete(&self, label: &str, last_active: &str) -> Athlete {
        seed_account(
            &self.resources,
            label,
            UserRole::User,
            instant("2026-08-02T10:00:00.250125"),
            instant(last_active),
        )
        .await
    }

    async fn llm_call(&self, account: &Athlete, at: &str) {
        self.resources
            .common
            .repos
            .seeder
            .seed_insert_llm_usage(&SeedLlmUsageRecord {
                id: Uuid::new_v4(),
                tenant_id: account.tenant.as_uuid(),
                user_id: account.user_id,
                conversation_id: None,
                provider: "golden".to_owned(),
                model: "golden-model".to_owned(),
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
                call_type: "chat".to_owned(),
                tool_calls_count: 0,
                execution_time_ms: 100,
                created_at: instant(at),
            })
            .await
            .unwrap();
    }
}

/// A user's usage opens on the first second of this month in UTC: a call
/// in the last half second of last month is not counted, and the calls of
/// this month fall under their UTC day.
#[tokio::test]
#[serial]
async fn a_users_usage_opens_on_the_first_of_the_month_in_utc_and_counts_by_utc_day() {
    let (today, body) = loop {
        let today = Today::read();
        // A fresh console each time round, so a midnight between two rounds
        // leaves no call behind.
        let console = Console::new("golden-admin-usage").await;
        let athlete = console
            .athlete("golden-admin-usage-athlete", "2026-08-29T10:15:30")
            .await;
        let first = first_of_month(today.utc);
        for at in [
            format!("{}T23:59:59.500250", shifted(first, -1)),
            format!("{}T00:00:00", shifted(first, 0)),
            format!("{}T00:00:00", shifted(today.utc, 0)),
        ] {
            console.llm_call(&athlete, &at).await;
        }
        let body = console
            .read(&format!("/api/admin/users/{}/usage", athlete.user_id))
            .await;
        if today.holds() {
            break (today, body);
        }
    };

    let first = shifted(first_of_month(today.utc), 0);
    let opened = format!("{first}T00:00:00+00:00");
    let mut expected = vec![
        ("/from", opened.as_str()),
        ("/daily/0/date", first.as_str()),
    ];
    let todays = shifted(today.utc, 0);
    if todays == first {
        // On the first of the month both calls fall on one day.
        expected.push(("/daily/0/calls", "2"));
    } else {
        expected.extend([
            ("/daily/0/calls", "1"),
            ("/daily/1/date", todays.as_str()),
            ("/daily/1/calls", "1"),
        ]);
    }
    let counts: Vec<&str> = expected
        .iter()
        .filter(|(pointer, _)| pointer.ends_with("/calls"))
        .map(|(pointer, _)| *pointer)
        .collect();
    assert_eq!(
        with_values(dates_in(&body), &body, &counts),
        table(&expected),
        "{body}"
    );
}

/// Accounts are daily within one day of their last activity, weekly within
/// seven, monthly within thirty, and dormant after: each edge is measured
/// from now, to the instant.
#[tokio::test]
#[serial]
async fn engagement_tiers_turn_a_whole_number_of_days_after_the_last_activity() {
    let console = Console::new("golden-admin-engagement").await;
    // (minutes before now): inside and outside each edge by ten minutes.
    for (label, minutes) in [
        ("inside-day", 24 * 60 - 10),
        ("outside-day", 24 * 60 + 10),
        ("inside-week", 7 * 24 * 60 - 10),
        ("outside-week", 7 * 24 * 60 + 10),
        ("inside-month", 30 * 24 * 60 - 10),
        ("outside-month", 30 * 24 * 60 + 10),
    ] {
        let at = minutes_before_now(minutes);
        seed_account(
            &console.resources,
            &format!("golden-engagement-{label}"),
            UserRole::User,
            instant("2026-08-02T10:00:00"),
            at,
        )
        .await;
    }
    let body = console.read("/api/admin/engagement").await;
    // The admin signed in, so is active now: daily too.
    assert_eq!(
        with_values(
            dates_in(&body),
            &body,
            &[
                "/users/total",
                "/users/daily",
                "/users/weekly",
                "/users/monthly",
                "/users/dormant"
            ]
        ),
        table(&[
            ("/users/daily", "2"),
            ("/users/dormant", "1"),
            ("/users/monthly", "6"),
            ("/users/total", "7"),
            ("/users/weekly", "4"),
        ]),
        "{body}"
    );
}

#[tokio::test]
async fn a_pending_account_answers_its_stored_instants_with_an_offset() {
    let console = Console::new("golden-admin-pending").await;
    let pending = console
        .athlete("golden-admin-pending-athlete", "2026-08-29T10:15:30")
        .await;
    console
        .resources
        .common
        .repos
        .users
        .update_status(pending.user_id, UserStatus::Pending, None)
        .await
        .unwrap();
    let body = console.read("/api/admin/pending-users").await;
    let id = pending.user_id.to_string();
    let row = body["data"]["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id.as_str())
        .unwrap_or_else(|| panic!("the account waits: {body}"));
    assert_eq!(
        dates_in(row),
        table(&[
            ("/created_at", "2026-08-02T10:00:00.250125+00:00"),
            ("/last_active", "2026-08-29T10:15:30+00:00"),
        ]),
        "{body}"
    );
}

#[tokio::test]
async fn an_admin_token_is_stamped_now_and_expires_the_days_asked_for_after() {
    on_the_clock(|| async {
        let console = Console::new("golden-admin-tokens").await;
        let before = Instant::now();
        let (status, created) = send(
            console.app.clone(),
            with_json(
                "POST",
                "/api/admin/tokens",
                &console.admin.token,
                &json!({
                    "service_name": format!("golden-{}", Uuid::new_v4()),
                    "expires_in_days": TOKEN_DAYS,
                }),
            ),
        )
        .await;
        let after = Instant::now();
        assert_eq!(status, StatusCode::CREATED, "{created}");

        // The create answer writes the expiry with `+00:00`.
        assert_eq!(
            dates_in_stamped(&created["data"], &["/expires_at"]),
            table(&[("/expires_at", "dddd-dd-ddTdd:dd:dd.f+dd:dd")]),
            "{created}"
        );
        let answered = created["data"]["expires_at"].as_str().unwrap();
        assert!(
            lies_days_after(answered, before, after, TOKEN_DAYS),
            "{created}"
        );

        // The list writes the same instants with `Z`.
        let body = console.read("/api/admin/tokens").await;
        let tokens = &body["data"]["tokens"];
        assert_eq!(
            dates_in_stamped(tokens, &["/0/created_at", "/0/expires_at"]),
            table(&[
                ("/0/created_at", "dddd-dd-ddTdd:dd:dd.fZ"),
                ("/0/expires_at", "dddd-dd-ddTdd:dd:dd.fZ"),
            ]),
            "{body}"
        );
        assert_eq!(tokens[0]["id"], created["data"]["token_id"], "{body}");
        assert_eq!(
            tokens[0]["expires_at"],
            as_stored(
                &console.resources.agent.database,
                &answered.replace("+00:00", "Z")
            ),
            "{body}"
        );
        assert!(
            lies_between(tokens[0]["created_at"].as_str().unwrap(), before, after),
            "{body}"
        );
    })
    .await;
}

#[tokio::test]
async fn a_flag_set_for_a_user_is_stamped_now() {
    on_the_clock(|| async {
        let console = Console::new("golden-admin-flags").await;
        let athlete = console
            .athlete("golden-admin-flags-athlete", "2026-08-29T10:15:30")
            .await;
        let uri = format!("/api/admin/users/{}/features/api_tokens", athlete.user_id);
        let before = Instant::now();
        let (status, body) = send(
            console.app.clone(),
            with_json(
                "PUT",
                &uri,
                &console.admin.token,
                &json!({ "enabled": true }),
            ),
        )
        .await;
        let after = Instant::now();
        assert!(status.is_success(), "{status} {body}");
        let listed = console
            .read(&format!("/api/admin/users/{}/features", athlete.user_id))
            .await;
        assert_eq!(
            dates_in_stamped(&listed["rows"], &["/0/updated_at"]),
            table(&[("/0/updated_at", "dddd-dd-ddTdd:dd:dd.f+dd:dd")]),
            "{listed}"
        );
        assert!(
            lies_between(
                listed["rows"][0]["updated_at"].as_str().unwrap(),
                before,
                after
            ),
            "stamped while the request ran: {listed}"
        );
    })
    .await;
}
