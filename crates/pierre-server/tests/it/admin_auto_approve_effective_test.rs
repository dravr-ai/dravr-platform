// ABOUTME: Asserts the auto-approval control surface reports the effective value, not the requested one
// ABOUTME: Also asserts GET /api/providers surfaces a repository failure instead of "disconnected"
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Three "the answer must be true, not merely well-formed" contracts.
//!
//! 1. Auto-approval precedence is env var > database. An admin who saves the
//!    toggle while `AUTO_APPROVE_USERS` is set writes a row that changes
//!    nothing, so the value reported back has to be the effective one from
//!    [`admin_settings::get_auto_approval_settings`] — reporting the requested value
//!    shows success and then snaps back on the next read.
//! 2. `/api/providers` answers "disconnected" only for a user who genuinely
//!    holds no connection; a repository failure is a 5xx.
//! 3. Registration decides approval through the same
//!    [`admin_settings::get_auto_approval_settings`]: a stored setting that
//!    cannot be read refuses the registration instead of creating the account
//!    under whatever the process default happens to be.

use crate::common;
use crate::helpers;

use axum::http::StatusCode;
use common::{create_test_server_resources, create_test_user};
use helpers::axum_test::AxumTestRequest;
use pierre_config::environment::AppBehaviorConfig;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_auth::AuthRoutes;
use pierre_services::admin_settings;

/// Breaks the connection table so the status repository read fails.
const DROP_CONNECTIONS_TABLE: &str = "DROP TABLE provider_connections";

/// Breaks the settings table so the stored auto-approval read fails.
const DROP_SETTINGS_TABLE: &str = "DROP TABLE system_settings";

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
    admin_settings::set_auto_approval(data.database(), true)
        .await
        .unwrap();

    // AUTO_APPROVE_USERS=false in the environment outranks that row, so the
    // value an admin surface may report is `false` — never the requested `true`.
    let settings =
        admin_settings::get_auto_approval_settings(data.database(), &app_behavior(false, true))
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
    let settings =
        admin_settings::get_auto_approval_settings(data.database(), &app_behavior(false, false))
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
    admin_settings::set_auto_approval(data.database(), false)
        .await
        .unwrap();

    let settings =
        admin_settings::get_auto_approval_settings(data.database(), &app_behavior(true, true))
            .await
            .unwrap();
    assert!(
        settings.enabled,
        "env var must outrank the persisted row in the on direction too"
    );
    assert!(settings.overridden_by_env);
}

#[tokio::test]
async fn auto_approval_read_failure_is_an_error_when_the_environment_does_not_decide() {
    let resources = create_test_server_resources().await.unwrap();
    let data = resources.data();
    drop_settings_table(&resources).await;

    // With AUTO_APPROVE_USERS set the stored row is never read, so the
    // environment still answers.
    let settings =
        admin_settings::get_auto_approval_settings(data.database(), &app_behavior(true, true))
            .await
            .unwrap();
    assert!(settings.enabled);

    // Without it the row is the authority, and a failed read is reported
    // rather than replaced by the process default.
    let failure =
        admin_settings::get_auto_approval_settings(data.database(), &app_behavior(true, false))
            .await;
    assert!(
        failure.is_err(),
        "a failed read must not fall back to the configured default"
    );
}

#[tokio::test]
async fn registration_is_refused_when_the_auto_approval_setting_cannot_be_read() {
    let resources = create_test_server_resources().await.unwrap();
    let router = AuthRoutes::routes(resources.auth_routes_context());

    // Baseline: the same door creates an account while the setting reads.
    let response = AxumTestRequest::post("/api/auth/register")
        .json(&registration("readable-setting@example.com"))
        .send(router.clone())
        .await;
    assert_eq!(response.status_code(), StatusCode::CREATED);

    drop_settings_table(&resources).await;

    let email = "unreadable-setting@example.com";
    let response = AxumTestRequest::post("/api/auth/register")
        .json(&registration(email))
        .send(router)
        .await;
    assert_eq!(
        response.status_code(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "an unreadable approval setting must refuse the registration"
    );
    let created = resources
        .common
        .repos
        .users
        .get_by_email(email)
        .await
        .unwrap();
    assert!(
        created.is_none(),
        "no account may be created under an approval posture nobody chose"
    );
}

/// A registration body for `email`.
fn registration(email: &str) -> serde_json::Value {
    serde_json::json!({
        "email": email,
        "password": "registrationPassword123",
        "display_name": "Approval Setting Tester"
    })
}

/// Drop the table the stored auto-approval setting lives in.
async fn drop_settings_table(resources: &ServerContext) {
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(DROP_SETTINGS_TABLE)
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(DROP_SETTINGS_TABLE)
                .execute(db.pool())
                .await
                .unwrap();
        }
    }
}

#[tokio::test]
async fn providers_status_surfaces_a_repository_failure_instead_of_reporting_disconnected() {
    let resources = create_test_server_resources().await.unwrap();
    let (_user_id, user) = create_test_user(&resources.agent.database).await.unwrap();
    let token = resources
        .auth
        .auth_manager
        .generate_token(&user, &resources.auth.jwks_manager)
        .unwrap();
    let auth = format!("Bearer {token}");
    let router = AuthRoutes::routes(resources.auth_routes_context());

    // Baseline: a user holding no connection is genuinely disconnected.
    let response = AxumTestRequest::get("/api/providers")
        .header("authorization", &auth)
        .send(router.clone())
        .await;
    assert_eq!(response.status_code(), StatusCode::OK);
    let body: serde_json::Value = response.json();
    let cards = body["providers"].as_array().expect("provider cards");
    assert!(!cards.is_empty(), "the catalogue lists its cards: {body}");
    assert!(
        cards.iter().all(|c| c["connected"] == false),
        "nothing is connected yet: {body}"
    );

    // Break the backing table so the repository read fails for the same user.
    match resources.agent.database.backend() {
        DatabaseBackend::SQLite(db) => {
            sqlx::query(DROP_CONNECTIONS_TABLE)
                .execute(db.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(db) => {
            sqlx::query(DROP_CONNECTIONS_TABLE)
                .execute(db.pool())
                .await
                .unwrap();
        }
    }

    let response = AxumTestRequest::get("/api/providers")
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
