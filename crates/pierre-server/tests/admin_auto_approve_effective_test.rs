// ABOUTME: Asserts the auto-approval control surface reports the effective value, not the requested one
// ABOUTME: Also asserts GET /api/oauth/status surfaces a repository failure instead of "disconnected"
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Two admin-facing "the answer must be true, not merely well-formed" contracts.
//!
//! 1. Auto-approval precedence is env var > database. An admin who saves the
//!    toggle while `AUTO_APPROVE_USERS` is set writes a row that changes
//!    nothing, so the value reported back has to be the effective one from
//!    [`admin_ops::get_auto_approval_settings`] — reporting the requested value
//!    shows success and then snaps back on the next read.
//! 2. `/api/oauth/status` answers "disconnected" only for a user who genuinely
//!    holds no tokens; a repository failure is a 5xx.

mod common;
mod helpers;

use axum::http::StatusCode;
use common::{create_test_server_resources, create_test_user};
use helpers::axum_test::AxumTestRequest;
use pierre_config::environment::AppBehaviorConfig;
use pierre_routes_auth::AuthRoutes;
use pierre_services::admin_ops;

/// Build an app-behavior config whose only interesting axis is where the
/// auto-approval flag came from.
fn app_behavior(auto_approve_users: bool, from_env: bool) -> AppBehaviorConfig {
    AppBehaviorConfig {
        auto_approve_users,
        auto_approve_users_from_env: from_env,
        auto_approve_domains: vec!["dravr.ai".to_owned()],
        ..AppBehaviorConfig::default()
    }
}

#[tokio::test]
async fn auto_approval_resolves_to_the_env_value_and_flags_the_override() {
    let resources = create_test_server_resources().await.unwrap();
    let data = resources.data();

    // The admin flips the toggle on: the row lands in system_settings.
    admin_ops::set_auto_approval(&data, true).await.unwrap();

    // AUTO_APPROVE_USERS=false in the environment outranks that row, so the
    // value an admin surface may report is `false` — never the requested `true`.
    let settings = admin_ops::get_auto_approval_settings(&data, &app_behavior(false, true))
        .await
        .unwrap();
    assert!(
        !settings.enabled,
        "env var must outrank the persisted row, so the effective value is false"
    );
    assert!(
        settings.overridden_by_env,
        "the override must be reported so the control surface can lock the toggle"
    );
    assert_eq!(settings.auto_approve_domains, vec!["dravr.ai".to_owned()]);

    // With the env var absent the same stored row is the effective value and
    // the toggle is genuinely editable.
    let settings = admin_ops::get_auto_approval_settings(&data, &app_behavior(false, false))
        .await
        .unwrap();
    assert!(
        settings.enabled,
        "the persisted row wins when AUTO_APPROVE_USERS is unset"
    );
    assert!(!settings.overridden_by_env);
}

#[tokio::test]
async fn auto_approval_env_override_holds_after_a_write_that_agrees_with_the_database() {
    let resources = create_test_server_resources().await.unwrap();
    let data = resources.data();

    // Admin saves "off"; the environment says "on".
    admin_ops::set_auto_approval(&data, false).await.unwrap();

    let settings = admin_ops::get_auto_approval_settings(&data, &app_behavior(true, true))
        .await
        .unwrap();
    assert!(
        settings.enabled,
        "env var must outrank the persisted row in the on direction too"
    );
    assert!(settings.overridden_by_env);
}

#[tokio::test]
async fn oauth_status_surfaces_a_repository_failure_instead_of_reporting_disconnected() {
    let resources = create_test_server_resources().await.unwrap();
    let (_user_id, user) = create_test_user(&resources.coach.database).await.unwrap();
    let token = resources
        .auth
        .auth_manager
        .generate_token(&user, &resources.auth.jwks_manager)
        .unwrap();
    let auth = format!("Bearer {token}");
    let router = AuthRoutes::routes(resources.auth_routes_context());

    // Baseline: a user holding no tokens is genuinely disconnected.
    let response = AxumTestRequest::get("/api/oauth/status")
        .header("authorization", &auth)
        .send(router.clone())
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
    let body: serde_json::Value = response.json();
    let statuses = body.as_array().expect("status array");
    assert_eq!(statuses.len(), 2);
    assert_eq!(statuses[0]["provider"], "strava");
    assert_eq!(statuses[0]["connected"], false);
    assert_eq!(statuses[1]["provider"], "fitbit");
    assert_eq!(statuses[1]["connected"], false);

    // Break the backing table so the repository read fails for the same user.
    let pool = resources
        .coach
        .database
        .sqlite_pool()
        .expect("tests run on the SQLite backend");
    sqlx::query("DROP TABLE user_oauth_tokens")
        .execute(pool)
        .await
        .unwrap();

    let response = AxumTestRequest::get("/api/oauth/status")
        .header("authorization", &auth)
        .send(router)
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "a repository failure must not be reported as a successful disconnected status"
    );
    let body: serde_json::Value = response.json();
    assert_eq!(body["code"], "DatabaseError");
}
