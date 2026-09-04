// ABOUTME: Admin guardian-config endpoint tests — authz, persistence, hot-install, env-pin shadowing
// ABOUTME: Direct handler calls against a wired AdminApiContext (feature_flags_admin_authz_test pattern)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tests for `GET`/`PUT /admin/settings/guardian` (shared by the cookie and
//! bearer mounts — the handlers gate on the [`ValidatedAdminToken`] extension
//! both middlewares supply, so one direct-call suite covers both).
//!
//! Beyond authz, these pin the contract the CLI and web tab parse: the wire
//! shape of the response (`config` / `effective` / `sources` / `env_pinned`),
//! that a PUT persists the row AND hot-installs the policy the dispatch path
//! reads, and that an env-pinned field is reported instead of silently
//! shadowing the operator's edit.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::sync::Arc;

use axum::body::to_bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_core::admin::models::{AdminPermissions, ValidatedAdminToken};
use pierre_core::errors::ErrorCode;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_routes_admin::auth::service::AdminAuthService;
use pierre_routes_admin::handlers::guardian_config::{
    handle_get_guardian_config, handle_put_guardian_config,
};
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit};
use pierre_tool_runtime::guardian::{
    GuardianConfigDocument, GuardianConfigRegistry, GuardianConfigSource, GuardianEnvOverrides,
    GuardianMode, TaintedDestructive, GUARDIAN_CONFIG_SETTING_KEY,
};
use serde_json::Value;
use uuid::Uuid;

/// Build a fully-wired [`AdminApiContext`] with an explicit guardian
/// registry, so tests can inject env-pinned variants without touching
/// process env.
async fn build_context(guardian_registry: GuardianConfigRegistry) -> Arc<AdminApiContext> {
    let database = common::create_test_database().await.unwrap();
    let auth_manager = common::create_test_auth_manager();
    let jwks_manager = common::get_shared_test_jwks();

    let database_arc = Arc::new((*database).clone());
    let repos_arc = Arc::new(database_arc.repositories());

    Arc::new(AdminApiContext::new(AdminApiContextInit {
        database: database_arc,
        repos: repos_arc,
        jwt_secret: "test_admin_jwt_secret_for_guardian_config".to_owned(),
        auth_manager,
        jwks_manager,
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        admin_token_cache_ttl_secs: AdminAuthService::DEFAULT_CACHE_TTL_SECS,
        harness_config_registry: Arc::new(HarnessConfigRegistry::bootstrap()),
        guardian_config_registry: Arc::new(guardian_registry),
        prompt_registry: Arc::new(pierre_contremaitre::PromptRegistry::new()),
        tool_description_registry: Arc::new(pierre_contremaitre::ToolDescriptionRegistry::new()),
        evidence_registry: Arc::new(pierre_contremaitre::EvidenceRegistry::new()),
        messaging_strings_registry: Arc::new(pierre_contremaitre::MessagingStringsRegistry::new()),
        cageux_config_registry: Arc::new(CageuxConfigRegistry::from_env()),
        persona_contract_registry: Arc::new(PersonaContractRegistry::new()),
        training_catalogue_registry: Arc::new(pierre_contremaitre::TrainingCatalogueRegistry::new()),
        contremaitre_config: None,
    }))
}

fn env_free_registry() -> GuardianConfigRegistry {
    GuardianConfigRegistry::with_env_overrides(
        GuardianEnvOverrides::default(),
        GuardianConfigDocument::default(),
        GuardianConfigSource::Defaults,
    )
}

/// Cookie-style super-admin token, mirroring what `cookie_admin_middleware`
/// synthesizes for every web admin (and what a super-admin bearer carries).
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

/// Plain admin: passes both auth middlewares but carries only the default
/// keys-management permission set — no ViewConfiguration/ManageConfiguration.
fn plain_admin_token() -> ValidatedAdminToken {
    ValidatedAdminToken {
        token_id: format!("cookie:{}", Uuid::new_v4()),
        service_name: "plain-admin@example.com".to_owned(),
        permissions: AdminPermissions::default_admin(),
        is_super_admin: false,
        tenant_id: None,
        user_info: None,
    }
}

async fn body_json(resp: impl IntoResponse) -> (StatusCode, Value) {
    let resp = resp.into_response();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn get_serves_defaults_with_wire_shape_the_clients_parse() {
    let context = build_context(env_free_registry()).await;

    let resp = handle_get_guardian_config(State(context), Extension(super_admin_token()))
        .await
        .expect("super-admin GET succeeds");
    let (status, json) = body_json(resp).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["source"], "default");
    assert_eq!(json["updated_at"], Value::Null);
    assert_eq!(json["config"]["schema_version"], 1);
    assert_eq!(json["effective"]["mode"], "enforce");
    assert_eq!(json["effective"]["max_destructive_per_turn"], 1);
    assert_eq!(json["effective"]["max_writes_per_turn"], 50);
    assert_eq!(json["effective"]["tainted_destructive"], "log");
    assert_eq!(json["effective"]["plan_mode"], "off");
    assert_eq!(json["effective"]["external_send"], "none");
    assert_eq!(json["sources"]["mode"], "default");
    assert_eq!(json["env_pinned"], Value::Array(vec![]));
}

#[tokio::test]
async fn put_persists_the_row_and_hot_installs_the_policy() {
    let context = build_context(env_free_registry()).await;

    let document = GuardianConfigDocument {
        mode: Some(GuardianMode::Observe),
        tainted_destructive: Some(TaintedDestructive::Deny),
        ..GuardianConfigDocument::default()
    };
    let resp = handle_put_guardian_config(
        State(context.clone()),
        Extension(super_admin_token()),
        Json(document),
    )
    .await
    .expect("super-admin PUT succeeds");
    let (status, json) = body_json(resp).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["source"], "persisted");
    assert_eq!(json["effective"]["mode"], "observe");
    assert_eq!(json["effective"]["tainted_destructive"], "deny");
    assert_eq!(json["sources"]["mode"], "database");
    assert!(json["updated_at"].is_string());

    // The dispatch path reads the NEW policy immediately (hot install)…
    let effective = context.guardian_config_registry.current_guardian();
    assert_eq!(effective.policy().mode, GuardianMode::Observe);
    assert_eq!(
        effective.policy().tainted_destructive,
        TaintedDestructive::Deny
    );

    // …and the row survives a restart (persisted + parseable).
    let row = context
        .database
        .get_system_setting(GUARDIAN_CONFIG_SETTING_KEY)
        .await
        .unwrap()
        .expect("row persisted");
    let stored: GuardianConfigDocument = serde_json::from_str(&row.value).unwrap();
    assert_eq!(stored.mode, Some(GuardianMode::Observe));
    assert_eq!(stored.tainted_destructive, Some(TaintedDestructive::Deny));
}

#[tokio::test]
async fn put_rejects_an_invalid_document_without_installing() {
    let context = build_context(env_free_registry()).await;

    let Err(err) = handle_put_guardian_config(
        State(context.clone()),
        Extension(super_admin_token()),
        Json(GuardianConfigDocument {
            schema_version: 99,
            mode: Some(GuardianMode::Off),
            ..GuardianConfigDocument::default()
        }),
    )
    .await
    else {
        panic!("schema 99 must be rejected");
    };
    assert_eq!(err.code, ErrorCode::InvalidInput);

    // Nothing installed, nothing persisted.
    assert_eq!(
        context
            .guardian_config_registry
            .current_guardian()
            .policy()
            .mode,
        GuardianMode::Enforce
    );
    assert!(context
        .database
        .get_system_setting(GUARDIAN_CONFIG_SETTING_KEY)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn plain_admin_is_denied_both_verbs() {
    let context = build_context(env_free_registry()).await;

    let Err(err) =
        handle_get_guardian_config(State(context.clone()), Extension(plain_admin_token())).await
    else {
        panic!("plain admin must not read guardian config");
    };
    assert_eq!(err.code, ErrorCode::PermissionDenied);

    let Err(err) = handle_put_guardian_config(
        State(context),
        Extension(plain_admin_token()),
        Json(GuardianConfigDocument::default()),
    )
    .await
    else {
        panic!("plain admin must not write guardian config");
    };
    assert_eq!(err.code, ErrorCode::PermissionDenied);
}

#[tokio::test]
async fn env_pinned_field_shadows_the_edit_and_says_so() {
    let env = GuardianEnvOverrides {
        mode: Some(GuardianMode::Enforce),
        ..GuardianEnvOverrides::default()
    };
    let registry = GuardianConfigRegistry::with_env_overrides(
        env,
        GuardianConfigDocument::default(),
        GuardianConfigSource::Defaults,
    );
    let context = build_context(registry).await;

    let resp = handle_put_guardian_config(
        State(context.clone()),
        Extension(super_admin_token()),
        Json(GuardianConfigDocument {
            mode: Some(GuardianMode::Observe),
            ..GuardianConfigDocument::default()
        }),
    )
    .await
    .expect("PUT succeeds even when a field is env-pinned");
    let (status, json) = body_json(resp).await;

    assert_eq!(status, StatusCode::OK);
    // The edit persisted…
    assert_eq!(json["config"]["mode"], "observe");
    // …but the env pin wins and the response says exactly that.
    assert_eq!(json["effective"]["mode"], "enforce");
    assert_eq!(json["sources"]["mode"], "env");
    assert_eq!(json["env_pinned"][0], "mode");
    assert_eq!(
        context
            .guardian_config_registry
            .current_guardian()
            .policy()
            .mode,
        GuardianMode::Enforce
    );
}
