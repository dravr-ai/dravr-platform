// ABOUTME: Integration tests for per-user tool overrides (user_tool_overrides) + admin_ops tier/plan
// ABOUTME: Covers repo CRUD, UserOverride precedence vs plan/tenant/global, and per-user divergence
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::Utc;
use pierre_config::tool_selection::ToolSelectionConfig;
use pierre_core::models::{ToolEnablementSource, UserTier};
use pierre_database::backends::factory::Database;
use pierre_database::repositories::UserToolOverride;
use pierre_services::admin_ops;
use pierre_tool_runtime::tool_selection::ToolSelectionService;
use std::sync::Arc;

mod common;

/// A catalogued starter-plan tool, enabled by default (`tc-001`-adjacent rows).
const STARTER_TOOL: &str = "get_activities";
/// A catalogued tool with `min_plan = 'professional'` (tc-048), enabled by
/// default — invisible to a starter tenant unless user-overridden.
const PROFESSIONAL_TOOL: &str = "analyze_weather_impact";

fn service(db: &Arc<Database>) -> ToolSelectionService {
    ToolSelectionService::new(&Arc::new(db.repositories()))
}

fn service_with_disabled(db: &Arc<Database>, disabled: Vec<String>) -> ToolSelectionService {
    let config = ToolSelectionConfig::with_disabled_tools(disabled);
    ToolSelectionService::with_config(&Arc::new(db.repositories()), config)
}

#[tokio::test]
async fn test_repo_crud_roundtrip() {
    let db = common::create_test_database().await.unwrap();
    let repos = db.repositories();
    let (user_id, _user) = common::create_test_user(&db).await.unwrap();

    // Upsert an explicit disable with audit fields.
    let now = Utc::now();
    let row = UserToolOverride {
        user_id,
        tool_name: STARTER_TOOL.to_owned(),
        is_enabled: false,
        set_by: Some(user_id),
        reason: Some("crud test".to_owned()),
        created_at: now,
        updated_at: now,
    };
    repos.user_tool_overrides.upsert(&row).await.unwrap();

    let fetched = repos
        .user_tool_overrides
        .get(user_id, STARTER_TOOL)
        .await
        .unwrap()
        .expect("override row should exist after upsert");
    assert_eq!(fetched.tool_name, STARTER_TOOL);
    assert!(!fetched.is_enabled);
    assert_eq!(fetched.set_by, Some(user_id));
    assert_eq!(fetched.reason.as_deref(), Some("crud test"));
    let original_created_at = fetched.created_at;

    // Second override for the same user, different tool.
    let row2 = UserToolOverride {
        tool_name: PROFESSIONAL_TOOL.to_owned(),
        is_enabled: true,
        ..row.clone()
    };
    repos.user_tool_overrides.upsert(&row2).await.unwrap();
    let listed = repos
        .user_tool_overrides
        .list_for_user(user_id)
        .await
        .unwrap();
    assert_eq!(listed.len(), 2, "both overrides should be listed");

    // Re-upsert the first row flipped: is_enabled updates, created_at is preserved.
    let flipped = UserToolOverride {
        is_enabled: true,
        reason: Some("flipped".to_owned()),
        ..row
    };
    repos.user_tool_overrides.upsert(&flipped).await.unwrap();
    let refetched = repos
        .user_tool_overrides
        .get(user_id, STARTER_TOOL)
        .await
        .unwrap()
        .unwrap();
    assert!(refetched.is_enabled, "upsert should flip is_enabled");
    assert_eq!(refetched.reason.as_deref(), Some("flipped"));
    assert_eq!(
        refetched.created_at.timestamp(),
        original_created_at.timestamp(),
        "created_at must be preserved across upserts"
    );

    // Delete removes exactly the targeted row.
    let removed = repos
        .user_tool_overrides
        .delete(user_id, STARTER_TOOL)
        .await
        .unwrap();
    assert!(removed);
    assert!(repos
        .user_tool_overrides
        .get(user_id, STARTER_TOOL)
        .await
        .unwrap()
        .is_none());
    let remaining = repos
        .user_tool_overrides
        .list_for_user(user_id)
        .await
        .unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].tool_name, PROFESSIONAL_TOOL);
}

#[tokio::test]
async fn test_user_disable_hides_plan_enabled_tool() {
    let db = common::create_test_database().await.unwrap();
    let repos = db.repositories();
    let (user_id, _user, tenant_id) =
        common::create_test_user_with_plan(&db, "disable@example.com", "starter")
            .await
            .unwrap();
    let svc = service(&db);

    // Baseline: enabled for the tenant and for the user.
    assert!(svc.is_tool_enabled(tenant_id, STARTER_TOOL).await.unwrap());
    assert!(svc
        .is_tool_enabled_for_user(tenant_id, user_id, STARTER_TOOL)
        .await
        .unwrap());

    admin_ops::set_user_tool_override(&repos, user_id, STARTER_TOOL, false, None, None)
        .await
        .unwrap();

    // The user no longer sees it; the tenant-level answer is unchanged.
    assert!(!svc
        .is_tool_enabled_for_user(tenant_id, user_id, STARTER_TOOL)
        .await
        .unwrap());
    assert!(svc.is_tool_enabled(tenant_id, STARTER_TOOL).await.unwrap());

    let effective = svc
        .get_effective_tools_for_user(tenant_id, user_id)
        .await
        .unwrap();
    let tool = effective
        .iter()
        .find(|t| t.tool_name == STARTER_TOOL)
        .expect("tool should still be in the effective list");
    assert!(!tool.is_enabled);
    assert_eq!(tool.source, ToolEnablementSource::UserOverride);

    // tools/list-shaped view: the enabled set omits it by name.
    let enabled = svc
        .get_enabled_tools_for_user(tenant_id, user_id)
        .await
        .unwrap();
    assert!(
        !enabled.iter().any(|t| t.tool_name == STARTER_TOOL),
        "user-disabled tool must be absent from the enabled set"
    );
    assert!(
        !enabled.is_empty(),
        "other tools must remain enabled for the user"
    );
}

#[tokio::test]
async fn test_user_enable_exposes_plan_restricted_tool() {
    let db = common::create_test_database().await.unwrap();
    let repos = db.repositories();
    let (user_id, _user, tenant_id) =
        common::create_test_user_with_plan(&db, "enable@example.com", "starter")
            .await
            .unwrap();
    let svc = service(&db);

    // Baseline: plan-restricted for the starter tenant.
    let tenant_view = svc.get_effective_tools(tenant_id).await.unwrap();
    let restricted = tenant_view
        .iter()
        .find(|t| t.tool_name == PROFESSIONAL_TOOL)
        .expect("professional tool should be catalogued");
    assert!(!restricted.is_enabled, "starter plan must not enable it");
    assert_eq!(restricted.source, ToolEnablementSource::PlanRestriction);

    admin_ops::set_user_tool_override(
        &repos,
        user_id,
        PROFESSIONAL_TOOL,
        true,
        None,
        Some("comp grant".to_owned()),
    )
    .await
    .unwrap();

    // The user override outranks the plan restriction.
    assert!(svc
        .is_tool_enabled_for_user(tenant_id, user_id, PROFESSIONAL_TOOL)
        .await
        .unwrap());
    let effective = svc
        .get_effective_tools_for_user(tenant_id, user_id)
        .await
        .unwrap();
    let tool = effective
        .iter()
        .find(|t| t.tool_name == PROFESSIONAL_TOOL)
        .unwrap();
    assert!(tool.is_enabled);
    assert_eq!(tool.source, ToolEnablementSource::UserOverride);
}

#[tokio::test]
async fn test_global_disabled_is_immovable_by_user_override() {
    let db = common::create_test_database().await.unwrap();
    let repos = db.repositories();
    let (user_id, _user, tenant_id) =
        common::create_test_user_with_plan(&db, "global@example.com", "enterprise")
            .await
            .unwrap();
    let svc = service_with_disabled(&db, vec![STARTER_TOOL.to_owned()]);

    // A user-level enable must NOT resurrect a globally-disabled tool.
    admin_ops::set_user_tool_override(&repos, user_id, STARTER_TOOL, true, None, None)
        .await
        .unwrap();

    assert!(!svc
        .is_tool_enabled_for_user(tenant_id, user_id, STARTER_TOOL)
        .await
        .unwrap());
    let effective = svc
        .get_effective_tools_for_user(tenant_id, user_id)
        .await
        .unwrap();
    let tool = effective
        .iter()
        .find(|t| t.tool_name == STARTER_TOOL)
        .unwrap();
    assert!(!tool.is_enabled);
    assert_eq!(
        tool.source,
        ToolEnablementSource::GlobalDisabled,
        "global disable outranks the user override"
    );
}

#[tokio::test]
async fn test_two_users_same_tenant_diverge() {
    let db = common::create_test_database().await.unwrap();
    let repos = db.repositories();
    let (user_a, _ua, tenant_id) =
        common::create_test_user_with_plan(&db, "user-a@example.com", "starter")
            .await
            .unwrap();
    let (user_b, _ub) = common::create_test_user_with_email(&db, "user-b@example.com")
        .await
        .unwrap();
    let svc = service(&db);

    admin_ops::set_user_tool_override(&repos, user_a, STARTER_TOOL, false, None, None)
        .await
        .unwrap();

    // Same tenant context, per-user answers diverge.
    assert!(!svc
        .is_tool_enabled_for_user(tenant_id, user_a, STARTER_TOOL)
        .await
        .unwrap());
    assert!(svc
        .is_tool_enabled_for_user(tenant_id, user_b, STARTER_TOOL)
        .await
        .unwrap());
}

#[tokio::test]
async fn test_set_override_unknown_tool_is_not_found() {
    let db = common::create_test_database().await.unwrap();
    let repos = db.repositories();
    let (user_id, _user) = common::create_test_user(&db).await.unwrap();

    let err = admin_ops::set_user_tool_override(
        &repos,
        user_id,
        "nonexistent_tool_xyz",
        true,
        None,
        None,
    )
    .await
    .expect_err("unknown tool must be rejected");
    assert!(
        err.to_string().contains("nonexistent_tool_xyz"),
        "error should name the unknown tool, got: {err}"
    );
}

#[tokio::test]
async fn test_remove_user_tool_override_reverts_to_default() {
    let db = common::create_test_database().await.unwrap();
    let repos = db.repositories();
    let (user_id, _user, tenant_id) =
        common::create_test_user_with_plan(&db, "revert@example.com", "starter")
            .await
            .unwrap();
    let svc = service(&db);

    admin_ops::set_user_tool_override(&repos, user_id, STARTER_TOOL, false, None, None)
        .await
        .unwrap();
    assert!(!svc
        .is_tool_enabled_for_user(tenant_id, user_id, STARTER_TOOL)
        .await
        .unwrap());

    let removed = admin_ops::remove_user_tool_override(&repos, user_id, STARTER_TOOL)
        .await
        .unwrap();
    assert!(removed);

    // Back to the catalog default (enabled), with no override trace.
    assert!(svc
        .is_tool_enabled_for_user(tenant_id, user_id, STARTER_TOOL)
        .await
        .unwrap());
    let effective = svc
        .get_effective_tools_for_user(tenant_id, user_id)
        .await
        .unwrap();
    let tool = effective
        .iter()
        .find(|t| t.tool_name == STARTER_TOOL)
        .unwrap();
    assert_eq!(tool.source, ToolEnablementSource::Default);

    // Removing again reports nothing removed.
    let removed_again = admin_ops::remove_user_tool_override(&repos, user_id, STARTER_TOOL)
        .await
        .unwrap();
    assert!(!removed_again);
}

#[tokio::test]
async fn test_admin_ops_set_user_tier_writes_tier_and_marker() {
    let db = common::create_test_database().await.unwrap();
    let repos = db.repositories();
    let (user_id, user) = common::create_test_user(&db).await.unwrap();
    assert_eq!(
        user.tier,
        UserTier::Starter,
        "fixture users start on Starter"
    );

    let updated = admin_ops::set_user_tier(
        &repos,
        user_id,
        UserTier::Professional,
        Some("qa comp".to_owned()),
        None,
    )
    .await
    .unwrap();
    assert_eq!(updated.tier, UserTier::Professional);

    // The effective tier the server reads per request.
    let reloaded = repos.users.get_global(user_id).await.unwrap().unwrap();
    assert_eq!(reloaded.tier, UserTier::Professional);

    // The anti-clobber marker the billing webhook consults.
    let marker = repos
        .user_tier_overrides
        .get(user_id)
        .await
        .unwrap()
        .expect("tier override marker should exist");
    assert_eq!(marker.tier, UserTier::Professional);
    assert_eq!(marker.note.as_deref(), Some("qa comp"));

    // Clearing removes the marker but leaves the tier in place.
    let removed = admin_ops::clear_user_tier_override(&repos, user_id)
        .await
        .unwrap();
    assert!(removed);
    assert!(repos
        .user_tier_overrides
        .get(user_id)
        .await
        .unwrap()
        .is_none());
    let after_clear = repos.users.get_global(user_id).await.unwrap().unwrap();
    assert_eq!(after_clear.tier, UserTier::Professional);
}

#[tokio::test]
async fn test_admin_ops_set_tenant_plan() {
    let db = common::create_test_database().await.unwrap();
    let repos = db.repositories();
    let (_user_id, _user, tenant_id) =
        common::create_test_user_with_plan(&db, "plan@example.com", "starter")
            .await
            .unwrap();

    let updated = admin_ops::set_tenant_plan(&repos, tenant_id, "Professional")
        .await
        .unwrap();
    assert_eq!(
        updated.plan, "professional",
        "plan is normalized to lowercase"
    );

    let reloaded = repos.tenants.get_by_id(tenant_id).await.unwrap();
    assert_eq!(reloaded.plan, "professional");

    // A bogus plan is rejected before any write.
    let err = admin_ops::set_tenant_plan(&repos, tenant_id, "platinum")
        .await
        .expect_err("unknown plan must be rejected");
    assert!(
        err.to_string().contains("platinum"),
        "error should name the bad plan, got: {err}"
    );
    let unchanged = repos.tenants.get_by_id(tenant_id).await.unwrap();
    assert_eq!(unchanged.plan, "professional");
}

#[tokio::test]
async fn test_plan_change_unlocks_plan_restricted_tool() {
    let db = common::create_test_database().await.unwrap();
    let repos = db.repositories();
    let (user_id, _user, tenant_id) =
        common::create_test_user_with_plan(&db, "upgrade@example.com", "starter")
            .await
            .unwrap();
    let svc = service(&db);

    // Starter: restricted.
    assert!(!svc
        .is_tool_enabled_for_user(tenant_id, user_id, PROFESSIONAL_TOOL)
        .await
        .unwrap());

    admin_ops::set_tenant_plan(&repos, tenant_id, "professional")
        .await
        .unwrap();
    // The in-process cache would serve the stale starter computation for up to
    // the TTL; the web route busts it explicitly — mirror that here.
    svc.invalidate_tenant(tenant_id).await;

    assert!(
        svc.is_tool_enabled_for_user(tenant_id, user_id, PROFESSIONAL_TOOL)
            .await
            .unwrap(),
        "professional plan must unlock the plan-gated tool"
    );
}
