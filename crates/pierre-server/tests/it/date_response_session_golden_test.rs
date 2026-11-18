// ABOUTME: Response date goldens for what a session carries beside a body: the expired-token refusal, the budget header, quota periods
// ABOUTME: Pins the second in UTC a refusal names, X-RateLimit-Reset as epoch seconds, and the day and week keys quotas are counted under
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Response date goldens: the refusal of an expired session token, the
//! `X-RateLimit-Reset` header, and `/api/usage/status` (carnet#1620, gate 3).
//!
//! - A token past its expiry is refused with a sentence that names the
//!   expiry by hand, to the second, in UTC. The token's own `exp` claim says
//!   which second.
//! - Every REST answer carries the caller's request budget; its window is
//!   the calendar month in UTC and `X-RateLimit-Reset` is the first second
//!   of the next one, as seconds since the epoch.
//! - Quotas are counted under a period key written by hand: the UTC day for
//!   a daily counter, the Sunday that opens the week for a weekly one. The
//!   key is never answered; what it selects is. Rows are stored under the
//!   keys the suite computes and read back through the route, beside rows
//!   under the neighbouring keys, which must not be counted.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use axum::http::StatusCode;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use pierre_auth::auth::AuthManager;
use pierre_core::permissions::UserRole;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use serde_json::Value;
use std::time::Duration as Wait;
use tokio::time;

use crate::common;
use crate::helpers::axum_test::AxumTestRequest;
use crate::helpers::date_golden::{
    dates_in, dates_in_stamped, epoch, first_of_next_month, get, instant, on_the_clock,
    seed_account, send, shifted, sunday_of, table, Today,
};

#[tokio::test]
async fn an_expired_token_is_refused_naming_its_expiry_to_the_second_in_utc() {
    on_the_clock(|| async {
        let resources = common::create_test_server_resources().await.unwrap();
        let app = ProviderToolRouter::build_http_app(&resources);
        let account = seed_account(
            &resources,
            "golden-expired",
            UserRole::User,
            instant("2026-08-02T10:00:00"),
            instant("2026-08-02T10:00:00"),
        )
        .await;
        let user = resources
            .common
            .repos
            .users
            .get_global(account.user_id)
            .await
            .unwrap()
            .unwrap();
        // (hours a session lasts, how long ago the refusal says it ended): a
        // session that ended as it was issued, one that ended an hour before,
        // and one that ended two days before. The sentence counts minutes under
        // an hour, hours under a day, and days past it.
        for (lasts, ago) in [(0, "0 minutes"), (-1, "1 hours"), (-48, "2 days")] {
            let expired = AuthManager::new(lasts)
                .generate_token(&user, &resources.auth.jwks_manager)
                .unwrap();
            // A session of no length ends the second it is issued: wait it out.
            time::sleep(Wait::from_millis(1_100)).await;
            let claims: Value = serde_json::from_slice(
                &URL_SAFE_NO_PAD
                    .decode(expired.split('.').nth(1).unwrap())
                    .unwrap(),
            )
            .unwrap();
            let expiry = claims["exp"].as_i64().unwrap();

            for uri in [
                "/api/me/calendar?from=2026-08-24&to=2026-08-30",
                "/api/keys",
                "/api/user/mcp-tokens",
            ] {
                let (status, body) = send(app.clone(), get(uri, &expired)).await;
                assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri}: {body}");
                let message = body["message"].as_str().unwrap();
                let (said, at) = message.split_once(" ago at ").unwrap();
                assert_eq!(
                    said,
                    format!(
                        "Authentication failed: The provided authentication credentials are \
                     invalid: JWT validation failed: JWT token expired {ago}"
                    ),
                    "{uri}"
                );
                // `2026-11-17 18:47:43 UTC`: a space for the `T`, no fraction,
                // and the zone as a word. Read back, it is the claim's own
                // second.
                let stamp = at.strip_suffix(" UTC").unwrap();
                assert_eq!(stamp.len(), "2026-11-17 18:47:43".len(), "{uri}: {at}");
                assert_eq!(epoch(&stamp.replace(' ', "T")), expiry, "{uri}: {at}");
                assert_eq!(
                    dates_in_stamped(&body, &["/timestamp"]),
                    table(&[
                        // The claim's own second, read back above.
                        ("/message#0", at),
                        ("/timestamp", "dddd-dd-ddTdd:dd:dd.f+dd:dd"),
                    ]),
                    "{uri}: {body}"
                );
            }
        }
    })
    .await;
}

#[tokio::test]
async fn the_request_budget_resets_on_the_first_second_of_next_month_in_utc() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let account = seed_account(
        &resources,
        "golden-budget",
        UserRole::User,
        instant("2026-08-02T10:00:00"),
        instant("2026-08-02T10:00:00"),
    )
    .await;

    let (today, reset) = loop {
        let today = Today::read();
        let response = AxumTestRequest::get("/api/keys")
            .header("authorization", &format!("Bearer {}", account.token))
            .send(app.clone())
            .await;
        assert_eq!(response.status(), 200, "{}", response.body_text());
        let reset = response
            .header("x-ratelimit-reset")
            .expect("the answer carries the budget's reset")
            .to_owned();
        if today.holds() {
            break (today, reset);
        }
    };

    assert_eq!(
        reset,
        epoch(&format!(
            "{}T00:00:00",
            shifted(first_of_next_month(today.utc), 0)
        ))
        .to_string(),
        "whole seconds since the epoch, as digits"
    );
}

#[tokio::test]
async fn quotas_are_counted_under_the_utc_day_and_the_sunday_that_opens_the_week() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let (today, body) = loop {
        let today = Today::read();
        // A fresh account each time round: the counters only add, so a
        // midnight between two rounds would otherwise find this round's
        // "tomorrow" already counted under the new today.
        let account = seed_account(
            &resources,
            "golden-quota",
            UserRole::User,
            instant("2026-08-02T10:00:00"),
            instant("2026-08-02T10:00:00"),
        )
        .await;
        let (tenant, user) = (account.tenant.to_string(), account.user_id.to_string());
        let sunday = sunday_of(today.utc);
        // (counter, period key, amount). The first row of each counter is
        // the key of the current period; the others are the day before, the
        // day after and the day written as an instant for the daily counter,
        // and the Monday of this week and the Sundays before and after for
        // the weekly one.
        let stored = [
            ("daily_messages", shifted(today.utc, 0), 3),
            ("daily_messages", shifted(today.utc, -1), 100),
            ("daily_messages", shifted(today.utc, 1), 100),
            (
                "daily_messages",
                format!("{}T00:00:00Z", shifted(today.utc, 0)),
                100,
            ),
            ("weekly_messages", shifted(sunday, 0), 5),
            ("weekly_messages", shifted(sunday, 1), 100),
            ("weekly_messages", shifted(sunday, -7), 100),
            ("weekly_messages", shifted(sunday, 7), 100),
            ("weekly_tool_calls", shifted(sunday, 0), 7),
            ("daily_tool_calls", shifted(today.utc, 0), 2),
        ];
        for (counter, period, amount) in &stored {
            resources
                .common
                .repos
                .usage_counters
                .increment_counter(&tenant, &user, counter, period, *amount)
                .await
                .unwrap();
        }
        let (status, body) = send(app.clone(), get("/api/usage/status", &account.token)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if today.holds() {
            break (today, body);
        }
    };

    let counted: Vec<i64> = [
        "/daily/messages/current",
        "/daily/tool_calls/current",
        "/weekly/messages/current",
        "/weekly/tool_calls/current",
    ]
    .iter()
    .map(|pointer| body.pointer(pointer).and_then(Value::as_i64).unwrap())
    .collect();
    assert_eq!(counted, [3, 2, 5, 7], "{body}");

    // A daily counter resets at the next midnight UTC, a weekly one at the
    // midnight that opens next Sunday: whole seconds, with `Z`.
    let tomorrow = format!("{}T00:00:00Z", shifted(today.utc, 1));
    let next_sunday = format!("{}T00:00:00Z", shifted(sunday_of(today.utc), 7));
    assert_eq!(
        dates_in(&body),
        table(&[
            ("/daily/messages/resets_at", tomorrow.as_str()),
            ("/daily/tokens/resets_at", tomorrow.as_str()),
            ("/daily/tool_calls/resets_at", tomorrow.as_str()),
            ("/weekly/messages/resets_at", next_sunday.as_str()),
            ("/weekly/tokens/resets_at", next_sunday.as_str()),
            ("/weekly/tool_calls/resets_at", next_sunday.as_str()),
        ]),
        "{body}"
    );
}
