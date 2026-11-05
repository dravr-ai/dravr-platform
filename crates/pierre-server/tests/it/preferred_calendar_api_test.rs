// ABOUTME: HTTP tests for GET/PUT/DELETE /api/me/preferred-calendar — the calendar Dravr's pushes go to (carnet#1393)
// ABOUTME: Only the athlete's own connected calendars are offered and recorded; a disconnected choice reads as such

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::{json, Value};

use common::{create_test_server_resources, create_test_user, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::{ConnectionType, TenantId};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::calendar_preference::CalendarPreferenceRoutes;
use pierre_services::plan_calendar_push::connected_calendars;
use uuid::Uuid;

const ROUTE: &str = "/api/me/preferred-calendar";

struct Setup {
    resources: Arc<ServerContext>,
    router: axum::Router,
    token: String,
    user_id: Uuid,
    tenant: TenantId,
    /// Two calendar providers this build compiles, as the registry ranks
    /// them for an athlete holding both.
    calendars: [String; 2],
}

async fn setup() -> Setup {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let token = format!("Bearer {}", generate_test_token(&resources, &user).await);
    let tenant = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("tenants")
        .first()
        .expect("user has a tenant")
        .id;
    let registry = resources.fitness.provider_registry.as_ref();
    let writers = registry.calendar_write_providers();
    assert!(
        writers.len() >= 2,
        "two calendar providers compiled: {writers:?}"
    );
    let ranked = connected_calendars(registry, &writers[..2]);
    let calendars = [ranked[0].to_owned(), ranked[1].to_owned()];
    let router = CalendarPreferenceRoutes::routes(Arc::clone(&resources));
    Setup {
        resources,
        router,
        token,
        user_id,
        tenant,
        calendars,
    }
}

impl Setup {
    async fn connect(&self, provider: &str, connection_type: ConnectionType) {
        self.resources
            .common
            .repos
            .provider_connections
            .register_connection(self.user_id, self.tenant, provider, &connection_type, None)
            .await
            .expect("register connection");
    }

    async fn read(&self) -> Value {
        AxumTestRequest::get(ROUTE)
            .header("Authorization", &self.token)
            .send(self.router.clone())
            .await
            .assert_status(StatusCode::OK)
            .json::<Value>()
    }

    async fn put(&self, provider: &str) -> (u16, Value) {
        let response = AxumTestRequest::put(ROUTE)
            .header("Authorization", &self.token)
            .json(&json!({ "provider": provider }))
            .send(self.router.clone())
            .await;
        let status = response.status();
        (status, response.json::<Value>())
    }

    fn option(&self, backend: &str) -> Value {
        let name = self
            .resources
            .fitness
            .provider_registry
            .get_display_name(backend)
            .unwrap_or(backend)
            .to_owned();
        json!({ "provider": backend, "name": name })
    }
}

#[tokio::test]
async fn an_athlete_with_no_calendar_reads_nothing_to_choose() {
    let setup = setup().await;
    assert_eq!(
        setup.read().await,
        json!({ "chosen": null, "chosen_connected": false, "calendars": [] })
    );
}

/// Only the athlete's own calendars are offered: a provider that writes no
/// calendar is not one, and a calendar read through a human coach's link
/// takes that coach's writes, never theirs.
#[tokio::test]
async fn only_own_connected_calendars_are_offered_and_recorded() {
    let setup = setup().await;
    let [first, second] = setup.calendars.clone();
    setup.connect("strava", ConnectionType::OAuth).await;
    setup.connect(&second, ConnectionType::OAuth).await;
    setup.connect(&first, ConnectionType::OAuth).await;

    assert_eq!(
        setup.read().await,
        json!({
            "chosen": null,
            "chosen_connected": false,
            "calendars": [setup.option(&first), setup.option(&second)],
        })
    );

    let (status, answered) = setup.put(&second).await;
    assert_eq!(status, StatusCode::OK.as_u16(), "{answered}");
    assert_eq!(answered["chosen"], setup.option(&second));
    assert_eq!(answered["chosen_connected"], true);
    assert_eq!(
        setup.read().await,
        answered,
        "the write answers as the read does"
    );
    let stored = setup
        .resources
        .common
        .repos
        .calendar_preferences
        .get_calendar_preference(setup.tenant, setup.user_id)
        .await
        .expect("read")
        .expect("recorded");
    assert_eq!(stored.provider, second);
    assert_eq!(
        stored.set_by,
        Some(setup.user_id),
        "the athlete recorded it"
    );

    let (status, refused) = setup.put("strava").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST.as_u16(),
        "not a calendar: {refused}"
    );
    assert_eq!(
        setup.read().await["chosen"],
        setup.option(&second),
        "a refused write changes nothing"
    );
}

#[tokio::test]
async fn a_calendar_held_only_through_a_link_or_not_at_all_is_refused() {
    let setup = setup().await;
    let [first, second] = setup.calendars.clone();
    setup.connect(&first, ConnectionType::OAuth).await;
    setup.connect(&second, ConnectionType::Delegated).await;

    assert_eq!(
        setup.read().await["calendars"],
        json!([setup.option(&first)]),
        "a link's calendar is the coach's to write"
    );
    let (status, refused) = setup.put(&second).await;
    assert_eq!(status, StatusCode::BAD_REQUEST.as_u16(), "{refused}");
    let none = setup
        .resources
        .common
        .repos
        .calendar_preferences
        .get_calendar_preference(setup.tenant, setup.user_id)
        .await
        .expect("read");
    assert_eq!(none, None);
}

#[tokio::test]
async fn delete_forgets_the_choice_and_a_disconnected_choice_reads_as_such() {
    let setup = setup().await;
    let [first, second] = setup.calendars.clone();
    setup.connect(&first, ConnectionType::OAuth).await;
    setup.connect(&second, ConnectionType::OAuth).await;

    let (status, _) = setup.put(&first).await;
    assert_eq!(status, StatusCode::OK.as_u16());
    setup
        .resources
        .common
        .repos
        .provider_connections
        .remove_connection(setup.user_id, setup.tenant, &first)
        .await
        .expect("disconnect");
    assert_eq!(
        setup.read().await,
        json!({
            "chosen": setup.option(&first),
            "chosen_connected": false,
            "calendars": [setup.option(&second)],
        }),
        "the choice stays, said to be disconnected"
    );

    let cleared = AxumTestRequest::delete(ROUTE)
        .header("Authorization", &setup.token)
        .send(setup.router.clone())
        .await
        .assert_status(StatusCode::OK)
        .json::<Value>();
    assert_eq!(
        cleared,
        json!({
            "chosen": null,
            "chosen_connected": false,
            "calendars": [setup.option(&second)],
        })
    );
}

#[tokio::test]
async fn the_route_requires_a_session() {
    let setup = setup().await;
    let status = AxumTestRequest::get(ROUTE)
        .send(setup.router.clone())
        .await
        .status();
    assert_eq!(status, StatusCode::UNAUTHORIZED.as_u16());
}
