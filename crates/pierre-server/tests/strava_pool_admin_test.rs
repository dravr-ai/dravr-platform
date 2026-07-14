// ABOUTME: Authorization + happy-path tests for the admin Strava-pool endpoints
// ABOUTME: Super-admin CRUD lands in strava_oauth_app_pool with server-side encryption; non-super-admin is 403
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_core::admin::models::{AdminPermission, AdminPermissions, ValidatedAdminToken};
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_routes_admin::auth::service::AdminAuthService;
use pierre_routes_admin::handlers::strava_pool::{
    handle_delete_strava_pool_app, handle_list_strava_pool_apps,
    handle_set_strava_pool_app_enabled, handle_upsert_strava_pool_app,
};
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit};
use serde_json::json;
use uuid::Uuid;

async fn build_context() -> (
    Arc<AdminApiContext>,
    Arc<pierre_database::RepositoryRegistry>,
) {
    let database = common::create_test_database().await.unwrap();
    let auth_manager = common::create_test_auth_manager();
    let jwks_manager = common::get_shared_test_jwks();

    let database_arc = Arc::new((*database).clone());
    let repos_arc = Arc::new(database_arc.repositories());

    let context = AdminApiContext::new(AdminApiContextInit {
        database: database_arc,
        repos: repos_arc.clone(),
        jwt_secret: "test_admin_jwt_secret_for_strava_pool".to_owned(),
        auth_manager,
        jwks_manager,
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        admin_token_cache_ttl_secs: AdminAuthService::DEFAULT_CACHE_TTL_SECS,
        harness_config_registry: Arc::new(HarnessConfigRegistry::bootstrap()),
        prompt_registry: Arc::new(pierre_contremaitre::PromptRegistry::new()),
        tool_description_registry: Arc::new(pierre_contremaitre::ToolDescriptionRegistry::new()),
        evidence_registry: Arc::new(pierre_contremaitre::EvidenceRegistry::new()),
        messaging_strings_registry: Arc::new(pierre_contremaitre::MessagingStringsRegistry::new()),
        cageux_config_registry: Arc::new(CageuxConfigRegistry::from_env()),
        persona_contract_registry: Arc::new(PersonaContractRegistry::new()),
        contremaitre_config: None,
    });

    (Arc::new(context), repos_arc)
}

fn super_admin_token() -> ValidatedAdminToken {
    ValidatedAdminToken {
        token_id: format!("cookie:{}", Uuid::new_v4()),
        service_name: "admin@example.com".to_owned(),
        permissions: AdminPermissions::super_admin(),
        is_super_admin: true,
        tenant_id: None,
        user_info: None,
    }
}

fn non_super_admin_token() -> ValidatedAdminToken {
    ValidatedAdminToken {
        token_id: format!("cookie:{}", Uuid::new_v4()),
        service_name: "regular-admin@example.com".to_owned(),
        permissions: AdminPermissions::new(vec![AdminPermission::ManageAdminTokens]),
        is_super_admin: false,
        tenant_id: None,
        user_info: None,
    }
}

#[tokio::test]
async fn super_admin_can_crud_strava_pool_apps() {
    let (context, repos) = build_context().await;

    // POST: add a pool app (secret encrypted server-side).
    let resp = handle_upsert_strava_pool_app(
        State(context.clone()),
        Extension(super_admin_token()),
        Json(json!({
            "client_id": "201455",
            "client_secret": "0f2b184c076e60a35e8ced43db9c3c20c5fcf4f3",
            "seat_cap": 10,
            "label": "dravr-app-2"
        })),
    )
    .await
    .expect("upsert handler");
    assert_eq!(resp.status(), StatusCode::CREATED);

    // The app landed, and its secret roundtrips through decryption.
    let apps = repos
        .oauth_tokens
        .list_strava_pool_apps(false)
        .await
        .unwrap();
    assert_eq!(apps.len(), 1);
    assert_eq!(apps[0].client_id, "201455");
    assert_eq!(apps[0].seat_cap, 10);
    assert_eq!(
        repos
            .oauth_tokens
            .get_strava_pool_app_secret("201455")
            .await
            .unwrap()
            .as_deref(),
        Some("0f2b184c076e60a35e8ced43db9c3c20c5fcf4f3"),
        "server-encrypted secret decrypts back to the submitted value"
    );

    // GET: lists the app + seat summary.
    let resp = handle_list_strava_pool_apps(State(context.clone()), Extension(super_admin_token()))
        .await
        .expect("list handler");
    assert_eq!(resp.status(), StatusCode::OK);

    // PATCH: disable it.
    let resp = handle_set_strava_pool_app_enabled(
        State(context.clone()),
        Extension(super_admin_token()),
        Path("201455".to_owned()),
        Json(json!({ "enabled": false })),
    )
    .await
    .expect("patch handler");
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        !repos
            .oauth_tokens
            .list_strava_pool_apps(false)
            .await
            .unwrap()[0]
            .enabled,
        "app is now disabled"
    );

    // DELETE: remove it.
    let resp = handle_delete_strava_pool_app(
        State(context),
        Extension(super_admin_token()),
        Path("201455".to_owned()),
    )
    .await
    .expect("delete handler");
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(repos
        .oauth_tokens
        .list_strava_pool_apps(false)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn non_super_admin_is_forbidden() {
    let (context, repos) = build_context().await;

    let resp = handle_upsert_strava_pool_app(
        State(context),
        Extension(non_super_admin_token()),
        Json(json!({
            "client_id": "999999",
            "client_secret": "shouldnotbestoredatalllength30!",
            "seat_cap": 5
        })),
    )
    .await
    .expect("handler returns a response (403), not an error");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Nothing was written.
    assert!(repos
        .oauth_tokens
        .list_strava_pool_apps(false)
        .await
        .unwrap()
        .is_empty());
}
