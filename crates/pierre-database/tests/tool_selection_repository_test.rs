// ABOUTME: Covers the tool catalog and per-tenant tool overrides against whichever backend DATABASE_URL names
// ABOUTME: Pins the catalog's stored timestamps, the plan hierarchy, the override round trip and the enabled count
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `tool_catalog` and `tenant_tool_overrides` are written once and emitted
//! for both backends, differing only in how the override row's uuid columns
//! reach the table — native on `PostgreSQL`, `TEXT` on `SQLite`.
//!
//! The repository had no direct test on either backend. The one the catalog
//! timestamp needed most: `SQLite` used to parse `created_at` as RFC 3339
//! and, when the row had been stamped by the database's own clock in its
//! `YYYY-MM-DD HH:MM:SS` form, substitute `Utc::now()` — so every read of a
//! catalog entry reported it created just now. These run on `SQLite` and on
//! `PostgreSQL`: `create_test_db` opens whichever `DATABASE_URL` names.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use chrono::Utc;
use pierre_core::models::{Tenant, TenantId, TenantPlan, ToolCatalogEntry, ToolCategory, User};
use pierre_database::database::test_utils::create_test_db;
use pierre_database::RepositoryRegistry;
use tokio::time::sleep;
use uuid::Uuid;

/// A distinct tenant on the given plan, owned by a fresh user.
async fn fresh_tenant(repos: &RepositoryRegistry, plan: &str) -> (TenantId, Uuid) {
    let user = User::new(
        format!("tools-{}@example.com", Uuid::new_v4()),
        "argon2-hash-placeholder".to_owned(),
        Some("Tool Selection Tester".to_owned()),
    );
    let user_id = repos.users.create(&user).await.unwrap();
    let tenant = Tenant::new(
        "Tool Tenant".to_owned(),
        format!("tools-{}", Uuid::new_v4()),
        None,
        plan.to_owned(),
        user_id,
    );
    repos.tenants.create(&tenant).await.unwrap();
    (tenant.id, user_id)
}

/// A catalog entry under a name no other test uses.
fn entry(category: ToolCategory, min_plan: TenantPlan, enabled: bool) -> ToolCatalogEntry {
    let name = format!("test_tool_{}", Uuid::new_v4().simple());
    ToolCatalogEntry {
        id: name.clone(),
        tool_name: name,
        display_name: "A test tool".to_owned(),
        description: "Exists for the repository test".to_owned(),
        category,
        is_enabled_by_default: enabled,
        requires_provider: None,
        min_plan,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[tokio::test]
async fn a_catalog_entry_reads_back_with_the_moment_it_was_stored() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let repo = &repos.tool_selection;

    let before = Utc::now();
    let entry = entry(ToolCategory::Analysis, TenantPlan::Professional, true);
    repo.upsert_tool_catalog_entry(&entry).await.unwrap();

    let first = repo
        .get_tool_catalog_entry(&entry.tool_name)
        .await
        .unwrap()
        .expect("the entry just written reads back");
    assert_eq!(first.tool_name, entry.tool_name);
    assert_eq!(first.category, ToolCategory::Analysis);
    assert_eq!(first.min_plan, TenantPlan::Professional);
    assert!(first.is_enabled_by_default);
    assert!(
        first.created_at >= before - chrono::Duration::seconds(1) && first.created_at <= Utc::now(),
        "created_at is the database's clock at the write: {}",
        first.created_at
    );

    sleep(Duration::from_millis(1100)).await;
    let second = repo
        .get_tool_catalog_entry(&entry.tool_name)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        second.created_at, first.created_at,
        "created_at is stored, not recomputed on every read"
    );

    let mut renamed = entry.clone();
    renamed.display_name = "Renamed".to_owned();
    renamed.is_enabled_by_default = false;
    repo.upsert_tool_catalog_entry(&renamed).await.unwrap();
    let updated = repo
        .get_tool_catalog_entry(&entry.tool_name)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.display_name, "Renamed");
    assert!(!updated.is_enabled_by_default);
    assert_eq!(
        updated.created_at, first.created_at,
        "an upsert keeps the original created_at"
    );
    assert!(updated.updated_at >= first.updated_at);

    assert!(repo
        .delete_tool_catalog_entry(&entry.tool_name)
        .await
        .unwrap());
    assert!(!repo
        .delete_tool_catalog_entry(&entry.tool_name)
        .await
        .unwrap());
}

#[tokio::test]
async fn the_plan_hierarchy_bounds_what_a_plan_may_use() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let repo = &repos.tool_selection;

    let starter = entry(ToolCategory::Fitness, TenantPlan::Starter, true);
    let professional = entry(ToolCategory::Fitness, TenantPlan::Professional, true);
    let enterprise = entry(ToolCategory::Fitness, TenantPlan::Enterprise, true);
    for e in [&starter, &professional, &enterprise] {
        repo.upsert_tool_catalog_entry(e).await.unwrap();
    }
    let names = |entries: Vec<ToolCatalogEntry>| {
        entries
            .into_iter()
            .map(|e| e.tool_name)
            .filter(|n| {
                n == &starter.tool_name
                    || n == &professional.tool_name
                    || n == &enterprise.tool_name
            })
            .collect::<Vec<_>>()
    };

    let for_starter = names(
        repo.get_tools_by_min_plan(TenantPlan::Starter)
            .await
            .unwrap(),
    );
    assert_eq!(for_starter, vec![starter.tool_name.clone()]);

    let mut for_professional = names(
        repo.get_tools_by_min_plan(TenantPlan::Professional)
            .await
            .unwrap(),
    );
    for_professional.sort();
    let mut expected = vec![starter.tool_name.clone(), professional.tool_name.clone()];
    expected.sort();
    assert_eq!(for_professional, expected);

    let for_enterprise = names(
        repo.get_tools_by_min_plan(TenantPlan::Enterprise)
            .await
            .unwrap(),
    );
    assert_eq!(
        for_enterprise.len(),
        3,
        "an enterprise plan may use every tool"
    );

    let fitness = names(
        repo.get_tools_by_category(ToolCategory::Fitness)
            .await
            .unwrap(),
    );
    assert_eq!(fitness.len(), 3);
    let analysis = names(
        repo.get_tools_by_category(ToolCategory::Analysis)
            .await
            .unwrap(),
    );
    assert!(analysis.is_empty());
}

#[tokio::test]
async fn an_override_round_trips_and_counts_toward_the_tenant() {
    let db = create_test_db().await.unwrap();
    let repos = db.repositories();
    let repo = &repos.tool_selection;
    let (tenant_id, operator) = fresh_tenant(&repos, "starter").await;

    let on_by_default = entry(ToolCategory::Goals, TenantPlan::Starter, true);
    let off_by_default = entry(ToolCategory::Goals, TenantPlan::Starter, false);
    let out_of_plan = entry(ToolCategory::Goals, TenantPlan::Enterprise, true);
    for e in [&on_by_default, &off_by_default, &out_of_plan] {
        repo.upsert_tool_catalog_entry(e).await.unwrap();
    }
    let baseline = repo.count_enabled_tools(tenant_id).await.unwrap();

    assert!(repo
        .get_override(tenant_id, &off_by_default.tool_name)
        .await
        .unwrap()
        .is_none());

    let stored = repo
        .upsert_override(
            tenant_id,
            &off_by_default.tool_name,
            true,
            Some(operator),
            Some("switched on for the pilot".to_owned()),
        )
        .await
        .unwrap();
    assert_eq!(stored.tenant_id, tenant_id);
    assert_eq!(stored.tool_name, off_by_default.tool_name);
    assert!(stored.is_enabled);
    assert_eq!(stored.enabled_by_user_id, Some(operator));
    assert_eq!(stored.reason.as_deref(), Some("switched on for the pilot"));

    let again = repo
        .upsert_override(tenant_id, &off_by_default.tool_name, false, None, None)
        .await
        .unwrap();
    assert_eq!(again.id, stored.id, "the upsert keeps the row's id");
    assert!(!again.is_enabled);
    assert_eq!(again.enabled_by_user_id, None);
    assert_eq!(again.reason, None);

    repo.upsert_override(tenant_id, &off_by_default.tool_name, true, None, None)
        .await
        .unwrap();
    repo.upsert_override(tenant_id, &on_by_default.tool_name, false, None, None)
        .await
        .unwrap();
    let overrides = repo.get_overrides(tenant_id).await.unwrap();
    assert_eq!(overrides.len(), 2);
    assert_eq!(
        repo.count_enabled_tools(tenant_id).await.unwrap(),
        baseline,
        "one tool switched on and one switched off leave the count where it was"
    );

    assert!(repo
        .delete_override(tenant_id, &on_by_default.tool_name)
        .await
        .unwrap());
    assert_eq!(
        repo.count_enabled_tools(tenant_id).await.unwrap(),
        baseline + 1,
        "reverting the switched-off tool to its default counts it again"
    );
    assert!(!repo
        .delete_override(tenant_id, &on_by_default.tool_name)
        .await
        .unwrap());
}
