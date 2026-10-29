// ABOUTME: A tool an operator switched off for a user is offered on no chat surface, as tools/list hides it
// ABOUTME: Declarations and loopback leave it out together, a call by name is refused; another user keeps it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#406. `tools/list` left out what an operator switched off, but a chat
//! turn published the whole chat-callable set, so the agent was offered a tool
//! the deployment did not offer. Every chat surface now reads
//! `chat_catalogue::offered_schemas` with the turn's `SwitchedOff`, resolved
//! from the same `ToolSelectionService` decision `tools/list` uses.

use crate::common;

use std::sync::Arc;

use embacle_tool_host::ToolSurface;
use pierre_chat_pipeline::ToolSessionTurn;
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_mcp_server::mcp::resources::tool_surface::{HostedToolBridge, TurnToolSurface};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_services::admin_ops;
use pierre_tool_runtime::chat_catalogue::{self, SwitchedOff};
use pierre_tool_runtime::coach_seat::TurnSeat;
use pierre_tool_runtime::guardian::TENANT_DISABLED_ERROR_CODE;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::json;
use uuid::Uuid;

/// Chat-callable, catalogued, and enabled by default for every tenant.
const TARGET_TOOL: &str = "get_activities";

async fn switched_off_for(
    resources: &Arc<ServerContext>,
    tenant: TenantId,
    user_id: Uuid,
) -> SwitchedOff {
    SwitchedOff::resolve(resources.tool_selection(), tenant, user_id).await
}

fn loopback_surface(
    resources: &Arc<ServerContext>,
    tenant: TenantId,
    user_id: Uuid,
    switched_off: &SwitchedOff,
) -> TurnToolSurface {
    let tool_runtime: Arc<dyn ToolRuntime> = resources.clone();
    let bridge = HostedToolBridge::new(true, resources.mcp.tool_registry.clone(), tool_runtime);
    bridge.turn_surface(ToolSessionTurn {
        user_id: &user_id.to_string(),
        tenant_id: tenant,
        conversation_id: "conv-under-test",
        turn_id: ConversationTurnId(Uuid::new_v4()),
        turn_agent_id: None,
        budget: 64,
        seat: TurnSeat::Subject,
        switched_off,
    })
}

async fn loopback_names(
    resources: &Arc<ServerContext>,
    tenant: TenantId,
    user_id: Uuid,
    switched_off: &SwitchedOff,
) -> Vec<String> {
    loopback_surface(resources, tenant, user_id, switched_off)
        .list_tools()
        .await
        .into_iter()
        .map(|tool| tool.name)
        .collect()
}

/// Two users of one tenant, `TARGET_TOOL` switched off for the first.
async fn tenant_with_one_user_switched_off(
    resources: &Arc<ServerContext>,
    prefix: &str,
) -> (TenantId, Uuid, Uuid) {
    let (owner, _) = common::create_test_tenant(resources, &format!("{prefix}-owner@example.com"))
        .await
        .unwrap();
    let (other, _) = common::create_test_tenant(resources, &format!("{prefix}-other@example.com"))
        .await
        .unwrap();
    let repos = resources.agent.database.repositories();
    let tenant = repos
        .tenants
        .list_for_user(owner.id)
        .await
        .unwrap()
        .first()
        .expect("owner must have a tenant")
        .id;
    admin_ops::set_user_tool_override(
        repos,
        owner.id,
        TARGET_TOOL,
        false,
        None,
        Some("carnet#406".to_owned()),
    )
    .await
    .unwrap();
    (tenant, owner.id, other.id)
}

#[tokio::test]
async fn a_tool_switched_off_for_the_user_is_on_no_chat_surface_and_another_user_keeps_it() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (tenant, owner, other) =
        tenant_with_one_user_switched_off(&resources, "switched-off").await;
    let registry = &resources.mcp.tool_registry;

    let switched_off = switched_off_for(&resources, tenant, owner).await;
    assert!(
        switched_off.contains(TARGET_TOOL),
        "the user's override must switch '{TARGET_TOOL}' off"
    );

    let declared: Vec<String> = chat_catalogue::declarations(chat_catalogue::offered_schemas(
        registry,
        TurnSeat::Subject,
        &switched_off,
    ))
    .function_declarations
    .into_iter()
    .map(|decl| decl.name)
    .collect();
    assert!(
        !declared.iter().any(|name| name == TARGET_TOOL),
        "the declarations must leave out a tool switched off for the user"
    );
    assert_eq!(
        declared.len() + 1,
        registry.chat_callable_schemas().len(),
        "exactly the switched-off tool leaves the set"
    );
    assert_eq!(
        loopback_names(&resources, tenant, owner, &switched_off).await,
        declared,
        "the loopback listing must offer exactly what the declarations do"
    );

    let other_switched_off = switched_off_for(&resources, tenant, other).await;
    assert!(
        loopback_names(&resources, tenant, other, &other_switched_off)
            .await
            .iter()
            .any(|name| name == TARGET_TOOL),
        "another user of the tenant must keep '{TARGET_TOOL}'"
    );
}

/// Hidden is not enough: the model can name a tool it was never offered, and
/// the executor must refuse it for the user it is switched off for, as the
/// per-user precedence `tools/list` lists by says.
#[tokio::test]
async fn a_tool_switched_off_for_the_user_is_refused_when_called_by_name() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (tenant, owner, other) = tenant_with_one_user_switched_off(&resources, "refused").await;
    let nothing_hidden = SwitchedOff::default();

    let refused = loopback_surface(&resources, tenant, owner, &nothing_hidden)
        .call(TARGET_TOOL, &json!({}))
        .await;
    assert!(refused.is_error, "the switched-off tool must not run");
    assert!(
        refused.text.contains(TENANT_DISABLED_ERROR_CODE),
        "the refusal must be the tool-disabled one, got: {}",
        refused.text
    );

    let allowed = loopback_surface(&resources, tenant, other, &nothing_hidden)
        .call(TARGET_TOOL, &json!({}))
        .await;
    assert!(
        !allowed.text.contains(TENANT_DISABLED_ERROR_CODE),
        "another user of the tenant must not be refused as disabled, got: {}",
        allowed.text
    );
}
