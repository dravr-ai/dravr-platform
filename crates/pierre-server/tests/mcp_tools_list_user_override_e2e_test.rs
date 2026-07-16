// ABOUTME: E2E tests proving per-user tool overrides shape the real MCP tools/list wire response
// ABOUTME: Two members of one tenant diverge after a user-level disable; reset restores parity
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// Why this exists
// ===============
// The per-user override overlay is unit-tested at the ToolSelectionService layer
// (user_tool_overrides_test.rs), but the dispatcher glue — ToolContext.user_id
// parsing in PierreToolDispatcher::list_tools → tenant_filtered_schemas(_, user_id)
// — only shows up on the real HTTP path. This file drives a real MCP server and
// asserts the wire response, so a regression in the glue (e.g. dropping the
// user_id before the service call) cannot pass silently.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use anyhow::Result;
use pierre_database::backends::factory::Database;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_services::admin_ops;
use reqwest::Client;
use serde_json::{json, Value};
use std::{collections::HashSet, sync::Arc};
use uuid::Uuid;

/// Tool used for the divergence assertions — catalogued, starter-plan,
/// enabled by default, and always present for a tenant member.
const TARGET_TOOL: &str = "get_activities";

/// Add an existing user to a tenant as a Member in the `tenant_users` junction
/// table (owner rows make `is_admin()` true and bypass the tenant-filtered path).
async fn add_user_to_tenant_as_member(
    resources: &Arc<ServerContext>,
    tenant_id: &str,
    user_id: Uuid,
) -> Result<()> {
    match &*resources.coach.database {
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
                "user-override wire test only runs against SQLite (test default); \
                 the PG repo path is covered by user_tool_overrides_postgresql_test."
            );
        }
    }
    Ok(())
}

/// POST a JSON-RPC `tools/list` and return the response body.
async fn post_tools_list(client: &Client, base_url: &str, token: &str) -> Result<Value> {
    let response = client
        .post(format!("{base_url}/mcp"))
        .header("Content-Type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }))
        .send()
        .await?;
    assert_eq!(
        response.status().as_u16(),
        200,
        "tools/list should return 200"
    );
    Ok(response.json().await?)
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

#[tokio::test]
async fn test_user_disabled_tool_absent_from_wire_tools_list_and_reset_restores() -> Result<()> {
    let resources = common::create_test_server_resources().await?;

    // Owner O anchors tenant T; members A and B join T so both flow through the
    // non-admin tenant-filtered path with the SAME active tenant.
    let (owner, _o_token) =
        common::create_test_tenant(&resources, "override-wire-owner@example.com").await?;
    let (member_a, _a_token) =
        common::create_test_tenant(&resources, "override-wire-a@example.com").await?;
    let (member_b, _b_token) =
        common::create_test_tenant(&resources, "override-wire-b@example.com").await?;

    let repos = resources.coach.database.repositories();
    let tenant_id = repos
        .tenants
        .list_for_user(owner.id)
        .await
        .map_err(|e| anyhow::anyhow!("list_for_user(owner) failed: {e}"))?
        .first()
        .expect("owner must have a tenant")
        .id
        .to_string();

    add_user_to_tenant_as_member(&resources, &tenant_id, member_a.id).await?;
    add_user_to_tenant_as_member(&resources, &tenant_id, member_b.id).await?;

    let token_a = resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &member_a,
            &resources.auth.jwks_manager,
            Some(tenant_id.clone()),
        )
        .map_err(|e| anyhow::anyhow!("token for member A failed: {e}"))?;
    let token_b = resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &member_b,
            &resources.auth.jwks_manager,
            Some(tenant_id.clone()),
        )
        .map_err(|e| anyhow::anyhow!("token for member B failed: {e}"))?;

    let server = common::spawn_http_mcp_server(&resources).await?;
    let client = Client::new();

    // Baseline: both members see the target tool on the wire.
    let a_before =
        extract_tool_names(&post_tools_list(&client, &server.base_url(), &token_a).await?);
    let b_before =
        extract_tool_names(&post_tools_list(&client, &server.base_url(), &token_b).await?);
    assert!(
        a_before.contains(TARGET_TOOL) && b_before.contains(TARGET_TOOL),
        "baseline: both members must see '{TARGET_TOOL}' (A: {}, B: {})",
        a_before.contains(TARGET_TOOL),
        b_before.contains(TARGET_TOOL)
    );

    // Disable the tool for member A only — the production write path.
    admin_ops::set_user_tool_override(
        &repos,
        member_a.id,
        TARGET_TOOL,
        false,
        None,
        Some("wire e2e".to_owned()),
    )
    .await
    .map_err(|e| anyhow::anyhow!("set_user_tool_override failed: {e}"))?;

    // A no longer sees it; B (same tenant, same request shape) still does.
    let a_after =
        extract_tool_names(&post_tools_list(&client, &server.base_url(), &token_a).await?);
    let b_after =
        extract_tool_names(&post_tools_list(&client, &server.base_url(), &token_b).await?);
    assert!(
        !a_after.contains(TARGET_TOOL),
        "'{TARGET_TOOL}' must be absent from member A's wire tools/list after user-disable"
    );
    assert!(
        b_after.contains(TARGET_TOOL),
        "'{TARGET_TOOL}' must remain in member B's wire tools/list (per-user granularity)"
    );
    // The override removes exactly one tool from A's set — nothing else regresses.
    assert_eq!(
        a_after.len() + 1,
        a_before.len(),
        "user-disable must remove exactly one tool from A's list"
    );

    // Reset restores parity, live (no cache in the per-user overlay).
    let removed = admin_ops::remove_user_tool_override(&repos, member_a.id, TARGET_TOOL)
        .await
        .map_err(|e| anyhow::anyhow!("remove_user_tool_override failed: {e}"))?;
    assert!(removed, "reset must report the override row removed");

    let a_reset =
        extract_tool_names(&post_tools_list(&client, &server.base_url(), &token_a).await?);
    assert!(
        a_reset.contains(TARGET_TOOL),
        "'{TARGET_TOOL}' must reappear in member A's wire tools/list after reset"
    );

    println!(
        "✓ per-user override on the wire: A {} → {} → {} tools; B stable at {}",
        a_before.len(),
        a_after.len(),
        a_reset.len(),
        b_after.len()
    );
    Ok(())
}
