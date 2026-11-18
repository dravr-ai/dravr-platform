// ABOUTME: Response date goldens for groups, agents, the session and the quota page, through the whole HTTP app
// ABOUTME: Pins UTC instants written with an offset on group, member and agent rows, and when each quota counter resets
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Response date goldens: `/api/groups/{id}`, its members, `POST
//! /api/groups`, `/api/agents`, `/api/agents/{id}`, `/api/auth/session`,
//! `/a2a/clients` and `/api/users/me/quota` (carnet#1620, gate 3).
//!
//! Group, membership and agent rows are stamped with the wall clock by their
//! repositories, so each is written through the repository and then moved to
//! a chosen instant with one `UPDATE`, and the answer compared as a literal.
//! All of them are written by hand with an offset (`+00:00`), never `Z`.
//!
//! Stamped with the wall clock, and so compared by shape: a list's
//! `metadata.timestamp`, and the group a `POST` creates.

#![cfg(feature = "provider-strava")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use axum::http::StatusCode;
use axum::Router;
use pierre_a2a::ClientRegistrationRequest;
use pierre_core::permissions::UserRole;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::common;
use crate::helpers::coach_roster_fixtures::{
    coach, coached_group, group_agent, join_at, person, Person,
};
use crate::helpers::date_golden::{
    dates_in, dates_in_stamped, get, instant, on_the_clock, restamp, seed_account, send, shifted,
    sunday_of, table, with_json, Today,
};

const MOVE_GROUP: &str = "UPDATE coaching_groups SET created_at = $1 \
                          WHERE CAST(id AS TEXT) = $2 AND CAST(tenant_id AS TEXT) = $3";
const TOUCH_GROUP: &str = "UPDATE coaching_groups SET updated_at = $1 \
                           WHERE CAST(id AS TEXT) = $2 AND CAST(tenant_id AS TEXT) = $3";
const CONSENT: &str = "UPDATE coaching_group_members SET consent_given_at = $1 \
                       WHERE CAST(user_id AS TEXT) = $2 AND CAST(tenant_id AS TEXT) = $3";
const MOVE_AGENT: &str = "UPDATE agents SET created_at = $1 \
                          WHERE CAST(id AS TEXT) = $2 AND CAST(tenant_id AS TEXT) = $3";
const TOUCH_AGENT: &str = "UPDATE agents SET updated_at = $1 \
                           WHERE CAST(id AS TEXT) = $2 AND CAST(tenant_id AS TEXT) = $3";

struct Club {
    app: Router,
    coach: Person,
    athlete: Person,
    agent: String,
    group: Uuid,
}

impl Club {
    /// A coach, their agent and their group, with the coach and one athlete
    /// as members, every row moved to an instant of August.
    async fn new() -> Self {
        let res = common::create_test_server_resources().await.unwrap();
        let app = ProviderToolRouter::build_http_app(&res);
        let tag = Uuid::new_v4();
        let coach = coach(&res, &format!("coach-{tag}@example.com"), "Cole Coach").await;
        let athlete = person(&res, &format!("ada-{tag}@example.com"), "Ada").await;
        let agent = group_agent(&res, &coach).await;
        let group = coached_group(&res, coach.tenant, coach.id, &agent, "Club").await;
        join_at(
            &res,
            group,
            coach.id,
            coach.tenant,
            true,
            instant("2026-08-01T10:00:00"),
        )
        .await;
        join_at(
            &res,
            group,
            athlete.id,
            coach.tenant,
            true,
            instant("2026-08-02T10:00:00.250125"),
        )
        .await;
        let database = &res.agent.database;
        for (sql, at, id) in [
            (MOVE_GROUP, "2026-07-31T22:00:00", group.to_string()),
            (TOUCH_GROUP, "2026-08-28T02:59:03.500250", group.to_string()),
            (CONSENT, "2026-08-01T10:00:01.250125", coach.id.to_string()),
            (CONSENT, "2026-08-02T10:00:01", athlete.id.to_string()),
            (MOVE_AGENT, "2026-07-30T08:15:00.250125", agent.clone()),
            (TOUCH_AGENT, "2026-08-29T10:15:30", agent.clone()),
        ] {
            restamp(database, sql, instant(at), &id, Some(coach.tenant)).await;
        }
        Self {
            app,
            coach,
            athlete,
            agent,
            group,
        }
    }

    async fn read(&self, who: &Person, uri: &str) -> Value {
        let token = who.auth.trim_start_matches("Bearer ");
        let (status, body) = send(self.app.clone(), get(uri, token)).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        body
    }
}

#[tokio::test]
async fn a_group_and_its_members_answer_their_instants_in_utc_with_an_offset() {
    on_the_clock(|| async {
        let club = Club::new().await;

        let group = club
            .read(&club.coach, &format!("/api/groups/{}", club.group))
            .await;
        assert_eq!(
            dates_in(&group),
            table(&[
                ("/created_at", "2026-07-31T22:00:00+00:00"),
                ("/updated_at", "2026-08-28T02:59:03.500250+00:00"),
            ]),
            "{group}"
        );

        let members = club
            .read(&club.coach, &format!("/api/groups/{}/members", club.group))
            .await;
        let member = |who: &Person| {
            members["members"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["user_id"] == who.id.to_string())
                .unwrap_or_else(|| panic!("{} is a member: {members}", who.id))
        };
        assert_eq!(
            dates_in(member(&club.coach)),
            table(&[
                ("/consent_given_at", "2026-08-01T10:00:01.250125+00:00"),
                ("/joined_at", "2026-08-01T10:00:00+00:00"),
            ]),
            "{members}"
        );
        assert_eq!(
            dates_in(member(&club.athlete)),
            table(&[
                ("/consent_given_at", "2026-08-02T10:00:01+00:00"),
                ("/joined_at", "2026-08-02T10:00:00.250125+00:00"),
            ]),
            "{members}"
        );
        assert_eq!(
            dates_in_stamped(&members["metadata"], &["/timestamp"]),
            table(&[("/timestamp", "dddd-dd-ddTdd:dd:dd.f+dd:dd")]),
            "{members}"
        );
    })
    .await;
}

#[tokio::test]
async fn a_new_group_is_stamped_in_utc_with_an_offset() {
    on_the_clock(|| async {
        let club = Club::new().await;
        let token = club.coach.auth.trim_start_matches("Bearer ");

        let (status, created) = send(
            club.app.clone(),
            with_json(
                "POST",
                "/api/groups",
                token,
                &json!({ "name": "Second club", "agent_id": club.agent }),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::CREATED, "{created}");
        assert_eq!(
            dates_in_stamped(&created, &["/created_at", "/updated_at"]),
            table(&[
                ("/created_at", "dddd-dd-ddTdd:dd:dd.f+dd:dd"),
                ("/updated_at", "dddd-dd-ddTdd:dd:dd.f+dd:dd"),
            ]),
            "{created}"
        );
        assert_eq!(created["created_at"], created["updated_at"], "{created}");
    })
    .await;
}

#[tokio::test]
async fn an_agent_answers_its_instants_in_utc_with_an_offset() {
    on_the_clock(|| async {
        let club = Club::new().await;

        let one = club
            .read(&club.coach, &format!("/api/agents/{}", club.agent))
            .await;
        assert_eq!(
            dates_in(&one),
            table(&[
                ("/created_at", "2026-07-30T08:15:00.250125+00:00"),
                ("/updated_at", "2026-08-29T10:15:30+00:00"),
            ]),
            "{one}"
        );

        let listed = club.read(&club.coach, "/api/agents").await;
        assert_eq!(
            dates_in_stamped(&listed, &["/metadata/timestamp"]),
            table(&[
                ("/agents/0/created_at", "2026-07-30T08:15:00.250125+00:00"),
                ("/agents/0/updated_at", "2026-08-29T10:15:30+00:00"),
                ("/metadata/timestamp", "dddd-dd-ddTdd:dd:dd.f+dd:dd"),
            ]),
            "{listed}"
        );
    })
    .await;
}

#[tokio::test]
async fn the_session_and_a_registered_a2a_client_answer_their_accounts_instants() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let account = seed_account(
        &resources,
        "golden-session",
        UserRole::User,
        instant("2026-08-02T10:00:00.250125"),
        instant("2026-08-29T10:15:30"),
    )
    .await;

    let (status, session) = send(app.clone(), get("/api/auth/session", &account.token)).await;
    assert_eq!(status, StatusCode::OK, "{session}");
    assert_eq!(
        dates_in(&session),
        table(&[("/user/created_at", "2026-08-02T10:00:00.250125+00:00")]),
        "{session}"
    );

    let client_id = resources
        .a2a
        .a2a_client_manager
        .register_client(
            ClientRegistrationRequest {
                name: format!("golden-{}", account.user_id),
                description: "date golden client".to_owned(),
                capabilities: vec!["fitness-data-analysis".to_owned()],
                redirect_uris: Vec::new(),
                contact_email: "golden@example.com".to_owned(),
            },
            account.user_id,
        )
        .await
        .unwrap()
        .client_id;
    for (sql, at) in [
        (
            "UPDATE a2a_clients SET created_at = $1 WHERE client_id = $2",
            "2026-08-28T02:59:03.500250",
        ),
        (
            "UPDATE a2a_clients SET updated_at = $1 WHERE client_id = $2",
            "2026-08-29T10:15:30",
        ),
    ] {
        restamp(
            &resources.agent.database,
            sql,
            instant(at),
            &client_id,
            None,
        )
        .await;
    }
    let (status, clients) = send(app, get("/a2a/clients", &account.token)).await;
    assert_eq!(status, StatusCode::OK, "{clients}");
    assert_eq!(
        dates_in(&clients),
        table(&[
            ("/0/created_at", "2026-08-28T02:59:03.500250+00:00"),
            ("/0/updated_at", "2026-08-29T10:15:30+00:00"),
        ]),
        "{clients}"
    );
}

#[tokio::test]
async fn the_quota_page_says_when_each_counter_resets_in_utc() {
    let resources = common::create_test_server_resources().await.unwrap();
    let app = ProviderToolRouter::build_http_app(&resources);
    let account = seed_account(
        &resources,
        "golden-quota-page",
        UserRole::User,
        instant("2026-08-02T10:00:00"),
        instant("2026-08-02T10:00:00"),
    )
    .await;

    let (today, body) = loop {
        let today = Today::read();
        let (status, body) = send(app.clone(), get("/api/users/me/quota", &account.token)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        if today.holds() {
            break (today, body);
        }
    };

    // A daily counter resets at the next midnight UTC and a weekly one at
    // the midnight that opens next Sunday; a count of something held, not
    // spent, is answered with tomorrow's midnight too.
    let tomorrow = format!("{}T00:00:00Z", shifted(today.utc, 1));
    let next_sunday = format!("{}T00:00:00Z", shifted(sunday_of(today.utc), 7));
    let resets: Vec<(String, Option<String>)> = body["counters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|counter| {
            (
                counter["counter_type"].as_str().unwrap().to_owned(),
                counter["resets_at"].as_str().map(str::to_owned),
            )
        })
        .collect();
    let expected: Vec<(String, Option<String>)> = [
        ("daily_messages", Some(&tomorrow)),
        ("daily_tokens", Some(&tomorrow)),
        ("weekly_tokens", Some(&next_sunday)),
        ("daily_tool_calls", Some(&tomorrow)),
        ("daily_conversations", Some(&tomorrow)),
        ("active_coaches", Some(&tomorrow)),
    ]
    .into_iter()
    .map(|(counter, at)| (counter.to_owned(), at.cloned()))
    .collect();
    assert_eq!(resets, expected, "{body}");
}
