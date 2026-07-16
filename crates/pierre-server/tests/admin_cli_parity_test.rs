// ABOUTME: Integration tests for the CLI admin-parity surface (approve/suspend/reset, rate-limit,
// ABOUTME: feature flags, tenant tool overrides) — exercises the shared admin_ops/service fns
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_core::feature_flags::FeatureKey;
use pierre_core::models::{ToolEnablementSource, UserStatus};
use pierre_services::admin_ops;
use pierre_tool_runtime::tool_selection::ToolSelectionService;
use std::sync::Arc;

mod common;

#[tokio::test]
async fn test_rate_limit_override_set_and_clear() {
    let db = common::create_test_database().await.unwrap();
    let repos = db.repositories();
    let (user_id, _u) = common::create_test_user(&db).await.unwrap();
    let (admin_id, _a) = common::create_test_user_with_email(&db, "rl-admin@example.com")
        .await
        .unwrap();

    admin_ops::set_user_rate_limit_override(
        &repos,
        user_id,
        Some(500),
        None,
        Some("cli parity test".to_owned()),
        Some(admin_id),
    )
    .await
    .unwrap();

    let row = repos
        .user_rate_limit_overrides
        .get(user_id)
        .await
        .unwrap()
        .expect("override row should exist");
    assert_eq!(row.daily_limit, Some(500));
    assert_eq!(row.monthly_limit, None, "omitted dimension is unlimited");
    assert_eq!(row.note.as_deref(), Some("cli parity test"));
    assert_eq!(row.set_by, Some(admin_id));

    // Zero caps are rejected before any write.
    let err = admin_ops::set_user_rate_limit_override(&repos, user_id, Some(0), None, None, None)
        .await
        .expect_err("zero daily cap must be rejected");
    assert!(err.to_string().contains("positive"), "got: {err}");

    // Unknown users are a clean not-found.
    let missing = uuid::Uuid::new_v4();
    let err = admin_ops::set_user_rate_limit_override(&repos, missing, Some(10), None, None, None)
        .await
        .expect_err("unknown user must be rejected");
    assert!(err.to_string().contains("not found"), "got: {err}");

    // Clear removes the row exactly once.
    assert!(admin_ops::clear_user_rate_limit_override(&repos, user_id)
        .await
        .unwrap());
    assert!(repos
        .user_rate_limit_overrides
        .get(user_id)
        .await
        .unwrap()
        .is_none());
    assert!(!admin_ops::clear_user_rate_limit_override(&repos, user_id)
        .await
        .unwrap());
}

#[tokio::test]
async fn test_approve_and_suspend_transition_status_with_audit() {
    let db = common::create_test_database().await.unwrap();
    let repos = db.repositories();
    let (user_id, _u) = common::create_test_user_with_email(&db, "pending@example.com")
        .await
        .unwrap();
    let (admin_id, _a) = common::create_test_user_with_email(&db, "approver@example.com")
        .await
        .unwrap();

    // Fixture users start Active; park this one in Pending like a fresh signup.
    repos
        .users
        .update_status(user_id, UserStatus::Pending, None)
        .await
        .unwrap();

    let approved =
        admin_ops::transition_user_status(&repos, user_id, UserStatus::Active, Some(admin_id))
            .await
            .unwrap();
    assert_eq!(approved.user_status, UserStatus::Active);
    assert_eq!(approved.approved_by, Some(admin_id), "audit actor recorded");

    let suspended =
        admin_ops::transition_user_status(&repos, user_id, UserStatus::Suspended, Some(admin_id))
            .await
            .unwrap();
    assert_eq!(suspended.user_status, UserStatus::Suspended);
    assert!(!suspended.user_status.can_login());

    // Idempotence guard: re-suspending an already-suspended user is rejected.
    let err =
        admin_ops::transition_user_status(&repos, user_id, UserStatus::Suspended, Some(admin_id))
            .await
            .expect_err("double suspend must be rejected");
    assert!(err.to_string().contains("already"), "got: {err}");
}

#[tokio::test]
async fn test_password_reset_token_issue() {
    let db = common::create_test_database().await.unwrap();
    let repos = db.repositories();
    let (user_id, _u) = common::create_test_user(&db).await.unwrap();

    let token = admin_ops::issue_password_reset_token(&repos, user_id, "pierre-cli")
        .await
        .unwrap();

    // Selector/verifier shape: "<selector>.<verifier>", both halves non-trivial.
    let (selector, verifier) = token
        .split_once('.')
        .expect("token must be selector.verifier");
    assert!(selector.len() >= 8, "selector too short: {selector}");
    assert!(verifier.len() >= 16, "verifier too short");
    // Compile-time policy pin (clippy::assertions_on_constants forbids the
    // runtime form): a reset link must live at least 10 minutes.
    const { assert!(admin_ops::PASSWORD_RESET_TTL_SECONDS >= 600) };
}

#[tokio::test]
async fn test_feature_flag_user_and_tenant_round_trip() {
    let db = common::create_test_database().await.unwrap();
    let repos = db.repositories();
    let (user_id, _u, tenant_id) =
        common::create_test_user_with_plan(&db, "flags@example.com", "starter")
            .await
            .unwrap();
    let key = FeatureKey::ApiTokens;

    // Per-user override round trip.
    repos
        .feature_flags
        .set_user_override(user_id, key, true, None)
        .await
        .unwrap();
    let user_rows = repos
        .feature_flags
        .list_user_overrides(user_id)
        .await
        .unwrap();
    assert_eq!(user_rows.len(), 1);
    assert_eq!(user_rows[0].feature_key, key);
    assert!(user_rows[0].enabled);
    assert!(repos
        .feature_flags
        .clear_user_override(user_id, key)
        .await
        .unwrap());
    assert!(repos
        .feature_flags
        .list_user_overrides(user_id)
        .await
        .unwrap()
        .is_empty());

    // Tenant-default round trip.
    let tenant_uuid = tenant_id.as_uuid();
    repos
        .feature_flags
        .set_tenant_default(tenant_uuid, key, true, None)
        .await
        .unwrap();
    let tenant_rows = repos
        .feature_flags
        .list_tenant_defaults(tenant_uuid)
        .await
        .unwrap();
    assert_eq!(tenant_rows.len(), 1);
    assert!(tenant_rows[0].enabled);
    assert!(repos
        .feature_flags
        .clear_tenant_default(tenant_uuid, key)
        .await
        .unwrap());
}

#[tokio::test]
async fn test_tenant_tool_override_via_service() {
    let db = common::create_test_database().await.unwrap();
    let (_user_id, admin, tenant_id) =
        common::create_test_user_with_plan(&db, "tenant-tools@example.com", "enterprise")
            .await
            .unwrap();
    let repos = Arc::new(db.repositories());
    let svc = ToolSelectionService::new(&repos);

    // Baseline: enabled by catalog default.
    assert!(svc
        .is_tool_enabled(tenant_id, "get_activities")
        .await
        .unwrap());

    // Tenant-wide disable through the same write path web/token/CLI share.
    svc.set_tool_override(
        tenant_id,
        "get_activities",
        false,
        admin.id,
        Some("cli parity test".to_owned()),
    )
    .await
    .unwrap();
    assert!(!svc
        .is_tool_enabled(tenant_id, "get_activities")
        .await
        .unwrap());
    let effective = svc.get_effective_tools(tenant_id).await.unwrap();
    let tool = effective
        .iter()
        .find(|t| t.tool_name == "get_activities")
        .unwrap();
    assert_eq!(tool.source, ToolEnablementSource::TenantOverride);

    // Unknown tools are a clean not-found.
    let err = svc
        .set_tool_override(tenant_id, "nonexistent_tool_xyz", true, admin.id, None)
        .await
        .expect_err("unknown tool must be rejected");
    assert!(
        err.to_string().contains("nonexistent_tool_xyz"),
        "got: {err}"
    );

    // Reset reverts to catalog default.
    assert!(svc
        .remove_tool_override(tenant_id, "get_activities")
        .await
        .unwrap());
    assert!(svc
        .is_tool_enabled(tenant_id, "get_activities")
        .await
        .unwrap());
}
