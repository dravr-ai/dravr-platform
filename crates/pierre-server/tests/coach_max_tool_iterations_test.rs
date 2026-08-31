// ABOUTME: Integration tests for the per-coach and tenant-wide tool-loop budget
// ABOUTME: Covers the REST keep/clear/set round-trip, range rejection, and the admin config parameter
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::collections::HashMap;
use std::sync::Arc;

use axum::http::StatusCode;
use serde_json::json;

use common::{create_test_server_resources, create_test_user, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_config::admin_types::{ConfigDataType, ValidateConfigRequest};
use pierre_core::constants::tool_execution::{
    DEFAULT_MAX_TOOL_ITERATIONS, MAX_MAX_TOOL_ITERATIONS, MIN_MAX_TOOL_ITERATIONS,
};
use pierre_core::models::TenantId;
use pierre_database::database::test_utils::create_test_db;
use pierre_mcp_server::config::admin::service::AdminConfigService;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_coaches::build_coaches_router;
use pierre_routes_coaches::coaches::CoachResponse;
use pierre_runtime_context::ConfigLookupScope;

/// Build a coaches router plus the bearer header and the context the runtime
/// read path needs, so a test can assert what the chat turn would load.
async fn setup() -> (axum::Router, String, Arc<ServerContext>, TenantId) {
    let resources = create_test_server_resources().await.unwrap();
    let (_user_id, user) = create_test_user(&resources.coach.database).await.unwrap();
    let token = generate_test_token(&resources, &user).await;

    let tenant_id = resources
        .coach
        .database
        .repositories()
        .tenants
        .list_for_user(user.id)
        .await
        .unwrap()
        .first()
        .map(|t| t.id)
        .expect("test user belongs to a tenant");

    let router = build_coaches_router::<ServerContext>().with_state(Arc::clone(&resources));
    (router, format!("Bearer {token}"), resources, tenant_id)
}

// ============================================================================
// Coach column round-trip
// ============================================================================

#[tokio::test]
async fn create_coach_persists_max_tool_iterations_to_the_turn_path() {
    let (router, auth, resources, tenant_id) = setup().await;

    let created: CoachResponse = AxumTestRequest::post("/api/coaches")
        .header("authorization", &auth)
        .json(&json!({
            "title": "Deep Analysis Coach",
            "system_prompt": "You dig through many activities before answering.",
            "max_tool_iterations": 27
        }))
        .send(router.clone())
        .await
        .json();

    assert_eq!(created.max_tool_iterations, Some(27));

    let fetched: CoachResponse = AxumTestRequest::get(&format!("/api/coaches/{}", created.id))
        .header("authorization", &auth)
        .send(router)
        .await
        .json();
    assert_eq!(fetched.max_tool_iterations, Some(27));

    // The column the chat turn actually reads carries the same value.
    let runtime = resources
        .coach
        .database
        .repositories()
        .coaches
        .get_coach_runtime_context(&created.id, tenant_id)
        .await
        .unwrap()
        .expect("coach runtime context");
    assert_eq!(runtime.max_tool_iterations, Some(27));
}

#[tokio::test]
async fn create_coach_without_a_budget_leaves_the_coach_inheriting() {
    let (router, auth, resources, tenant_id) = setup().await;

    let created: CoachResponse = AxumTestRequest::post("/api/coaches")
        .header("authorization", &auth)
        .json(&json!({
            "title": "Inheriting Coach",
            "system_prompt": "You use whatever budget the tenant sets."
        }))
        .send(router)
        .await
        .json();

    assert_eq!(created.max_tool_iterations, None);

    let runtime = resources
        .coach
        .database
        .repositories()
        .coaches
        .get_coach_runtime_context(&created.id, tenant_id)
        .await
        .unwrap()
        .expect("coach runtime context");
    assert_eq!(runtime.max_tool_iterations, None);
}

#[tokio::test]
async fn update_coach_writes_a_new_budget_and_keeps_it_when_omitted() {
    let (router, auth, _resources, _tenant_id) = setup().await;

    let created: CoachResponse = AxumTestRequest::post("/api/coaches")
        .header("authorization", &auth)
        .json(&json!({
            "title": "Adjustable Coach",
            "system_prompt": "Original prompt",
            "max_tool_iterations": 12
        }))
        .send(router.clone())
        .await
        .json();
    assert_eq!(created.max_tool_iterations, Some(12));

    let updated: CoachResponse = AxumTestRequest::put(&format!("/api/coaches/{}", created.id))
        .header("authorization", &auth)
        .json(&json!({ "max_tool_iterations": 7 }))
        .send(router.clone())
        .await
        .json();
    assert_eq!(updated.max_tool_iterations, Some(7));

    // An update that says nothing about the budget leaves the stored value.
    let renamed: CoachResponse = AxumTestRequest::put(&format!("/api/coaches/{}", created.id))
        .header("authorization", &auth)
        .json(&json!({ "title": "Renamed Coach" }))
        .send(router)
        .await
        .json();
    assert_eq!(renamed.title, "Renamed Coach");
    assert_eq!(renamed.max_tool_iterations, Some(7));
}

#[tokio::test]
async fn update_coach_omitting_the_budget_keeps_the_stored_one_on_the_turn_path() {
    let (router, auth, resources, tenant_id) = setup().await;

    let created: CoachResponse = AxumTestRequest::post("/api/coaches")
        .header("authorization", &auth)
        .json(&json!({
            "title": "Pinned Coach",
            "system_prompt": "You get twenty-three rounds.",
            "max_tool_iterations": 23
        }))
        .send(router.clone())
        .await
        .json();
    assert_eq!(created.max_tool_iterations, Some(23));

    // The web form omits an untouched field. Absent must mean preserve, all the
    // way down to the column the chat turn reads.
    let renamed: CoachResponse = AxumTestRequest::put(&format!("/api/coaches/{}", created.id))
        .header("authorization", &auth)
        .json(&json!({ "title": "Renamed Pinned Coach" }))
        .send(router)
        .await
        .json();
    assert_eq!(renamed.title, "Renamed Pinned Coach");
    assert_eq!(renamed.max_tool_iterations, Some(23));

    let runtime = resources
        .coach
        .database
        .repositories()
        .coaches
        .get_coach_runtime_context(&created.id, tenant_id)
        .await
        .unwrap()
        .expect("coach runtime context");
    assert_eq!(runtime.max_tool_iterations, Some(23));
}

#[tokio::test]
async fn update_coach_with_an_explicit_null_clears_the_budget_back_to_the_admin_value() {
    let (router, auth, resources, tenant_id) = setup().await;

    let created: CoachResponse = AxumTestRequest::post("/api/coaches")
        .header("authorization", &auth)
        .json(&json!({
            "title": "Unpinnable Coach",
            "system_prompt": "You get twenty-three rounds until told otherwise.",
            "max_tool_iterations": 23
        }))
        .send(router.clone())
        .await
        .json();
    assert_eq!(created.max_tool_iterations, Some(23));

    // An explicit null is the form clearing the box: it must reset the pin,
    // which an absent field (preserve) can never do.
    let cleared: CoachResponse = AxumTestRequest::put(&format!("/api/coaches/{}", created.id))
        .header("authorization", &auth)
        .json(&json!({ "max_tool_iterations": null }))
        .send(router.clone())
        .await
        .json();
    assert_eq!(cleared.max_tool_iterations, None);

    let fetched: CoachResponse = AxumTestRequest::get(&format!("/api/coaches/{}", created.id))
        .header("authorization", &auth)
        .send(router)
        .await
        .json();
    assert_eq!(fetched.max_tool_iterations, None);

    let runtime = resources
        .coach
        .database
        .repositories()
        .coaches
        .get_coach_runtime_context(&created.id, tenant_id)
        .await
        .unwrap()
        .expect("coach runtime context");
    assert_eq!(runtime.max_tool_iterations, None);

    // With the coach column empty the turn falls through to the admin
    // parameter, on the very pool this server is running against.
    let svc = AdminConfigService::for_database(&resources.coach.database)
        .await
        .unwrap();
    let admin_value = svc
        .get_value("tool_execution.max_iterations", ConfigLookupScope::global())
        .await
        .unwrap()
        .expect("tool_execution.max_iterations is registered");
    assert_eq!(admin_value.as_i64(), Some(10));
    assert_eq!(i64::from(DEFAULT_MAX_TOOL_ITERATIONS), 10);
}

#[tokio::test]
async fn create_coach_accepts_both_ends_of_the_band() {
    let (router, auth, _resources, _tenant_id) = setup().await;

    let floor: CoachResponse = AxumTestRequest::post("/api/coaches")
        .header("authorization", &auth)
        .json(&json!({
            "title": "Floor Coach",
            "system_prompt": "One round only.",
            "max_tool_iterations": MIN_MAX_TOOL_ITERATIONS
        }))
        .send(router.clone())
        .await
        .json();
    assert_eq!(floor.max_tool_iterations, Some(1));

    let ceiling: CoachResponse = AxumTestRequest::post("/api/coaches")
        .header("authorization", &auth)
        .json(&json!({
            "title": "Ceiling Coach",
            "system_prompt": "Every round.",
            "max_tool_iterations": MAX_MAX_TOOL_ITERATIONS
        }))
        .send(router)
        .await
        .json();
    assert_eq!(ceiling.max_tool_iterations, Some(50));
}

// ============================================================================
// Range rejection
// ============================================================================

#[tokio::test]
async fn create_coach_rejects_a_budget_above_the_ceiling() {
    let (router, auth, _resources, _tenant_id) = setup().await;

    let response = AxumTestRequest::post("/api/coaches")
        .header("authorization", &auth)
        .json(&json!({
            "title": "Runaway Coach",
            "system_prompt": "Loop forever.",
            "max_tool_iterations": i32::from(MAX_MAX_TOOL_ITERATIONS) + 1
        }))
        .send(router.clone())
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    // The rejected create left no coach behind.
    let listed: serde_json::Value = AxumTestRequest::get("/api/coaches")
        .header("authorization", &auth)
        .send(router)
        .await
        .json();
    assert_eq!(listed["total"], json!(0));
}

#[tokio::test]
async fn create_coach_rejects_a_budget_below_the_floor() {
    let (router, auth, _resources, _tenant_id) = setup().await;

    let response = AxumTestRequest::post("/api/coaches")
        .header("authorization", &auth)
        .json(&json!({
            "title": "Zero Coach",
            "system_prompt": "Never call a tool.",
            "max_tool_iterations": 0
        }))
        .send(router)
        .await;

    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn update_coach_rejects_an_explicit_budget_above_the_ceiling() {
    let (router, auth, _resources, _tenant_id) = setup().await;

    let created: CoachResponse = AxumTestRequest::post("/api/coaches")
        .header("authorization", &auth)
        .json(&json!({
            "title": "Capped Coach",
            "system_prompt": "Original prompt",
            "max_tool_iterations": 23
        }))
        .send(router.clone())
        .await
        .json();

    // Clearing is the only way past the bounds check; a supplied number is
    // still range-checked exactly as before.
    let response = AxumTestRequest::put(&format!("/api/coaches/{}", created.id))
        .header("authorization", &auth)
        .json(&json!({ "max_tool_iterations": i32::from(MAX_MAX_TOOL_ITERATIONS) + 1 }))
        .send(router.clone())
        .await;
    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let fetched: CoachResponse = AxumTestRequest::get(&format!("/api/coaches/{}", created.id))
        .header("authorization", &auth)
        .send(router)
        .await
        .json();
    assert_eq!(fetched.max_tool_iterations, Some(23));
}

#[tokio::test]
async fn update_coach_rejects_an_out_of_range_budget_and_keeps_the_stored_one() {
    let (router, auth, _resources, _tenant_id) = setup().await;

    let created: CoachResponse = AxumTestRequest::post("/api/coaches")
        .header("authorization", &auth)
        .json(&json!({
            "title": "Guarded Coach",
            "system_prompt": "Original prompt",
            "max_tool_iterations": 9
        }))
        .send(router.clone())
        .await
        .json();

    let response = AxumTestRequest::put(&format!("/api/coaches/{}", created.id))
        .header("authorization", &auth)
        .json(&json!({ "max_tool_iterations": -4 }))
        .send(router.clone())
        .await;
    assert_eq!(response.status_code(), StatusCode::BAD_REQUEST);

    let fetched: CoachResponse = AxumTestRequest::get(&format!("/api/coaches/{}", created.id))
        .header("authorization", &auth)
        .send(router)
        .await
        .json();
    assert_eq!(fetched.max_tool_iterations, Some(9));
}

// ============================================================================
// Admin configuration parameter
// ============================================================================

#[tokio::test]
async fn tool_execution_max_iterations_is_a_registered_admin_parameter() {
    let db = create_test_db().await.unwrap();
    let svc = AdminConfigService::for_database(&db).await.unwrap();

    let value = svc
        .get_value("tool_execution.max_iterations", ConfigLookupScope::global())
        .await
        .unwrap()
        .expect("tool_execution.max_iterations is registered");

    // The chat pipeline reads this through `as_i64`, so assert the concrete
    // integer it will see rather than JSON equality with the same constant.
    assert_eq!(value.as_i64(), Some(10));
    assert_eq!(i64::from(DEFAULT_MAX_TOOL_ITERATIONS), 10);
}

#[tokio::test]
async fn tool_execution_max_iterations_validates_against_the_band() {
    let db = create_test_db().await.unwrap();
    let svc = AdminConfigService::for_database(&db).await.unwrap();

    let validate = |value: serde_json::Value| {
        let mut parameters = HashMap::new();
        parameters.insert("tool_execution.max_iterations".to_owned(), value);
        ValidateConfigRequest { parameters }
    };

    let accepted = svc
        .validate(&validate(json!(MAX_MAX_TOOL_ITERATIONS)))
        .await;
    assert!(accepted.is_valid, "errors: {:?}", accepted.errors);

    let rejected = svc
        .validate(&validate(json!(i32::from(MAX_MAX_TOOL_ITERATIONS) + 1)))
        .await;
    assert!(!rejected.is_valid);
    assert_eq!(rejected.errors.len(), 1);
    assert_eq!(
        rejected.errors[0].parameter,
        "tool_execution.max_iterations"
    );

    let too_small = svc.validate(&validate(json!(0))).await;
    assert!(!too_small.is_valid);
    assert_eq!(too_small.errors.len(), 1);
}

#[tokio::test]
async fn tool_execution_max_iterations_surfaces_in_the_admin_catalog() {
    let db = create_test_db().await.unwrap();
    let svc = AdminConfigService::for_database(&db).await.unwrap();

    let catalog = svc.get_catalog(ConfigLookupScope::global()).await.unwrap();
    let parameter = catalog
        .categories
        .iter()
        .flat_map(|category| &category.parameters)
        .find(|parameter| parameter.key == "tool_execution.max_iterations")
        .expect("tool_execution.max_iterations appears in the admin catalog");

    assert_eq!(parameter.category, "tool_execution");
    assert_eq!(parameter.display_name, "Max Tool Iterations");
    assert_eq!(parameter.data_type, ConfigDataType::Integer);
    assert_eq!(parameter.default_value.as_i64(), Some(10));
    assert!(parameter.is_runtime_configurable);

    let range = parameter
        .valid_range
        .as_ref()
        .expect("the parameter is range-bounded");
    assert_eq!(range.min.as_i64(), Some(1));
    assert_eq!(range.max.as_i64(), Some(50));
    assert_eq!(i64::from(MIN_MAX_TOOL_ITERATIONS), 1);
    assert_eq!(i64::from(MAX_MAX_TOOL_ITERATIONS), 50);
}
