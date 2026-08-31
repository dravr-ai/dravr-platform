// ABOUTME: Every chat-callable tool must have a tool_catalog row so a tenant can disable it
// ABOUTME: Proves guardian::tenant_tool_enabled refuses per-tenant for the plan tools (carnet#143)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `tool_catalog` completeness for the chat-callable surface.
//!
//! `guardian::tenant_tool_enabled` treats an uncatalogued tool as always
//! enabled (`ResourceNotFound` means "no per-tenant override applies"), so a
//! chat-callable tool missing from `tool_catalog` can never be disabled by a
//! tenant — the cross-tenant coach-write guard in `plan_scope.rs` was inert in
//! production for exactly that reason (carnet#143). The first test pins the
//! whole chat-callable set against the migrated catalog; the second proves the
//! guard has teeth on production data: a repository-level disable of
//! `save_training_plan` flips `tenant_tool_enabled` to `false` for that tenant
//! and leaves every other tenant enabled.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use common::{create_test_server_resources, create_test_user_with_email};
use pierre_core::models::{TenantId, TenantPlan};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::guardian::tenant_tool_enabled;
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::runtime::ToolRuntime;
use std::collections::BTreeSet;
use std::sync::Arc;
use uuid::Uuid;

/// The three training-plan tools carnet#143 is about — asserted by name so the
/// failure message points at the exact regression, not just a set difference.
const PLAN_TOOLS: [&str; 3] = [
    "get_training_plan",
    "save_training_plan",
    "push_training_plan",
];

/// Floor on the chat-callable set so the registry half of the comparison can
/// never pass vacuously (an empty registry has no missing tools).
const MIN_CHAT_CALLABLE_TOOLS: usize = 50;

/// Floor on the migrated catalog so a stub migration that inserts nothing
/// fails on content, not just on the set difference.
const MIN_CATALOG_ROWS: usize = 100;

/// A user plus the starter-plan tenant `create_test_user_with_email` creates
/// for them (owner enrolment included).
async fn seed_user_with_tenant(resources: &Arc<ServerContext>, label: &str) -> (Uuid, TenantId) {
    let email = format!("{label}-{}@example.com", Uuid::new_v4());
    let (user_id, _user) = create_test_user_with_email(&resources.coach.database, &email)
        .await
        .unwrap();
    let tenants = resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .unwrap();
    let tenant_id = tenants.first().expect("user owns a tenant").id;
    (user_id, tenant_id)
}

/// Every chat-callable tool name has a `tool_catalog` row after migrations.
///
/// The registry is built exactly the way the server builds it
/// (`register_builtin_tools`), and the catalog is read back through the
/// repository from a freshly migrated database — so a chat-callable tool added
/// without a catalog seed row fails here with its name in the message.
#[tokio::test]
async fn every_chat_callable_tool_has_a_catalog_row() {
    let resources = create_test_server_resources().await.unwrap();

    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    let chat_callable: BTreeSet<String> = registry
        .chat_callable_schemas()
        .into_iter()
        .map(|schema| schema.name)
        .collect();
    assert!(
        chat_callable.len() >= MIN_CHAT_CALLABLE_TOOLS,
        "chat-callable surface collapsed to {} tools (floor {MIN_CHAT_CALLABLE_TOOLS}) — \
         the completeness comparison below would be vacuous",
        chat_callable.len()
    );

    let catalog: BTreeSet<String> = resources
        .common
        .repos
        .tool_selection
        .get_tool_catalog()
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.tool_name)
        .collect();
    assert!(
        catalog.len() >= MIN_CATALOG_ROWS,
        "tool_catalog holds {} rows (floor {MIN_CATALOG_ROWS}) — the chat-callable \
         seed migration did not run or inserted nothing",
        catalog.len()
    );

    let missing: Vec<&String> = chat_callable
        .iter()
        .filter(|name| !catalog.contains(*name))
        .collect();
    assert!(
        missing.is_empty(),
        "chat-callable tools without a tool_catalog row — no tenant can disable them, \
         so tenant_tool_enabled can never refuse (carnet#143): {missing:?}"
    );

    for tool in PLAN_TOOLS {
        assert!(
            catalog.contains(tool),
            "plan tool '{tool}' has no tool_catalog row — the carnet#143 gap is back"
        );
    }
}

/// The seeded plan-tool rows keep today's availability.
///
/// Enabled by default, no provider requirement, and `starter` so no tenant
/// loses a tool it could always call while it was uncatalogued.
#[tokio::test]
async fn plan_tool_catalog_rows_preserve_default_availability() {
    let resources = create_test_server_resources().await.unwrap();

    for tool in PLAN_TOOLS {
        let entry = resources
            .common
            .repos
            .tool_selection
            .get_tool_catalog_entry(tool)
            .await
            .unwrap()
            .unwrap_or_else(|| panic!("'{tool}' has no tool_catalog row"));
        assert!(
            entry.is_enabled_by_default,
            "'{tool}' must stay enabled by default — it was always-on while uncatalogued"
        );
        assert_eq!(
            entry.min_plan,
            TenantPlan::Starter,
            "'{tool}' must be min_plan starter — a higher plan gate would REMOVE the tool \
             from tenants that could always call it while it was uncatalogued"
        );
        assert!(
            entry.requires_provider.is_none(),
            "'{tool}' must not require a provider — comparable catalog rows carry NULL"
        );
    }
}

/// Disabling `save_training_plan` for one tenant flips the guardian gate.
///
/// `tenant_tool_enabled` answers `false` for the disabling tenant and stays
/// `true` for another tenant — the carnet#143 guard refusing on production
/// data, not on a test-seeded catalog row.
#[tokio::test]
async fn a_tenant_disable_reaches_the_guardian_gate() {
    let resources = create_test_server_resources().await.unwrap();
    let (admin_a, tenant_a) = seed_user_with_tenant(&resources, "catalog-a").await;
    let (_admin_b, tenant_b) = seed_user_with_tenant(&resources, "catalog-b").await;
    let runtime: Arc<dyn ToolRuntime> = resources.clone();

    // The service resolves the tool through the catalog row the migration
    // seeded: Ok(true), not Err(ResourceNotFound). Guardian's `true` for an
    // uncatalogued tool would be indistinguishable at the gate, so this is
    // the assertion that the row exists.
    for tool in PLAN_TOOLS {
        let enabled = resources
            .tool_selection()
            .is_tool_enabled(tenant_a, tool)
            .await
            .unwrap_or_else(|e| panic!("'{tool}' must be catalogued (got a lookup error: {e})"));
        assert!(
            enabled,
            "'{tool}' must resolve as catalogued-and-enabled for a fresh tenant"
        );
        assert!(
            tenant_tool_enabled(&runtime, tenant_a, tool).await,
            "'{tool}' must start enabled for a fresh tenant"
        );
    }

    // A tenant admin disables the tool through the same service the admin
    // console uses. Before the migration this call itself failed with
    // "Tool 'save_training_plan' not found" — no tenant could record the
    // disable at all.
    resources
        .tool_selection()
        .set_tool_override(tenant_a, "save_training_plan", false, admin_a, None)
        .await
        .expect("disabling save_training_plan requires its tool_catalog row");

    assert!(
        !tenant_tool_enabled(&runtime, tenant_a, "save_training_plan").await,
        "the disabling tenant must be refused — the guard was inert without a catalog row"
    );
    assert!(
        tenant_tool_enabled(&runtime, tenant_b, "save_training_plan").await,
        "another tenant's enablement must be untouched by tenant A's override"
    );
    assert!(
        tenant_tool_enabled(&runtime, tenant_a, "get_training_plan").await,
        "only the overridden tool is disabled — its siblings stay enabled"
    );
}
