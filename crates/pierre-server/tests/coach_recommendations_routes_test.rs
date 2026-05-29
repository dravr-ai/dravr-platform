// ABOUTME: Integration tests for GET /api/coaches?personalize=true end-to-end
// ABOUTME: Verifies match_score/recommended wiring, serialization, and cold-start behavior
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use common::{create_test_server_resources, create_test_user, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_coaches::build_coaches_router;
use pierre_routes_coaches::coaches::ListCoachesResponse;

use axum::http::StatusCode;
use serde_json::json;

/// Build the coaches router for a fresh user with no connected provider
/// (the cold-start case for personalization).
async fn setup() -> (axum::Router, String) {
    let resources = create_test_server_resources().await.unwrap();
    let (_user_id, user) = create_test_user(&resources.coach.database).await.unwrap();
    let token = generate_test_token(&resources, &user).await;
    let router = build_coaches_router::<ServerContext>().with_state(resources);
    (router, format!("Bearer {token}"))
}

/// Create a coach via the API so the list has something to score.
async fn create_coach(router: &axum::Router, auth: &str, title: &str) {
    let response = AxumTestRequest::post("/api/coaches")
        .header("authorization", auth)
        .json(&json!({
            "title": title,
            "system_prompt": "You are a fitness coach.",
        }))
        .send(router.clone())
        .await;
    assert_eq!(response.status_code(), StatusCode::CREATED);
}

/// With `personalize=true`, every coach is tagged with `match_score` and
/// `recommended`. For a fresh user (no provider, no activities) an
/// empty-prerequisite coach is a provider-free starter, so it is recommended
/// at the sport-agnostic base score.
#[tokio::test]
async fn personalize_true_tags_coaches_with_score_and_recommended() {
    let (router, auth) = setup().await;
    create_coach(&router, &auth, "Personalize Coach").await;

    let response = AxumTestRequest::get("/api/coaches?personalize=true")
        .header("authorization", &auth)
        .send(router)
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);

    let list: ListCoachesResponse = response.json();
    let coach = list
        .coaches
        .iter()
        .find(|c| c.title == "Personalize Coach")
        .expect("created coach should be listed");

    assert_eq!(
        coach.recommended,
        Some(true),
        "empty-prereq coach is a cold-start starter"
    );
    let score = coach.match_score.expect("personalize sets match_score");
    // Sport-agnostic base score (COACH_REC_SPORT_AGNOSTIC_BASE_SCORE default).
    assert!(
        (score - 0.5).abs() < f32::EPSILON,
        "expected starter base score 0.5, got {score}"
    );
}

/// Without `personalize`, the recommendation fields are omitted entirely
/// (serde skips `None`), so the default coach list is unchanged.
#[tokio::test]
async fn personalize_absent_omits_recommendation_fields() {
    let (router, auth) = setup().await;
    create_coach(&router, &auth, "Plain Coach").await;

    let response = AxumTestRequest::get("/api/coaches")
        .header("authorization", &auth)
        .send(router)
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);

    let list: ListCoachesResponse = response.json();
    let coach = list
        .coaches
        .iter()
        .find(|c| c.title == "Plain Coach")
        .expect("created coach should be listed");

    assert!(coach.match_score.is_none(), "no score without personalize");
    assert!(coach.recommended.is_none(), "no flag without personalize");
}
