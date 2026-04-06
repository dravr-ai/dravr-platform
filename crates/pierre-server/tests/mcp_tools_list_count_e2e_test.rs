// ABOUTME: Strict E2E regression tests for MCP tools/list exact tool counts per auth tier.
// ABOUTME: Spawns real HTTP server, asserts exact tool-set parity to prevent silent tool loss.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// Why this exists
// ===============
// We shipped a regression where MCP `tools/list` responses dropped from 80+ tools to 15
// without any existing E2E test failing. The pre-existing e2e suites either:
//   - mocked the tool list in-process (mcp_comprehensive_client_e2e_test.rs),
//   - asserted only `!tools.is_empty()` (mcp_sse_real_server_e2e_test.rs), or
//   - used loose `>=` comparisons between auth tiers (routes_mcp_http_test.rs).
//
// None of them pinned the *exact* public set, nor asserted a count floor for the
// authenticated tier. This file closes that gap by driving a real HTTP MCP server
// end-to-end and comparing the wire response against concrete tool sets/thresholds.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use anyhow::Result;
use pierre_database::plugins::factory::Database;
use pierre_mcp_server::{
    constants::tools::PUBLIC_DISCOVERY_TOOLS, mcp::resources::ServerResources,
    tools::registry::ToolRegistry,
};
use reqwest::Client;
use serde_json::{json, Value};
use std::{collections::HashSet, sync::Arc};
use uuid::Uuid;

/// Floor for the admin/owner authenticated tool set. Used to catch silent regressions.
///
/// The full registry currently exposes 73+ tools (see `mcp_tools_unit_test::test_total_tool_count`).
/// This threshold sits well below that to avoid flaking on legitimate additions while still
/// catching catastrophic drops (e.g. the 15-tool regression).
const MIN_AUTHENTICATED_TOOLS: usize = 60;

/// Floor for a non-admin tenant member on a freshly seeded tenant (empty tool catalog).
///
/// Non-admin members flow through `tenant_filtered_tools` which combines catalog-enabled
/// tools with feature-flag "uncatalogued" tools. An empty catalog must still yield a
/// healthy set via the uncatalogued merge. This threshold is the exact regression
/// guard: when the merge broke, this set collapsed to ~15 tools.
const MIN_MEMBER_TOOLS: usize = 40;

/// Add an existing user to a tenant as a Member in the `tenant_users` junction table.
///
/// We cannot just downgrade the tenant owner's role because `list_for_user` joins
/// against a row with `role='owner'` — removing the last owner makes the tenant
/// invisible. Instead we add a second user with role='member' to an existing
/// owner-backed tenant, so both the owner and member rows coexist.
async fn add_user_to_tenant_as_member(
    resources: &Arc<ServerResources>,
    tenant_id: &str,
    user_id: Uuid,
) -> Result<()> {
    match &*resources.database {
        Database::SQLite(db) => {
            let row_id = Uuid::new_v4().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            sqlx::query(
                "INSERT INTO tenant_users \
                 (id, tenant_id, user_id, role, invited_at, joined_at) \
                 VALUES (?, ?, ?, 'member', ?, ?)",
            )
            .bind(&row_id)
            .bind(tenant_id)
            .bind(user_id.to_string())
            .bind(&now)
            .bind(&now)
            .execute(db.pool())
            .await?;
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(_) => {
            anyhow::bail!(
                "tenant-member regression test only runs against SQLite (test default). \
                 Postgres backend is covered by sqlite-equivalent semantics."
            );
        }
    }
    Ok(())
}

/// POST a JSON-RPC `tools/list` request to the MCP HTTP endpoint.
async fn post_tools_list(
    client: &Client,
    base_url: &str,
    auth_header: Option<&str>,
) -> Result<Value> {
    let mut req = client
        .post(format!("{base_url}/mcp"))
        .header("Content-Type", "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }));
    if let Some(header) = auth_header {
        req = req.header("authorization", header);
    }
    let response = req.send().await?;
    assert_eq!(
        response.status().as_u16(),
        200,
        "tools/list should return 200"
    );
    let body: Value = response.json().await?;
    Ok(body)
}

/// Extract the `result.tools` array as a set of tool names.
fn extract_tool_names(body: &Value) -> HashSet<String> {
    body["result"]["tools"]
        .as_array()
        .expect("tools/list response must contain result.tools array")
        .iter()
        .filter_map(|t| t["name"].as_str().map(ToOwned::to_owned))
        .collect()
}

/// Validate that every tool entry has a valid, non-empty schema shape.
fn assert_every_tool_has_valid_schema(body: &Value) {
    let tools = body["result"]["tools"]
        .as_array()
        .expect("result.tools must be an array");
    for tool in tools {
        let name = tool["name"].as_str().expect("tool must have a string name");
        assert!(!name.is_empty(), "tool name must not be empty");
        let desc = tool["description"]
            .as_str()
            .unwrap_or_else(|| panic!("tool {name} must have a string description"));
        assert!(
            desc.len() >= 10,
            "tool {name} description too short ({} chars): {desc}",
            desc.len()
        );
        let schema = tool
            .get("inputSchema")
            .unwrap_or_else(|| panic!("tool {name} must have an inputSchema"));
        assert!(
            schema.is_object(),
            "tool {name} inputSchema must be an object"
        );
        assert_eq!(
            schema["type"].as_str(),
            Some("object"),
            "tool {name} inputSchema.type must be 'object'"
        );
    }
}

/// Unauthenticated `tools/list` MUST return exactly the `PUBLIC_DISCOVERY_TOOLS` set.
///
/// Regression: an empty/partial catalog silently collapsed the public set. This test
/// pins the exact names so any drop (or accidental addition) is detected immediately.
#[tokio::test]
async fn test_tools_list_unauthenticated_exact_public_set() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let server = common::spawn_http_mcp_server(&resources).await?;
    let client = Client::new();

    let body = post_tools_list(&client, &server.base_url(), None).await?;
    let returned: HashSet<String> = extract_tool_names(&body);
    let expected: HashSet<String> = PUBLIC_DISCOVERY_TOOLS
        .iter()
        .map(|s| (*s).to_owned())
        .collect();

    let missing: Vec<&String> = expected.difference(&returned).collect();
    let extra: Vec<&String> = returned.difference(&expected).collect();

    assert!(
        missing.is_empty() && extra.is_empty(),
        "Unauthenticated tools/list drifted from PUBLIC_DISCOVERY_TOOLS.\n\
         Expected: {} tools, returned: {} tools.\n\
         Missing from response: {missing:?}\n\
         Extra in response (leak): {extra:?}",
        expected.len(),
        returned.len()
    );

    assert_every_tool_has_valid_schema(&body);

    println!(
        "✓ Unauthenticated tools/list: exact match, {} tools",
        returned.len()
    );
    Ok(())
}

/// Invalid bearer token MUST fall back to the public discovery set (same as unauth).
#[tokio::test]
async fn test_tools_list_invalid_token_falls_back_to_exact_public_set() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let server = common::spawn_http_mcp_server(&resources).await?;
    let client = Client::new();

    let body = post_tools_list(&client, &server.base_url(), Some("Bearer bogus_token_xyz")).await?;
    let returned = extract_tool_names(&body);
    let expected: HashSet<String> = PUBLIC_DISCOVERY_TOOLS
        .iter()
        .map(|s| (*s).to_owned())
        .collect();

    assert_eq!(
        returned, expected,
        "Invalid-token tools/list must match PUBLIC_DISCOVERY_TOOLS exactly"
    );

    println!(
        "✓ Invalid-token tools/list falls back cleanly: {} tools",
        returned.len()
    );
    Ok(())
}

/// Authenticated tenant owner MUST see the full authenticated tool set including admin tools.
///
/// This covers the regression path: when the `tenant_filtered`/admin branch returned a
/// depleted set (e.g. 15 tools from an empty catalog), this assertion would fail.
#[tokio::test]
async fn test_tools_list_authenticated_owner_exceeds_floor_and_superset_of_public() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let (_user, token) =
        common::create_test_tenant(&resources, "owner-tools-list@example.com").await?;
    let server = common::spawn_http_mcp_server(&resources).await?;
    let client = Client::new();

    let body = post_tools_list(
        &client,
        &server.base_url(),
        Some(&format!("Bearer {token}")),
    )
    .await?;
    let returned = extract_tool_names(&body);

    assert!(
        returned.len() >= MIN_AUTHENTICATED_TOOLS,
        "Authenticated tool count {} below regression floor {MIN_AUTHENTICATED_TOOLS}. \
         If this is intentional, update MIN_AUTHENTICATED_TOOLS. \
         Returned: {:?}",
        returned.len(),
        returned
    );

    // Admin users (tenant owner) must see admin_* tools.
    let admin_tools: Vec<&String> = returned
        .iter()
        .filter(|n| n.starts_with("admin_"))
        .collect();
    assert!(
        !admin_tools.is_empty(),
        "Tenant owner must see admin_* tools in tools/list, got none"
    );

    // Public discovery set must be a strict subset of the authenticated set.
    let public: HashSet<String> = PUBLIC_DISCOVERY_TOOLS
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let missing_public: Vec<&String> = public.difference(&returned).collect();
    assert!(
        missing_public.is_empty(),
        "Authenticated set must contain every PUBLIC_DISCOVERY_TOOL, missing: {missing_public:?}"
    );

    // Sensitive non-public tools that must only appear once authenticated.
    for expected_auth_only in &[
        "connect_provider",
        "disconnect_provider",
        "set_goal",
        "save_recipe",
    ] {
        assert!(
            returned.contains(*expected_auth_only),
            "Authenticated owner is missing sensitive tool '{expected_auth_only}'"
        );
    }

    assert_every_tool_has_valid_schema(&body);

    println!(
        "✓ Authenticated owner tools/list: {} tools ({} admin_*)",
        returned.len(),
        admin_tools.len()
    );
    Ok(())
}

/// HTTP parity: `POST /mcp` (admin) and `GET /mcp/tools` must expose the same tool set.
///
/// `GET /mcp/tools` is the discovery endpoint that bypasses the tiered visibility logic
/// and returns `tool_registry.all_schemas()`. An admin caller going through JSON-RPC
/// `tools/list` must receive the same set — otherwise a branch has diverged from the
/// registry source of truth.
#[tokio::test]
async fn test_tools_list_admin_matches_registry_discovery_endpoint() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let (_user, token) = common::create_test_tenant(&resources, "parity-owner@example.com").await?;
    let server = common::spawn_http_mcp_server(&resources).await?;
    let client = Client::new();

    // Admin path via JSON-RPC.
    let rpc_body = post_tools_list(
        &client,
        &server.base_url(),
        Some(&format!("Bearer {token}")),
    )
    .await?;
    let rpc_tools = extract_tool_names(&rpc_body);

    // Discovery endpoint (no tiered filtering).
    let discovery_resp = client
        .get(format!("{}/mcp/tools", server.base_url()))
        .send()
        .await?;
    assert_eq!(discovery_resp.status().as_u16(), 200);
    let discovery_body: Value = discovery_resp.json().await?;
    let discovery_tools: HashSet<String> = discovery_body["tools"]
        .as_array()
        .expect("GET /mcp/tools must return tools array")
        .iter()
        .filter_map(|t| t["name"].as_str().map(ToOwned::to_owned))
        .collect();

    let only_rpc: Vec<&String> = rpc_tools.difference(&discovery_tools).collect();
    let only_discovery: Vec<&String> = discovery_tools.difference(&rpc_tools).collect();

    assert!(
        only_rpc.is_empty() && only_discovery.is_empty(),
        "tools/list (admin) and GET /mcp/tools diverged.\n\
         Only in JSON-RPC response: {only_rpc:?}\n\
         Only in discovery endpoint: {only_discovery:?}"
    );

    println!(
        "✓ JSON-RPC admin tools/list == GET /mcp/tools ({} tools)",
        rpc_tools.len()
    );
    Ok(())
}

/// The on-the-wire HTTP response must match what an in-process `ToolRegistry` exposes.
///
/// This is the ultimate regression guard: if the HTTP server and a fresh registry
/// disagree, a transport layer is silently dropping tools.
#[tokio::test]
async fn test_tools_list_http_matches_in_process_registry() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let (_user, token) = common::create_test_tenant(&resources, "wire-parity@example.com").await?;
    let server = common::spawn_http_mcp_server(&resources).await?;
    let client = Client::new();

    // Wire: admin JSON-RPC tools/list.
    let body = post_tools_list(
        &client,
        &server.base_url(),
        Some(&format!("Bearer {token}")),
    )
    .await?;
    let wire: HashSet<String> = extract_tool_names(&body);

    // In-process: rebuild a registry and compare.
    let mut registry = ToolRegistry::new();
    registry.register_builtin_tools();
    let in_process: HashSet<String> = registry.all_schemas().into_iter().map(|s| s.name).collect();

    let only_wire: Vec<&String> = wire.difference(&in_process).collect();
    let only_reg: Vec<&String> = in_process.difference(&wire).collect();

    assert!(
        only_wire.is_empty() && only_reg.is_empty(),
        "HTTP tools/list and in-process ToolRegistry diverged ({} wire vs {} registry).\n\
         Only on the wire: {only_wire:?}\n\
         Only in registry: {only_reg:?}",
        wire.len(),
        in_process.len()
    );

    println!(
        "✓ HTTP tools/list matches in-process registry exactly ({} tools)",
        wire.len()
    );
    Ok(())
}

/// Non-admin tenant member on an empty-catalog tenant — the actual regression path.
///
/// When the tenant's `tool_catalog` is empty/unseeded, `tenant_filtered_tools()` must
/// still return a meaningful set via the `uncatalogued_user_schemas()` merge (feature-flag
/// tools like coaches, mobility, nutrition, etc.). The regression that shipped collapsed
/// this path to ~15 tools; this test pins a floor so that collapse fails CI.
///
/// The test deliberately:
/// - Runs `get_effective_tools` on an empty `tool_catalog` (no seeding).
/// - Routes through `is_admin()==false` by using a second user added as Member.
/// - Validates that `admin_*` tools are NOT exposed (leak guard).
/// - Validates that `PUBLIC_DISCOVERY_TOOLS` are all present (public floor).
#[tokio::test]
async fn test_tools_list_tenant_member_non_admin_path_no_collapse() -> Result<()> {
    let resources = common::create_test_server_resources().await?;

    // Owner user O creates tenant T_owner. Member user M has their own (unused) tenant
    // but is added to T_owner as a 'member'. Using T_owner as active tenant makes
    // is_admin() == false in the request path.
    let (owner, _owner_token) =
        common::create_test_tenant(&resources, "member-path-owner@example.com").await?;
    let (member, _member_own_token) =
        common::create_test_tenant(&resources, "member-path-member@example.com").await?;

    // Fetch T_owner's id (O is owner here, so list_for_user returns it).
    let repos = resources.database.repositories();
    let owner_tenants = repos
        .tenants
        .list_for_user(owner.id)
        .await
        .map_err(|e| anyhow::anyhow!("list_for_user(owner) failed: {e}"))?;
    let owner_tenant_id = owner_tenants
        .first()
        .expect("owner must have a tenant")
        .id
        .to_string();

    // Add M to T_owner with role='member'.
    add_user_to_tenant_as_member(&resources, &owner_tenant_id, member.id).await?;

    // Mint a JWT for M with T_owner as active_tenant_id. The request's
    // TenantContext will resolve to TenantRole::Member → is_admin()==false.
    let token = resources
        .auth_manager
        .generate_token_with_tenant(
            &member,
            &resources.jwks_manager,
            Some(owner_tenant_id.clone()),
        )
        .map_err(|e| anyhow::anyhow!("generate_token_with_tenant failed: {e}"))?;

    let server = common::spawn_http_mcp_server(&resources).await?;
    let client = Client::new();

    let body = post_tools_list(
        &client,
        &server.base_url(),
        Some(&format!("Bearer {token}")),
    )
    .await?;
    let returned = extract_tool_names(&body);

    // Floor: member path on an empty-catalog tenant must not collapse.
    assert!(
        returned.len() >= MIN_MEMBER_TOOLS,
        "Tenant member tool count {} below regression floor {MIN_MEMBER_TOOLS} \
         (empty-catalog uncatalogued-merge regression). Returned: {returned:?}",
        returned.len()
    );

    // Admin tools must NEVER leak to a non-admin member.
    let admin_leaks: Vec<&String> = returned
        .iter()
        .filter(|n| n.starts_with("admin_"))
        .collect();
    assert!(
        admin_leaks.is_empty(),
        "admin_* tools leaked to non-admin tenant member: {admin_leaks:?}"
    );

    // Public discovery set must be a subset of the member's view.
    let public: HashSet<String> = PUBLIC_DISCOVERY_TOOLS
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let missing_public: Vec<&String> = public.difference(&returned).collect();
    assert!(
        missing_public.is_empty(),
        "Tenant member missing PUBLIC_DISCOVERY_TOOLS: {missing_public:?}"
    );

    // Uncatalogued-merge sanity check: specific feature-flag tool categories
    // must be present (these come from the uncatalogued branch).
    for expected in &["list_coaches", "get_connection_status"] {
        assert!(
            returned.contains(*expected),
            "Tenant member missing uncatalogued feature-flag tool '{expected}'. \
             Returned: {returned:?}"
        );
    }

    assert_every_tool_has_valid_schema(&body);

    println!(
        "✓ Tenant member (non-admin) tools/list: {} tools, no admin leaks",
        returned.len()
    );
    Ok(())
}
