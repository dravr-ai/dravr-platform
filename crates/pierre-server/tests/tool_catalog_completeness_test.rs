// ABOUTME: The seeded tool_catalog and the ToolRegistry must name exactly the same tools
// ABOUTME: An orphaned seed row is deleted at boot, cascading away every tenant tool override
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `tool_catalog` completeness for the whole registered surface.
//!
//! `guardian::tenant_tool_enabled` treats an uncatalogued tool as always
//! enabled (`ResourceNotFound` means "no per-tenant override applies"), so a
//! tool missing from `tool_catalog` can never be disabled by a tenant — the
//! cross-tenant coach-write guard in `plan_scope.rs` was inert in production
//! for exactly that reason (carnet#143).
//!
//! The gate covers every registered tool, not the chat-callable subset, because
//! the guardian sits at the universal dispatch chokepoint: MCP-direct, A2A and
//! internal calls consult `tool_catalog` too. `chat_callable_schemas()` gates
//! only what the LLM may call mid-turn and deliberately excludes the `coaches`
//! and `admin` categories, so a tool there is invisible to a chat-callable
//! comparison.
//!
//! It compares against the catalog the **migrations** leave, opened through
//! `create_test_database` rather than a booted `ServerContext`. Booting runs
//! `sync_tool_catalog`, which inserts a row for every registered tool that has
//! none — so a comparison against a booted catalog is derived from the registry
//! it is being compared to and cannot fail whatever the seed says.
//!
//! Both directions are asserted, and the second is the one with teeth. The same
//! startup sync **deletes** every catalog row naming a tool the registry no
//! longer has, and `tenant_tool_overrides.tool_name` and
//! `user_tool_overrides.tool_name` both declare
//! `REFERENCES tool_catalog(tool_name) ON DELETE CASCADE` on both backends. So
//! a seeded row left behind by a rename is not inert: on the next boot it takes
//! every tenant's and every user's override for that tool with it, and the
//! renamed tool comes back enabled by default with nothing recording that
//! anyone had disabled it. Neither foreign key declares `ON UPDATE`, so the row
//! cannot simply be renamed in place either — a rename migration inserts the
//! new row, moves the override rows onto it, then deletes the old one.
//!
//! The last test proves the guard has teeth on production data: a
//! repository-level disable of `save_training_plan` flips `tenant_tool_enabled`
//! to `false` for that tenant and leaves every other tenant enabled.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use common::{create_test_database, create_test_server_resources, create_test_user_with_email};
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

/// Floor on the registered set so the registry half of the comparison can
/// never pass vacuously (an empty registry has no missing tools).
const MIN_REGISTERED_TOOLS: usize = 100;

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

/// The seeded `tool_catalog` and the registry name exactly the same tools.
///
/// The registry is built exactly the way the server builds it
/// (`register_builtin_tools`); the catalog is read through the repository from
/// a migrated database that has never been booted, so the rows are the ones the
/// migrations wrote and nothing else.
#[tokio::test]
async fn the_seeded_catalog_and_the_registry_name_the_same_tools() {
    let database = create_test_database().await.unwrap();

    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    let registered: BTreeSet<String> = registry
        .tool_names()
        .into_iter()
        .map(ToOwned::to_owned)
        .collect();
    assert!(
        registered.len() >= MIN_REGISTERED_TOOLS,
        "the registry collapsed to {} tools (floor {MIN_REGISTERED_TOOLS}) — \
         the completeness comparison below would be vacuous",
        registered.len()
    );

    let seeded: BTreeSet<String> = database
        .repositories()
        .tool_selection
        .get_tool_catalog()
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.tool_name)
        .collect();
    assert!(
        seeded.len() >= MIN_CATALOG_ROWS,
        "tool_catalog holds {} rows (floor {MIN_CATALOG_ROWS}) — the catalog \
         seed migrations did not run or inserted nothing",
        seeded.len()
    );

    // Name the category alongside the tool: a `coaches` or `admin` tool is the
    // case no chat-callable comparison can see, so the message says which one
    // the reader is looking at.
    let unseeded: Vec<String> = registered
        .difference(&seeded)
        .map(|name| {
            let category = registry.category_for_tool(name).unwrap_or("(none)");
            format!("{name} [category {category}]")
        })
        .collect();
    assert!(
        unseeded.is_empty(),
        "{} registered tool(s) with no seeded tool_catalog row. Until the first boot \
         syncs one in, no tenant can disable them and tenant_tool_enabled always allows \
         (carnet#143); the synthesised row then carries the LLM-facing tool description \
         as its operator display text. Seed a row on both backends (migrations/ and \
         migrations_pg/): {}",
        unseeded.len(),
        unseeded.join(", ")
    );

    let orphaned: Vec<&String> = seeded.difference(&registered).collect();
    let count = orphaned.len();
    assert!(
        orphaned.is_empty(),
        "{count} seeded tool_catalog row(s) name a tool the registry does not register. \
         sync_tool_catalog DELETES these on the next boot, and tenant_tool_overrides \
         and user_tool_overrides cascade on that delete — every tenant's and every \
         user's override for the tool goes with the row, and the tool returns enabled \
         by default. A rename must insert the new row, move the override rows onto it, \
         then delete the old row; neither foreign key declares ON UPDATE, so renaming \
         tool_name in place is not available: {orphaned:?}"
    );

    for tool in PLAN_TOOLS {
        assert!(
            seeded.contains(tool),
            "plan tool '{tool}' has no seeded tool_catalog row — the carnet#143 gap is back"
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
