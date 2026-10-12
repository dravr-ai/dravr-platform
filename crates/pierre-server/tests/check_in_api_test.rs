// ABOUTME: HTTP tests for GET/PUT /api/me/check-in — the check-in cadence asked in onboarding and changed in Settings
// ABOUTME: Null until chosen, every cadence (off included) stored and read back, the onboarding step recorded (carnet#831)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::{json, Value};

use common::{create_test_server_resources, create_test_user, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::CheckInCadence;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::check_in::CheckInRoutes;
use pierre_mcp_server::routes::onboarding::OnboardingRoutes;
use uuid::Uuid;

struct Setup {
    resources: Arc<ServerContext>,
    router: axum::Router,
    token: String,
    user_id: Uuid,
}

async fn setup() -> Setup {
    let resources = create_test_server_resources()
        .await
        .expect("server resources");
    let (user_id, user) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let token = format!("Bearer {}", generate_test_token(&resources, &user).await);
    let router = CheckInRoutes::routes(Arc::clone(&resources))
        .merge(OnboardingRoutes::routes(Arc::clone(&resources)));
    Setup {
        resources,
        router,
        token,
        user_id,
    }
}

async fn read_cadence(setup: &Setup) -> Value {
    AxumTestRequest::get("/api/me/check-in")
        .header("Authorization", &setup.token)
        .send(setup.router.clone())
        .await
        .assert_status(StatusCode::OK)
        .json::<Value>()
}

/// An athlete who never chose reads `null` — never checked in on — and every
/// cadence, `off` among them, is stored and read back as chosen.
#[tokio::test]
async fn the_cadence_reads_null_until_chosen_then_what_was_chosen() {
    let setup = setup().await;
    assert_eq!(read_cadence(&setup).await, json!({ "cadence": null }));

    for cadence in ["daily", "three_per_week", "weekly", "off"] {
        let answered: Value = AxumTestRequest::put("/api/me/check-in")
            .header("Authorization", &setup.token)
            .json(&json!({ "cadence": cadence }))
            .send(setup.router.clone())
            .await
            .assert_status(StatusCode::OK)
            .json();
        assert_eq!(answered, json!({ "cadence": cadence }), "PUT echoes it");
        assert_eq!(
            read_cadence(&setup).await,
            json!({ "cadence": cadence }),
            "GET reads back what was chosen"
        );
    }

    // Stored under the athlete's active tenant, where the scheduler lists it.
    let tenant = setup
        .resources
        .common
        .repos
        .tenants
        .list_for_user(setup.user_id)
        .await
        .unwrap()
        .first()
        .expect("the test user owns a tenant")
        .id;
    assert_eq!(
        setup
            .resources
            .common
            .repos
            .check_in_preferences
            .check_in_cadence(tenant, setup.user_id)
            .await
            .unwrap(),
        Some(CheckInCadence::Off)
    );
}

/// Only the four cadences are answers: anything else is refused and leaves
/// the athlete's choice as it was.
#[tokio::test]
async fn an_unknown_cadence_is_refused_and_nothing_is_stored() {
    let setup = setup().await;
    for body in [
        json!({ "cadence": "monthly" }),
        json!({ "cadence": null }),
        json!({}),
    ] {
        let status = AxumTestRequest::put("/api/me/check-in")
            .header("Authorization", &setup.token)
            .json(&body)
            .send(setup.router.clone())
            .await
            .status_code();
        assert!(status.is_client_error(), "{body} must be refused: {status}");
    }
    assert_eq!(read_cadence(&setup).await, json!({ "cadence": null }));
}

/// The onboarding step the web and the phone ask the cadence in is a step the
/// server records, so every surface steps past it once it is answered.
#[tokio::test]
async fn the_check_in_onboarding_step_is_recorded() {
    let setup = setup().await;
    AxumTestRequest::put("/api/me/onboarding/steps/check_in")
        .header("Authorization", &setup.token)
        .json(&json!({ "status": "complete" }))
        .send(setup.router.clone())
        .await
        .assert_status(StatusCode::NO_CONTENT);

    let status: Value = AxumTestRequest::get("/api/me/onboarding-status")
        .header("Authorization", &setup.token)
        .send(setup.router.clone())
        .await
        .assert_status(StatusCode::OK)
        .json();
    let steps = status["steps"].as_array().expect("the recorded steps");
    assert!(
        steps
            .iter()
            .any(|step| step["step_id"] == "check_in" && step["status"] == "complete"),
        "the check-in step reads back: {steps:?}"
    );
}

/// The endpoint is the athlete's own: no session, no answer.
#[tokio::test]
async fn the_cadence_needs_a_session() {
    let setup = setup().await;
    let status = AxumTestRequest::get("/api/me/check-in")
        .send(setup.router.clone())
        .await
        .status_code();
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
