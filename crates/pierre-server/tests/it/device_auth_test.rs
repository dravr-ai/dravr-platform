// ABOUTME: Device Authorization Grant (RFC 8628) end-to-end tests through the admin handlers
// ABOUTME: authorization -> pending poll -> admin approve -> token poll mints an admin JWT; a token without the super-admin flag is 403
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;

use pierre_config::mcp::AppBehaviorConfig;
use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::Response;
use axum::{Extension, Json};
use chrono::Utc;
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_core::admin::models::{AdminPermission, AdminPermissions, ValidatedAdminToken};
use pierre_core::models::{DeviceAuthorization, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_routes_admin::handlers::device_auth::{
    handle_device_approve, handle_device_authorization, handle_device_token,
};
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit};
use pierre_tool_runtime::guardian::GuardianConfigRegistry;
use serde_json::{json, Value};
use uuid::Uuid;

const DEVICE_GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

async fn build_context() -> Arc<AdminApiContext> {
    let database = common::create_test_database().await.unwrap();
    let auth_manager = common::create_test_auth_manager();
    let jwks_manager = common::get_shared_test_jwks();

    let database_arc = Arc::new((*database).clone());
    let repos_arc = Arc::clone(database_arc.repositories());

    let context = AdminApiContext::new(AdminApiContextInit {
        database: database_arc,
        repos: repos_arc,
        jwt_secret: "test_admin_jwt_secret_for_device_auth".to_owned(),
        auth_manager,
        jwks_manager,
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        harness_config_registry: Arc::new(HarnessConfigRegistry::bootstrap()),
        guardian_config_registry: Arc::new(GuardianConfigRegistry::bootstrap()),
        prompt_registry: Arc::new(pierre_contremaitre::PromptRegistry::new()),
        tool_description_registry: Arc::new(pierre_contremaitre::ToolDescriptionRegistry::new()),
        evidence_registry: Arc::new(pierre_contremaitre::EvidenceRegistry::new()),
        messaging_strings_registry: Arc::new(pierre_contremaitre::MessagingStringsRegistry::new()),
        cageux_config_registry: Arc::new(CageuxConfigRegistry::from_env()),
        persona_contract_registry: Arc::new(PersonaContractRegistry::new()),
        training_catalogue_registry: Arc::new(pierre_contremaitre::TrainingCatalogueRegistry::new()),
        contremaitre_config: None,
        app_behavior: AppBehaviorConfig::default(),
    });

    Arc::new(context)
}

fn super_admin_token() -> ValidatedAdminToken {
    ValidatedAdminToken {
        token_id: format!("cookie:{}", Uuid::new_v4()),
        service_name: "admin@example.com".to_owned(),
        permissions: AdminPermissions::super_admin(),
        is_super_admin: true,
        tenant_id: None,
        user_info: None,
        operator_user_id: None,
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
        operator_user_id: None,
    }
}

async fn body_json(resp: Response) -> Value {
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn token_poll(device_code: &str) -> Value {
    json!({ "grant_type": DEVICE_GRANT, "device_code": device_code })
}

#[tokio::test]
async fn device_flow_mints_admin_token_after_super_admin_approval() {
    let context = build_context().await;

    // 1. Start authorization → device_code + user_code.
    let resp = handle_device_authorization(State(context.clone()))
        .await
        .expect("authorization");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let device_code = body["device_code"].as_str().unwrap().to_owned();
    let user_code = body["user_code"].as_str().unwrap().to_owned();
    assert_ne!(device_code, "");
    assert!(user_code.contains('-'), "user_code is grouped: {user_code}");

    // 2. Poll before approval → authorization_pending.
    let resp = handle_device_token(State(context.clone()), Json(token_poll(&device_code)))
        .await
        .expect("pending poll");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"], "authorization_pending");

    // 3. Admin approves.
    let resp = handle_device_approve(
        State(context.clone()),
        Extension(super_admin_token()),
        Json(json!({ "user_code": user_code })),
    )
    .await
    .expect("approve");
    assert_eq!(resp.status(), StatusCode::OK);

    // 4. Poll after approval → a real admin JWT.
    let resp = handle_device_token(State(context.clone()), Json(token_poll(&device_code)))
        .await
        .expect("approved poll");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    let access_token = body["access_token"].as_str().unwrap();
    assert_eq!(body["token_type"], "Bearer");
    assert_eq!(
        access_token.split('.').count(),
        3,
        "access_token is a signed JWT, got: {access_token}"
    );

    // 5. Single-use: a second poll finds nothing (row consumed).
    let resp = handle_device_token(State(context), Json(token_poll(&device_code)))
        .await
        .expect("second poll");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"], "expired_token");
}

#[tokio::test]
async fn non_super_admin_cannot_approve_device_login() {
    let context = build_context().await;

    let resp = handle_device_authorization(State(context.clone()))
        .await
        .expect("authorization");
    let body = body_json(resp).await;
    let device_code = body["device_code"].as_str().unwrap().to_owned();
    let user_code = body["user_code"].as_str().unwrap().to_owned();

    // An approval attempt by a token without the super-admin flag is forbidden.
    let resp = handle_device_approve(
        State(context.clone()),
        Extension(non_super_admin_token()),
        Json(json!({ "user_code": user_code })),
    )
    .await
    .expect("approve attempt");
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // And the login is still pending — no token can be minted.
    let resp = handle_device_token(State(context), Json(token_poll(&device_code)))
        .await
        .expect("poll still pending");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"], "authorization_pending");
}

#[tokio::test]
async fn unknown_device_code_is_rejected() {
    let context = build_context().await;
    let resp = handle_device_token(State(context), Json(token_poll("deadbeef-not-a-real-code")))
        .await
        .expect("poll unknown");
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(resp).await["error"], "expired_token");
}

/// A login nobody approved in time answers 410 "expired" until the next
/// login starts, which deletes the expired rows (carnet#1589). From then on
/// its code is no row at all, and the answer is the 404 of a code that was
/// never issued: so that 404 says the code is unknown or expired, not only
/// unknown, and tells the admin to start a new login.
#[tokio::test]
async fn an_expired_login_swept_by_the_next_one_is_answered_as_unknown_or_expired() {
    let context = build_context().await;
    let lapsed = "LAPS-ED00";
    let now = Utc::now().timestamp();
    context
        .repos
        .oauth2_server
        .create_device_authorization(&DeviceAuthorization {
            device_code_hash: "hash-of-a-lapsed-device-code".to_owned(),
            user_code: lapsed.to_owned(),
            status: "pending".to_owned(),
            approved_by: None,
            created_at: now - 700,
            expires_at: now - 100,
        })
        .await
        .unwrap();
    let approve = || {
        handle_device_approve(
            State(context.clone()),
            Extension(super_admin_token()),
            Json(json!({ "user_code": lapsed })),
        )
    };

    let resp = approve().await.expect("approve an expired login");
    assert_eq!(resp.status(), StatusCode::GONE, "expired, and still a row");

    handle_device_authorization(State(context.clone()))
        .await
        .expect("the next login");
    let resp = approve().await.expect("approve a swept login");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    let message = body_json(resp).await["message"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        message,
        format!("Unknown or expired user_code: {lapsed}. Start a new login for a new code.")
    );
}

#[tokio::test]
async fn the_minted_token_acts_as_the_approving_operator_and_names_no_email() {
    let context = build_context().await;
    let mut operator = User::new(
        "approving-operator@example.com".to_owned(),
        "hash".to_owned(),
        Some("Operator".to_owned()),
    );
    operator.role = UserRole::Admin;
    operator.user_status = UserStatus::Active;
    context.repos.users.create(&operator).await.unwrap();

    let resp = handle_device_authorization(State(context.clone()))
        .await
        .expect("authorization");
    let body = body_json(resp).await;
    let device_code = body["device_code"].as_str().unwrap().to_owned();
    let user_code = body["user_code"].as_str().unwrap().to_owned();

    let approver = ValidatedAdminToken {
        operator_user_id: Some(operator.id),
        ..super_admin_token()
    };
    let resp = handle_device_approve(
        State(context.clone()),
        Extension(approver),
        Json(json!({ "user_code": user_code })),
    )
    .await
    .expect("approve");
    assert_eq!(resp.status(), StatusCode::OK);

    let resp = handle_device_token(State(context.clone()), Json(token_poll(&device_code)))
        .await
        .expect("approved poll");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_json(resp).await;
    // The CLI that asked is told who approved it.
    assert_eq!(body["subject"], "approving-operator@example.com");

    let minted: Vec<_> = context
        .repos
        .admin
        .list_tokens(false)
        .await
        .unwrap()
        .into_iter()
        .filter(|token| token.operator_user_id == Some(operator.id))
        .collect();
    assert_eq!(minted.len(), 1, "one token acts as the approving operator");
    assert_eq!(
        minted[0].service_name,
        format!("device-cli:{}", operator.id),
        "the name is a label carrying the operator's id, never their email"
    );
    assert!(minted[0].is_super_admin);
}
