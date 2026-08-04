// ABOUTME: Verifies group_creation_policy is a known admin config parameter
// ABOUTME: Regression guard for the "Failed to update policy" admin web bug
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used)]

use std::collections::HashMap;

use pierre_config::admin_types::ValidateConfigRequest;
use pierre_database::database::test_utils::create_sqlite_test_db;
use pierre_mcp_server::config::admin::service::AdminConfigService;

#[tokio::test]
async fn validate_accepts_group_creation_policy_admins_only() {
    let db = create_sqlite_test_db().await.unwrap();
    let pool = db.sqlite_pool().expect("test db is sqlite").clone();
    let svc = AdminConfigService::new(pool).await.unwrap();

    let mut params = HashMap::new();
    params.insert(
        "group_creation_policy".to_owned(),
        serde_json::json!("admins_only"),
    );
    let resp = svc
        .validate(&ValidateConfigRequest { parameters: params })
        .await;

    assert!(
        resp.is_valid,
        "admins_only must be a valid group_creation_policy value, got errors: {:?}",
        resp.errors
    );
}

#[tokio::test]
async fn validate_accepts_group_creation_policy_everyone() {
    let db = create_sqlite_test_db().await.unwrap();
    let pool = db.sqlite_pool().expect("test db is sqlite").clone();
    let svc = AdminConfigService::new(pool).await.unwrap();

    let mut params = HashMap::new();
    params.insert(
        "group_creation_policy".to_owned(),
        serde_json::json!("everyone"),
    );
    let resp = svc
        .validate(&ValidateConfigRequest { parameters: params })
        .await;

    assert!(
        resp.is_valid,
        "everyone must be a valid group_creation_policy value, got errors: {:?}",
        resp.errors
    );
}

#[tokio::test]
async fn validate_rejects_unknown_group_creation_policy_value() {
    let db = create_sqlite_test_db().await.unwrap();
    let pool = db.sqlite_pool().expect("test db is sqlite").clone();
    let svc = AdminConfigService::new(pool).await.unwrap();

    let mut params = HashMap::new();
    params.insert(
        "group_creation_policy".to_owned(),
        serde_json::json!("anyone_with_a_pulse"),
    );
    let resp = svc
        .validate(&ValidateConfigRequest { parameters: params })
        .await;

    assert!(!resp.is_valid, "unknown enum value must be rejected");
    assert_eq!(resp.errors.len(), 1);
    assert_eq!(resp.errors[0].parameter, "group_creation_policy");
}
