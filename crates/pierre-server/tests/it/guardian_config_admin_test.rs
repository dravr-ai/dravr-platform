// ABOUTME: Admin guardian-config endpoint tests — authz, persistence, hot-install, compare-and-set
// ABOUTME: Direct handler calls against a wired AdminApiContext (feature_flags_admin_authz_test pattern)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tests for `GET`/`PUT /admin/settings/guardian` (the handlers gate on the
//! [`ValidatedAdminToken`] extension the admin-token middleware supplies, so most
//! of this suite calls them directly, and the real-mount tests at the end cover
//! what only a router can show).
//!
//! Beyond authz, these pin the contract the CLI parses: the wire shape of the
//! response, that a PUT persists the row AND hot-installs the policy the dispatch
//! path reads, that a save is a compare-and-set, that a field the policy does not
//! read is refused, and what each
//! write hands the audit recorder (the rows themselves are pinned through the app in
//! `admin_audit_trail_test`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use pierre_config::mcp::AppBehaviorConfig;
use std::sync::Arc;

use anyhow::Result;
use axum::body::to_bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use chrono::{DateTime, Utc};
use helpers::axum_test::AxumTestRequest;
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_document::{
    HarnessConfigDocument, HARNESS_CONFIG_SETTING_KEY,
};
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_core::admin::models::{
    AdminPermission, AdminPermissions, CreateAdminTokenRequest, ValidatedAdminToken,
};
use pierre_core::errors::ErrorCode;
use pierre_core::permissions::UserRole;
use pierre_database::backends::factory::DatabaseBackend;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::handlers::guardian_config::{
    handle_get_guardian_config, handle_put_guardian_config, GuardianConfigPutRequest,
};
use pierre_routes_admin::handlers::setting_revision::ExpectedRevision;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit, AdminRoutes, AuditDetail};
use pierre_tool_runtime::guardian::{
    GuardianConfigDocument, GuardianConfigRegistry, GuardianReloadOutcome,
    GUARDIAN_CONFIG_SETTING_KEY,
};
use secrecy::ExposeSecret;
use serde_json::{json, Value};
use serial_test::serial;
use uuid::Uuid;

/// Build a fully-wired [`AdminApiContext`] with an explicit guardian
/// registry.
async fn build_context(guardian_registry: GuardianConfigRegistry) -> Arc<AdminApiContext> {
    let database = common::create_test_database().await.unwrap();
    let auth_manager = common::create_test_auth_manager();
    let jwks_manager = common::get_shared_test_jwks();

    let database_arc = Arc::new((*database).clone());
    let repos_arc = Arc::clone(database_arc.repositories());

    Arc::new(AdminApiContext::new(AdminApiContextInit {
        database: database_arc,
        repos: repos_arc,
        jwt_secret: "test_admin_jwt_secret_for_guardian_config".to_owned(),
        auth_manager,
        jwks_manager,
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
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
        app_behavior: AppBehaviorConfig::default(),
    }))
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
        operator_user_id: None,
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
        operator_user_id: None,
    }
}

async fn body_json(resp: impl IntoResponse) -> (StatusCode, Value) {
    let resp = resp.into_response();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

/// A super-admin who is a real account: a save is recorded against it.
async fn super_admin_with_account(context: &AdminApiContext) -> ValidatedAdminToken {
    let (user_id, _) = common::create_test_user(&context.database).await.unwrap();
    ValidatedAdminToken {
        operator_user_id: Some(user_id),
        ..super_admin_token()
    }
}

fn revision(expected: Option<DateTime<Utc>>) -> ExpectedRevision {
    expected.map_or(ExpectedRevision::NoRow, ExpectedRevision::At)
}

/// A save of `document`, naming the base it edits.
fn put_request(
    document: &GuardianConfigDocument,
    expected_updated_at: ExpectedRevision,
) -> Json<GuardianConfigPutRequest> {
    let GuardianConfigDocument {
        max_destructive_per_turn,
        max_writes_per_turn,
    } = *document;
    Json(GuardianConfigPutRequest {
        max_destructive_per_turn,
        max_writes_per_turn,
        expected_updated_at,
    })
}

/// A save from an editor that read `expected`.
fn save(
    document: &GuardianConfigDocument,
    expected: Option<DateTime<Utc>>,
) -> Json<GuardianConfigPutRequest> {
    put_request(document, revision(expected))
}

async fn get_config(context: &Arc<AdminApiContext>) -> Value {
    let resp = handle_get_guardian_config(State(context.clone()), Extension(super_admin_token()))
        .await
        .expect("GET succeeds");
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK);
    json
}

/// The concurrency token as a client echoes it: the `updated_at` text.
fn token_of(response: &Value) -> Option<DateTime<Utc>> {
    response["updated_at"].as_str().map(|text| {
        DateTime::parse_from_rfc3339(text)
            .expect("updated_at is RFC 3339")
            .with_timezone(&Utc)
    })
}

/// A document whose destructive cap is away from the default.
fn capped() -> GuardianConfigDocument {
    GuardianConfigDocument {
        max_destructive_per_turn: Some(1),
        ..GuardianConfigDocument::default()
    }
}

async fn stored_value(context: &AdminApiContext) -> Option<String> {
    context
        .database
        .get_system_setting(GUARDIAN_CONFIG_SETTING_KEY)
        .await
        .unwrap()
        .map(|row| row.value)
}

#[tokio::test]
async fn get_serves_defaults_with_wire_shape_the_clients_parse() {
    let context = build_context(GuardianConfigRegistry::bootstrap()).await;

    let resp = handle_get_guardian_config(State(context), Extension(super_admin_token()))
        .await
        .expect("super-admin GET succeeds");
    let (status, json) = body_json(resp).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["updated_at"], Value::Null);
    assert_eq!(json["config"], json!({}));
    let caps = json!({"max_destructive_per_turn": 3, "max_writes_per_turn": 50});
    assert_eq!(
        json["effective"], caps,
        "the two caps, and nothing the Guardian no longer decides"
    );
    assert_eq!(json["defaults"], caps);
    assert_eq!(json["in_sync"], true);
    assert_eq!(json["degraded"], false);
    assert_eq!(json["degraded_reason"], Value::Null);
    assert!(json["loaded_at"].is_string());
    assert!(json["instance_id"].as_str().unwrap().contains(':'));
    for retired in [
        "source",
        "sources",
        "env_pinned",
        "stored_schema_version",
        "plan_mode_armable",
    ] {
        assert!(json.get(retired).is_none(), "{retired}: {json}");
    }
}

#[tokio::test]
async fn put_persists_the_row_and_hot_installs_the_policy() {
    let context = build_context(GuardianConfigRegistry::bootstrap()).await;

    let document = GuardianConfigDocument {
        max_destructive_per_turn: Some(4),
        ..GuardianConfigDocument::default()
    };
    let resp = handle_put_guardian_config(
        State(context.clone()),
        Extension(super_admin_with_account(&context).await),
        Ok(save(&document, None)),
    )
    .await
    .expect("super-admin PUT succeeds");
    let (status, json) = body_json(resp).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["effective"]["max_destructive_per_turn"], 4);
    assert_eq!(json["in_sync"], true);
    assert!(json["updated_at"].is_string());

    // The dispatch path reads the NEW policy immediately (hot install)…
    let effective = context.guardian_config_registry.current_guardian();
    assert_eq!(effective.policy().max_destructive_per_turn, 4);

    // …and the row survives a restart (persisted + parseable).
    let row = context
        .database
        .get_system_setting(GUARDIAN_CONFIG_SETTING_KEY)
        .await
        .unwrap()
        .expect("row persisted");
    let stored: Value = serde_json::from_str(&row.value).unwrap();
    assert_eq!(
        stored,
        json!({"max_destructive_per_turn": 4}),
        "the document alone: no base, no retired keys"
    );

    // The response's token is the stored one, so the next save can use it.
    assert_eq!(token_of(&json), Some(row.updated_at));
}

#[tokio::test]
async fn put_rejects_an_invalid_document_without_installing() {
    let context = build_context(GuardianConfigRegistry::bootstrap()).await;

    let refused = handle_put_guardian_config(
        State(context.clone()),
        Extension(super_admin_with_account(&context).await),
        Ok(save(
            &GuardianConfigDocument {
                max_writes_per_turn: Some(0),
                ..GuardianConfigDocument::default()
            },
            None,
        )),
    )
    .await
    .expect("a refusal is a response, so the trail can list it");
    let (status, body) = body_json(refused).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a zero write budget must be rejected"
    );
    assert!(body.to_string().contains("max_writes_per_turn"), "{body}");

    // Nothing installed, nothing persisted.
    assert_eq!(
        context
            .guardian_config_registry
            .current_guardian()
            .policy()
            .max_writes_per_turn,
        50
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
    let context = build_context(GuardianConfigRegistry::bootstrap()).await;

    let Err(err) =
        handle_get_guardian_config(State(context.clone()), Extension(plain_admin_token())).await
    else {
        panic!("plain admin must not read guardian config");
    };
    assert_eq!(err.code, ErrorCode::PermissionDenied);

    let Err(err) = handle_put_guardian_config(
        State(context),
        Extension(plain_admin_token()),
        Ok(save(&GuardianConfigDocument::default(), None)),
    )
    .await
    else {
        panic!("plain admin must not write guardian config");
    };
    assert_eq!(err.code, ErrorCode::PermissionDenied);
}

// ============================================================================
// Compare-and-set
// ============================================================================

/// Two super admins read the same version. The second to save learns it is stale,
/// rather than wiping the first one's edit.
#[tokio::test]
async fn a_stale_save_is_refused_and_the_winner_keeps_the_row() {
    let context = build_context(GuardianConfigRegistry::bootstrap()).await;
    let admin_user = super_admin_with_account(&context).await;

    let first = handle_put_guardian_config(
        State(context.clone()),
        Extension(admin_user.clone()),
        Ok(save(&capped(), None)),
    )
    .await
    .unwrap();
    let (_, first) = body_json(first).await;
    let base = token_of(&first);
    assert!(base.is_some());

    let winner = GuardianConfigDocument {
        max_writes_per_turn: Some(10),
        ..capped()
    };
    let resp = handle_put_guardian_config(
        State(context.clone()),
        Extension(admin_user.clone()),
        Ok(save(&winner, base)),
    )
    .await
    .unwrap();
    assert_eq!(body_json(resp).await.0, StatusCode::OK);
    let after_winner = stored_value(&context).await;

    let loser = GuardianConfigDocument {
        max_writes_per_turn: Some(20),
        ..capped()
    };
    let resp = handle_put_guardian_config(
        State(context.clone()),
        Extension(admin_user),
        Ok(save(&loser, base)),
    )
    .await
    .unwrap();
    let (status, json) = body_json(resp).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["code"], "guardian_config_stale");
    assert!(json["message"].as_str().unwrap().contains("Reload"));
    assert_eq!(stored_value(&context).await, after_winner);
    assert_eq!(
        context
            .guardian_config_registry
            .current_guardian()
            .policy()
            .max_writes_per_turn,
        10,
        "the refused save installs nothing"
    );
}

/// A client that sends no token cannot save at all, and that is told apart from
/// one that read no row, which is how the first save creates it.
#[tokio::test]
async fn a_save_naming_no_base_is_refused_and_null_means_no_row_was_read() {
    let context = build_context(GuardianConfigRegistry::bootstrap()).await;
    let admin_user = super_admin_with_account(&context).await;

    let missing: GuardianConfigPutRequest =
        serde_json::from_value(json!({"max_writes_per_turn": 7})).unwrap();
    assert_eq!(missing.expected_updated_at, ExpectedRevision::Unnamed);
    let (status, json) = body_json(
        handle_put_guardian_config(
            State(context.clone()),
            Extension(admin_user.clone()),
            Ok(Json(missing)),
        )
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(json["code"], "guardian_config_stale");
    assert!(json["message"]
        .as_str()
        .unwrap()
        .contains("expected_updated_at"));
    assert_eq!(stored_value(&context).await, None);

    let null: GuardianConfigPutRequest = serde_json::from_value(json!({
        "max_writes_per_turn": 7,
        "expected_updated_at": null,
    }))
    .unwrap();
    assert_eq!(null.expected_updated_at, ExpectedRevision::NoRow);
    assert_eq!(null.max_writes_per_turn, Some(7));
    let (status, _) = body_json(
        handle_put_guardian_config(
            State(context.clone()),
            Extension(admin_user),
            Ok(Json(null)),
        )
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(stored_value(&context).await.is_some());
}

/// The wipe this closes: a pod that booted on defaults after a read error served
/// an empty document, and saving it replaced the row. GET now reads the row, and
/// the empty pod's own assumption fails the compare.
#[tokio::test]
async fn an_instance_on_defaults_serves_the_row_and_cannot_overwrite_it_blind() {
    let context = build_context(GuardianConfigRegistry::bootstrap()).await;
    let admin_user = super_admin_with_account(&context).await;
    let saved_elsewhere = GuardianConfigDocument {
        max_destructive_per_turn: Some(5),
        max_writes_per_turn: Some(11),
    };
    let row_json = serde_json::to_string(&saved_elsewhere).unwrap();
    context
        .database
        .set_system_setting(GUARDIAN_CONFIG_SETTING_KEY, &row_json)
        .await
        .unwrap();

    let json = get_config(&context).await;
    assert_eq!(json["config"]["max_destructive_per_turn"], 5);
    assert_eq!(json["config"]["max_writes_per_turn"], 11);
    assert_eq!(
        json["in_sync"], false,
        "this instance has not polled the row in"
    );
    assert_eq!(json["effective"]["max_destructive_per_turn"], 3);
    assert!(json["updated_at"].is_string());

    let (status, _) = body_json(
        handle_put_guardian_config(
            State(context.clone()),
            Extension(admin_user),
            Ok(save(&GuardianConfigDocument::default(), None)),
        )
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(stored_value(&context).await, Some(row_json));

    // The poll closes the gap the flag reported.
    assert_eq!(
        context
            .guardian_config_registry
            .reload_from_database(&context.database)
            .await,
        GuardianReloadOutcome::Installed
    );
    assert_eq!(get_config(&context).await["in_sync"], true);
}

/// A failed read must not look like an unconfigured policy, which is what an
/// editor would then save over the real one.
#[tokio::test]
async fn a_failed_row_read_is_an_error_not_a_default_document() {
    let context = build_context(GuardianConfigRegistry::bootstrap()).await;
    match context.database.backend() {
        DatabaseBackend::SQLite(sqlite) => sqlite.pool().close().await,
        #[cfg(feature = "postgresql")]
        DatabaseBackend::PostgreSQL(postgres) => postgres.pool().close().await,
    }

    let Err(err) = handle_get_guardian_config(State(context), Extension(super_admin_token())).await
    else {
        panic!("a failed read must not answer with a document");
    };
    assert_eq!(err.code, ErrorCode::InternalError);
}

// ============================================================================
// Scope and actor
// ============================================================================

fn tenant_scoped_token() -> ValidatedAdminToken {
    ValidatedAdminToken {
        token_id: Uuid::new_v4().to_string(),
        service_name: "tenant-service".to_owned(),
        permissions: AdminPermissions::super_admin(),
        is_super_admin: false,
        tenant_id: Some(Uuid::new_v4().to_string()),
        user_info: None,
        operator_user_id: Some(Uuid::new_v4()),
    }
}

#[tokio::test]
async fn a_tenant_scoped_token_holding_every_permission_is_refused_every_verb() {
    let context = build_context(GuardianConfigRegistry::bootstrap()).await;

    let Err(err) =
        handle_get_guardian_config(State(context.clone()), Extension(tenant_scoped_token())).await
    else {
        panic!("the policy is global");
    };
    assert_eq!(err.code, ErrorCode::PermissionDenied);

    let Err(err) = handle_put_guardian_config(
        State(context.clone()),
        Extension(tenant_scoped_token()),
        Ok(save(&capped(), None)),
    )
    .await
    else {
        panic!("the policy is global");
    };
    assert_eq!(err.code, ErrorCode::PermissionDenied);
    assert_eq!(stored_value(&context).await, None);
}

/// A console super-admin's synthesized token carries the active tenant. Refusing
/// on `tenant_id` alone would lock the whole web console out of this tab.
#[tokio::test]
async fn a_console_super_admin_with_an_active_tenant_keeps_the_tab() {
    let context = build_context(GuardianConfigRegistry::bootstrap()).await;
    let console_session = ValidatedAdminToken {
        tenant_id: Some(Uuid::new_v4().to_string()),
        ..super_admin_with_account(&context).await
    };

    assert!(
        handle_get_guardian_config(State(context.clone()), Extension(console_session.clone()))
            .await
            .is_ok()
    );
    let (status, _) = body_json(
        handle_put_guardian_config(
            State(context),
            Extension(console_session),
            Ok(save(&capped(), None)),
        )
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
}

/// A token naming no account still saves: the audit recorder names the token.
#[tokio::test]
async fn a_service_token_naming_no_account_can_save() {
    let context = build_context(GuardianConfigRegistry::bootstrap()).await;

    let (status, _) = body_json(
        handle_put_guardian_config(
            State(context.clone()),
            Extension(super_admin_token()),
            Ok(save(&capped(), None)),
        )
        .await
        .unwrap(),
    )
    .await;

    assert_eq!(status, StatusCode::OK);
    assert!(stored_value(&context).await.is_some());
}

// ============================================================================
// Corrupt rows and foreign schemas
// ============================================================================

/// The recovery path: the row does not parse, and the super admin can still see that,
/// see what is in force, and save over it with the row's own token.
#[tokio::test]
async fn a_corrupt_row_is_reported_and_can_be_saved_over() {
    let context = build_context(GuardianConfigRegistry::bootstrap()).await;
    let admin_user = super_admin_with_account(&context).await;
    context.guardian_config_registry.install(capped());
    context
        .database
        .set_system_setting(GUARDIAN_CONFIG_SETTING_KEY, "{not json")
        .await
        .unwrap();

    let json = get_config(&context).await;
    assert_eq!(json["degraded"], true);
    assert!(json["degraded_reason"]
        .as_str()
        .unwrap()
        .contains("does not parse"));
    assert_eq!(json["in_sync"], false);
    assert_eq!(
        json["config"]["max_destructive_per_turn"], 1,
        "the editor starts from the last good document, not an empty form"
    );
    assert_eq!(json["effective"]["max_destructive_per_turn"], 1);

    let resp = handle_put_guardian_config(
        State(context.clone()),
        Extension(admin_user),
        Ok(save(&capped(), token_of(&json))),
    )
    .await
    .unwrap();
    let audit = audit_detail(&resp);
    let (status, saved) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(saved["degraded"], false);
    assert_eq!(saved["in_sync"], true);

    // The audit row keeps what the save replaced, verbatim.
    assert_eq!(audit.before, Some(json!("{not json")));
    assert!(audit.summary.unwrap().contains("not a document"));
}

/// A row an earlier binary wrote, in the shape it wrote (carnet#1471): GET serves
/// what this binary reads of it, and a client echoing that saves, dropping the
/// retired keys and anything else this binary does not read.
#[tokio::test]
async fn a_row_an_earlier_binary_wrote_is_served_and_can_be_saved_over() {
    let context = build_context(GuardianConfigRegistry::bootstrap()).await;
    let admin_user = super_admin_with_account(&context).await;
    context
        .database
        .set_system_setting(
            GUARDIAN_CONFIG_SETTING_KEY,
            r#"{"schema_version": 2, "mode": "observe", "max_destructive_per_turn": 1, "future_rule": true}"#,
        )
        .await
        .unwrap();

    let json = get_config(&context).await;
    assert_eq!(json["config"], json!({"max_destructive_per_turn": 1}));
    assert_eq!(json["stored_unknown_fields"], json!(["future_rule"]));
    assert_eq!(json["degraded"], false);

    let mut echoed = json["config"].clone();
    echoed["expected_updated_at"] = json["updated_at"].clone();
    let request: GuardianConfigPutRequest = serde_json::from_value(echoed).unwrap();
    let resp = handle_put_guardian_config(
        State(context.clone()),
        Extension(admin_user),
        Ok(Json(request)),
    )
    .await
    .unwrap();
    let audit = audit_detail(&resp);
    let (status, saved) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    assert!(saved.get("stored_unknown_fields").is_none());
    assert_eq!(
        serde_json::from_str::<Value>(&stored_value(&context).await.unwrap()).unwrap(),
        json!({"max_destructive_per_turn": 1})
    );

    // What the save dropped survives in the audit row.
    let before = audit.before.unwrap();
    assert_eq!(before["future_rule"], true);
    assert_eq!(before["mode"], "observe");
    let summary = audit.summary.unwrap();
    assert!(summary.contains("future_rule: true → dropped"), "{summary}");
    assert!(
        summary.contains(r#"mode: "observe" → dropped"#),
        "{summary}"
    );
}

/// carnet#1471: a save needs its base and nothing more, a zero destructive budget
/// included.
#[tokio::test]
async fn a_budget_edit_saves_on_its_base_alone() {
    let context = build_context(GuardianConfigRegistry::bootstrap()).await;
    let admin_user = super_admin_with_account(&context).await;

    let (status, json) = body_json(
        handle_put_guardian_config(
            State(context),
            Extension(admin_user),
            Ok(put_request(
                &GuardianConfigDocument {
                    max_writes_per_turn: Some(20),
                    max_destructive_per_turn: Some(0),
                },
                ExpectedRevision::NoRow,
            )),
        )
        .await
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["effective"]["max_writes_per_turn"], 20);
    assert_eq!(json["effective"]["max_destructive_per_turn"], 0);
}

// ============================================================================
// Audit
// ============================================================================

/// What a response hands the audit recorder.
fn audit_detail(resp: &Response) -> AuditDetail {
    resp.extensions()
        .get::<AuditDetail>()
        .cloned()
        .expect("the response must carry its audit detail")
}

#[tokio::test]
async fn a_save_hands_the_recorder_what_it_replaced_and_what_it_wrote() {
    let context = build_context(GuardianConfigRegistry::bootstrap()).await;
    let admin_user = super_admin_with_account(&context).await;
    let mut base = None;
    let mut details = Vec::new();
    for writes in [5_u16, 6] {
        let resp = handle_put_guardian_config(
            State(context.clone()),
            Extension(admin_user.clone()),
            Ok(save(
                &GuardianConfigDocument {
                    max_writes_per_turn: Some(writes),
                    ..GuardianConfigDocument::default()
                },
                base,
            )),
        )
        .await
        .unwrap();
        details.push(audit_detail(&resp));
        let (status, json) = body_json(resp).await;
        assert_eq!(status, StatusCode::OK);
        base = token_of(&json);
    }

    let first = &details[0];
    assert_eq!(first.action.as_deref(), Some("guardian.config.update"));
    assert_eq!(first.before, Some(Value::Null), "the first save had no row");
    assert_eq!(first.after.as_ref().unwrap()["max_writes_per_turn"], 5);
    assert_eq!(
        first.summary.as_deref(),
        Some("Created the Guardian policy")
    );

    let second = &details[1];
    assert_eq!(second.before.as_ref().unwrap()["max_writes_per_turn"], 5);
    assert_eq!(second.after.as_ref().unwrap()["max_writes_per_turn"], 6);
    assert_eq!(
        second.summary.as_deref(),
        Some("Guardian policy max_writes_per_turn: 5 → 6")
    );
}

#[tokio::test]
async fn a_refused_save_names_the_action_and_carries_no_change() {
    let context = build_context(GuardianConfigRegistry::bootstrap()).await;
    let resp = handle_put_guardian_config(
        State(context.clone()),
        Extension(super_admin_token()),
        Ok(put_request(&capped(), ExpectedRevision::Unnamed)),
    )
    .await
    .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let audit = audit_detail(&resp);
    assert_eq!(audit.action.as_deref(), Some("guardian.config.update"));
    assert_eq!(audit.before, None);
    assert_eq!(audit.after, None);
}

// ============================================================================
// Through the real mounts
// ============================================================================

fn admin_context(resources: &Arc<ServerContext>) -> AdminApiContext {
    AdminApiContext::new(AdminApiContextInit {
        database: resources.agent.database.clone(),
        repos: resources.common.repos.clone(),
        jwt_secret: resources.auth.admin_jwt_secret.to_string(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        harness_config_registry: resources.fitness.harness_config_registry.clone(),
        guardian_config_registry: resources.fitness.guardian_config_registry.clone(),
        prompt_registry: resources.mcp.prompt_registry.clone(),
        tool_description_registry: resources.mcp.tool_description_registry.clone(),
        evidence_registry: resources.mcp.evidence_registry.clone(),
        messaging_strings_registry: resources.mcp.messaging_strings_registry.clone(),
        cageux_config_registry: resources.fitness.cageux_config_registry.clone(),
        persona_contract_registry: resources.fitness.persona_contract_registry.clone(),
        training_catalogue_registry: resources.mcp.training_catalogue_registry.clone(),
        contremaitre_config: None,
        app_behavior: resources.common.config.app_behavior.clone(),
    })
}

fn console(resources: &Arc<ServerContext>) -> axum::Router {
    AdminRoutes::cookie_admin_routes::<ServerContext>(admin_context(resources), resources)
}

fn token_api(resources: &Arc<ServerContext>) -> axum::Router {
    AdminRoutes::routes(admin_context(resources))
}

async fn bearer_token(resources: &Arc<ServerContext>, tenant_id: Option<String>) -> Result<String> {
    let request = CreateAdminTokenRequest {
        service_name: format!("guardian-test-{}", Uuid::new_v4()),
        service_description: None,
        permissions: Some(vec![
            AdminPermission::ViewConfiguration,
            AdminPermission::ManageConfiguration,
        ]),
        expires_in_days: Some(1),
        is_super_admin: false,
        tenant_id,
        operator_user_id: None,
    };
    let token = resources
        .common
        .repos
        .admin
        .create_token(
            &request,
            resources.auth.admin_jwt_secret.as_ref(),
            &resources.auth.jwks_manager,
        )
        .await?;
    Ok(format!("Bearer {}", token.jwt_token.expose_secret()))
}

/// A tenant-scoped token reaches none of the global settings through the mount
/// `pierre-cli` uses, harness included, while a global token with the same
/// permissions does, so the refusal can only be the scope gate.
#[tokio::test]
#[serial]
async fn the_token_mount_refuses_a_tenant_scoped_token_on_every_global_setting() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let (_, _, tenant_id) = common::create_test_user_with_plan(
        &resources.agent.database,
        "scoped-owner@example.com",
        "starter",
    )
    .await?;
    let scoped = bearer_token(&resources, Some(tenant_id.to_string())).await?;
    let global = bearer_token(&resources, None).await?;

    for path in ["/admin/settings/guardian", "/admin/settings/harness"] {
        let refused = AxumTestRequest::get(path)
            .header("authorization", &scoped)
            .send(token_api(&resources))
            .await;
        assert_eq!(refused.status(), 403, "{path} must refuse a scoped token");

        let served = AxumTestRequest::get(path)
            .header("authorization", &global)
            .send(token_api(&resources))
            .await;
        assert_eq!(served.status(), 200, "{path} serves a global token");
    }

    for path in ["/admin/settings/guardian", "/admin/settings/harness"] {
        let refused = AxumTestRequest::put(path)
            .header("authorization", &scoped)
            .header("content-type", "application/json")
            .json(&json!({
                "schema_version": 2,
                "compaction": {},
                "guardrails": {},
                "expected_updated_at": null,
            }))
            .send(token_api(&resources))
            .await;
        assert_ne!(
            refused.status(),
            200,
            "{path} must not take a scoped token's write"
        );
    }
    assert!(resources
        .agent
        .database
        .get_system_setting(GUARDIAN_CONFIG_SETTING_KEY)
        .await?
        .is_none());
    Ok(())
}

/// The mount `pierre-cli` uses: a device-login token and a service token with the
/// configuration permissions both save.
#[tokio::test]
#[serial]
async fn the_token_mount_saves_for_a_device_login_and_for_a_service_token() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let (admin_id, _, _) = common::create_test_user_with_plan(
        &resources.agent.database,
        "device-admin@example.com",
        "starter",
    )
    .await?;
    let device_login = resources
        .common
        .repos
        .admin
        .create_token(
            &CreateAdminTokenRequest {
                service_name: format!("guardian-test-{}", Uuid::new_v4()),
                service_description: None,
                permissions: None,
                expires_in_days: Some(1),
                is_super_admin: true,
                tenant_id: None,
                operator_user_id: Some(admin_id),
            },
            resources.auth.admin_jwt_secret.as_ref(),
            &resources.auth.jwks_manager,
        )
        .await?;
    let device_login = format!("Bearer {}", device_login.jwt_token.expose_secret());
    let service = bearer_token(&resources, None).await?;

    let by_service = AxumTestRequest::put("/admin/settings/guardian")
        .header("authorization", &service)
        .json(&json!({"max_writes_per_turn": 12, "expected_updated_at": null}))
        .send(token_api(&resources))
        .await;
    assert_eq!(by_service.status(), 200);
    let by_service: Value = by_service.json();

    let by_device_login = AxumTestRequest::put("/admin/settings/guardian")
        .header("authorization", &device_login)
        .json(&json!({
            "max_writes_per_turn": 13,
            "expected_updated_at": by_service["updated_at"],
        }))
        .send(token_api(&resources))
        .await;
    assert_eq!(by_device_login.status(), 200);
    Ok(())
}

fn harness_document() -> Value {
    serde_json::to_value(HarnessConfigDocument::default()).unwrap()
}

fn harness_save(topic: &str, expected: Option<&Value>) -> Value {
    let mut body = harness_document();
    body["guardrails"]["blocked_topics"] = json!([topic]);
    if let Some(expected) = expected {
        body["expected_updated_at"] = expected.clone();
    }
    body
}

async fn stored_harness(resources: &Arc<ServerContext>) -> Result<Option<Value>> {
    Ok(resources
        .agent
        .database
        .get_system_setting(HARNESS_CONFIG_SETTING_KEY)
        .await?
        .map(|row| serde_json::from_str(&row.value).unwrap()))
}

/// carnet#1418: the harness document answers to the contract the Guardian's does.
#[tokio::test]
#[serial]
async fn a_harness_save_is_a_compare_and_set() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let token = bearer_token(&resources, None).await?;
    let put = |body: Value| {
        AxumTestRequest::put("/admin/settings/harness")
            .header("authorization", &token)
            .json(&body)
            .send(token_api(&resources))
    };

    // No token at all: refused, and no row appears.
    let missing = put(harness_document()).await;
    assert_eq!(missing.status(), 409, "{}", missing.body_text());
    let missing: Value = missing.json();
    assert_eq!(missing["code"], "harness_config_stale");
    assert!(missing["message"]
        .as_str()
        .unwrap()
        .contains("expected_updated_at"));
    assert_eq!(stored_harness(&resources).await?, None);

    // Null means no row was read.
    let read = AxumTestRequest::get("/admin/settings/harness")
        .header("authorization", &token)
        .send(token_api(&resources))
        .await;
    let read: Value = read.json();
    assert_eq!(read["updated_at"], Value::Null);
    let created = put(harness_save("first", Some(&read["updated_at"]))).await;
    assert_eq!(created.status(), 200, "{}", created.body_text());
    let created: Value = created.json();

    // A null base no longer holds once a row exists.
    let late_create = put(harness_save("late", Some(&Value::Null))).await;
    assert_eq!(late_create.status(), 409);

    // The token a GET returns is the row's, and the save's own round-trips.
    let read = AxumTestRequest::get("/admin/settings/harness")
        .header("authorization", &token)
        .send(token_api(&resources))
        .await;
    let read: Value = read.json();
    assert_eq!(read["updated_at"], created["updated_at"]);

    // Two editors read the same row: the first wins and the second writes nothing.
    let winner = put(harness_save("winner", Some(&read["updated_at"]))).await;
    assert_eq!(winner.status(), 200, "{}", winner.body_text());
    let stale = put(harness_save("loser", Some(&read["updated_at"]))).await;
    assert_eq!(stale.status(), 409);
    let stale: Value = stale.json();
    assert_eq!(stale["code"], "harness_config_stale");
    assert_eq!(
        stored_harness(&resources).await?.unwrap()["guardrails"]["blocked_topics"],
        json!(["winner"]),
        "the winner keeps the row"
    );

    // A body that does not parse answers in the same shape.
    let malformed = put(json!({"compaction": "nope"})).await;
    assert_ne!(malformed.status(), 200);
    let malformed: Value = malformed.json();
    assert_eq!(malformed["code"], "harness_config_malformed");
    Ok(())
}

/// A save is the only repair for a row that does not parse, and it needs a token, so
/// GET must keep giving one.
#[tokio::test]
#[serial]
async fn a_corrupt_harness_row_can_still_be_read_and_replaced() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let token = bearer_token(&resources, None).await?;
    resources
        .agent
        .database
        .set_system_setting(HARNESS_CONFIG_SETTING_KEY, "{not json")
        .await?;

    let read = AxumTestRequest::get("/admin/settings/harness")
        .header("authorization", &token)
        .send(token_api(&resources))
        .await;
    assert_eq!(read.status(), 200, "{}", read.body_text());
    let read: Value = read.json();
    assert!(read["degraded_reason"]
        .as_str()
        .unwrap()
        .contains("does not parse"));
    assert!(read["updated_at"].is_string());

    let repaired = AxumTestRequest::put("/admin/settings/harness")
        .header("authorization", &token)
        .json(&harness_save("repaired", Some(&read["updated_at"])))
        .send(token_api(&resources))
        .await;
    assert_eq!(repaired.status(), 200, "{}", repaired.body_text());
    let repaired: Value = repaired.json();
    assert!(repaired.get("degraded_reason").is_none());
    Ok(())
}

/// The whole wire round trip: the body's fields and its null token decode as the
/// client sends them, and the token a save returns is the next save's base.
#[tokio::test]
#[serial]
async fn the_token_mount_saves_end_to_end() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let token = bearer_token(&resources, None).await?;

    let read = AxumTestRequest::get("/admin/settings/guardian")
        .header("authorization", &token)
        .send(token_api(&resources))
        .await;
    assert_eq!(read.status(), 200);
    let read: Value = read.json();
    assert_eq!(read["updated_at"], Value::Null);

    let no_token = AxumTestRequest::put("/admin/settings/guardian")
        .header("authorization", &token)
        .header("content-type", "application/json")
        .json(&json!({"max_writes_per_turn": 12}))
        .send(token_api(&resources))
        .await;
    assert_eq!(no_token.status(), 409);

    let saved = AxumTestRequest::put("/admin/settings/guardian")
        .header("authorization", &token)
        .header("content-type", "application/json")
        .json(&json!({"max_writes_per_turn": 12, "expected_updated_at": read["updated_at"]}))
        .send(token_api(&resources))
        .await;
    assert_eq!(saved.status(), 200);
    let saved: Value = saved.json();
    assert_eq!(saved["effective"]["max_writes_per_turn"], 12);

    // The token the save returned round-trips as the next save's base.
    let again = AxumTestRequest::put("/admin/settings/guardian")
        .header("authorization", &token)
        .header("content-type", "application/json")
        .json(&json!({"max_writes_per_turn": 13, "expected_updated_at": saved["updated_at"]}))
        .send(token_api(&resources))
        .await;
    assert_eq!(again.status(), 200);
    Ok(())
}

/// The web console's editor went with its routes: the policy has the one mount, and
/// a super admin's session reaches nothing under the console's prefix.
#[tokio::test]
#[serial]
async fn the_console_serves_no_guardian_settings() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let (_, mut user, _) = common::create_test_user_with_plan(
        &resources.agent.database,
        "console-super@example.com",
        "starter",
    )
    .await?;
    user.is_admin = true;
    user.role = UserRole::SuperAdmin;
    resources.common.repos.users.update(&user).await?;
    let session = format!(
        "Bearer {}",
        common::generate_test_token(&resources, &user).await
    );

    for path in [
        "/api/admin/settings/guardian",
        "/api/admin/settings/guardian/pending",
    ] {
        let response = AxumTestRequest::get(path)
            .header("authorization", &session)
            .send(console(&resources))
            .await;
        assert_eq!(response.status(), 404, "{path}");
    }
    let bearer = bearer_token(&resources, None).await?;
    let pending = AxumTestRequest::get("/admin/settings/guardian/pending")
        .header("authorization", &bearer)
        .send(token_api(&resources))
        .await;
    assert_eq!(pending.status(), 404);
    Ok(())
}

/// `Follow default` has to name what the field would follow, and that is not what
/// this instance enforces while the row says otherwise.
#[tokio::test]
async fn defaults_stay_the_compiled_ones_when_the_row_changes_what_is_enforced() {
    let context = build_context(GuardianConfigRegistry::bootstrap()).await;
    let admin_user = super_admin_with_account(&context).await;
    let resp = handle_put_guardian_config(
        State(context.clone()),
        Extension(admin_user),
        Ok(save(&capped(), None)),
    )
    .await
    .unwrap();
    let (status, json) = body_json(resp).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["effective"]["max_destructive_per_turn"], 1);
    assert_eq!(json["defaults"]["max_destructive_per_turn"], 3);
}

/// The extractor's rejection reaches the client as the admin API's own error
/// shape, not axum's plain text, and after the gates, so it tells a stranger nothing.
#[tokio::test]
#[serial]
async fn a_body_that_does_not_parse_gets_a_structured_refusal_through_the_mount() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let token = bearer_token(&resources, None).await?;

    for bad_budget in [json!(70_000), json!(1.5), json!("many")] {
        let refused = AxumTestRequest::put("/admin/settings/guardian")
            .header("authorization", &token)
            .header("content-type", "application/json")
            .json(&json!({"max_writes_per_turn": bad_budget, "expected_updated_at": null}))
            .send(token_api(&resources))
            .await;
        assert_eq!(refused.status(), 422, "{bad_budget}");
        let refused: Value = refused.json();
        assert_eq!(refused["code"], "guardian_config_malformed");
        let message = refused["message"].as_str().unwrap();
        assert!(message.contains("was not saved"), "{message}");
        assert!(message.contains("expected u16"), "{message}");
    }

    let plain_user = AxumTestRequest::put("/admin/settings/guardian")
        .header("authorization", "Bearer not-a-token")
        .header("content-type", "application/json")
        .json(&json!({"max_writes_per_turn": 70_000}))
        .send(token_api(&resources))
        .await;
    assert_eq!(plain_user.status(), 401);

    assert_eq!(
        resources
            .agent
            .database
            .get_system_setting(GUARDIAN_CONFIG_SETTING_KEY)
            .await?
            .map(|row| row.value),
        None
    );
    Ok(())
}

/// A wrapped body once saved an empty document over the policy, and a misspelled
/// field saves as "unset": neither reaches the row now, each refused like any other
/// body that does not parse, and the refusal names what the policy does not read.
#[tokio::test]
#[serial]
async fn a_body_naming_a_field_the_policy_does_not_read_is_refused_and_writes_nothing() -> Result<()>
{
    let resources = common::create_test_server_resources().await?;
    let token = bearer_token(&resources, None).await?;

    for (body, field) in [
        (
            json!({"document": {"max_writes_per_turn": 9}, "expected_updated_at": null}),
            "document",
        ),
        (
            json!({"max_write_per_turn": 9, "expected_updated_at": null}),
            "max_write_per_turn",
        ),
        (
            json!({"plan_mode": "enforce", "expected_updated_at": null}),
            "plan_mode",
        ),
        // Retired by carnet#1471: a client still sending them learns so.
        (
            json!({"mode": "observe", "expected_updated_at": null}),
            "mode",
        ),
        (
            json!({"max_writes_per_turn": 9, "acknowledge_impact": true, "expected_updated_at": null}),
            "acknowledge_impact",
        ),
    ] {
        let refused = AxumTestRequest::put("/admin/settings/guardian")
            .header("authorization", &token)
            .header("content-type", "application/json")
            .json(&body)
            .send(token_api(&resources))
            .await;
        assert_eq!(refused.status(), 422, "{body}: {}", refused.body_text());
        let refused: Value = refused.json();
        assert_eq!(refused["code"], "guardian_config_malformed", "{body}");
        let message = refused["message"].as_str().unwrap();
        assert!(message.contains("was not saved"), "{message}");
        assert!(message.contains(field), "names {field}: {message}");
    }

    assert_eq!(
        resources
            .agent
            .database
            .get_system_setting(GUARDIAN_CONFIG_SETTING_KEY)
            .await?
            .map(|row| row.value),
        None,
        "no refusal wrote a row"
    );
    assert_eq!(
        resources
            .fitness
            .guardian_config_registry
            .current_guardian()
            .policy()
            .max_writes_per_turn,
        50,
        "and none installed a policy"
    );
    Ok(())
}
