// ABOUTME: A tool an admin switched off for a user is offered on no chat surface, as tools/list hides it
// ABOUTME: Declarations and loopback leave it out together, a call by name is refused; another user keeps it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#406. `tools/list` left out what an admin switched off, but a chat
//! turn published the whole chat-callable set, so the agent was offered a tool
//! the deployment did not offer. Every chat surface now reads
//! `chat_catalogue::offered_schemas` with the turn's `SwitchedOff`, resolved
//! from the same `ToolSelectionService` decision `tools/list` uses.

use crate::common;
use pierre_core::permissions::scopes::OAuthScope;

use std::sync::Arc;

use crate::helpers::axum_test::AxumTestRequest;
use embacle_tool_host::ToolSurface;
use pierre_chat_pipeline::ToolSessionTurn;
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_core::transport::Transport;
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::tool_surface::{HostedToolBridge, TurnToolSurface};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_providers::ai_scope;
use pierre_services::admin_ops;
use pierre_tool_runtime::chat_catalogue::{self, SwitchedOff};
use pierre_tool_runtime::coach_seat::{MemberReads, TurnSeat};
use pierre_tool_runtime::context::TURN_DIRECT_MESSAGE;
use pierre_tool_runtime::guardian::TENANT_DISABLED_ERROR_CODE;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::tool_access::CallerRole;
use serde_json::{json, Value};
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

/// The loopback an in-app turn opens: built inside the web app's transport
/// declaration, as the turn service scopes every turn, on a direct-message
/// turn in the user's own conversation.
async fn loopback_surface(
    resources: &Arc<ServerContext>,
    tenant: TenantId,
    user_id: Uuid,
    switched_off: &SwitchedOff,
) -> TurnToolSurface {
    let tool_runtime: Arc<dyn ToolRuntime> = resources.clone();
    let bridge = HostedToolBridge::new(true, resources.mcp.tool_registry.clone(), tool_runtime);
    let conversation = common::own_conversation(resources, user_id, tenant).await;
    let surface = ai_scope::serve_over(Transport::WebApp, async {
        bridge.turn_surface(ToolSessionTurn {
            user_id: &user_id.to_string(),
            tenant_id: tenant,
            conversation_tenant_id: tenant,
            conversation_id: &conversation,
            turn_id: ConversationTurnId(Uuid::new_v4()),
            turn_agent_id: None,
            budget: 64,
            seat: TurnSeat::Subject,
            switched_off,
            grant: &OAuthScope::self_grant(),
            role: CallerRole::default(),
        })
    });
    TURN_DIRECT_MESSAGE.scope(Some(true), surface).await
}

async fn loopback_names(
    resources: &Arc<ServerContext>,
    tenant: TenantId,
    user_id: Uuid,
    switched_off: &SwitchedOff,
) -> Vec<String> {
    loopback_surface(resources, tenant, user_id, switched_off)
        .await
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

    let declared_under = |switched_off: &SwitchedOff| -> Vec<String> {
        chat_catalogue::declarations(chat_catalogue::offered_schemas(
            registry,
            TurnSeat::Subject,
            switched_off,
            chat_catalogue::OfferedTo {
                transport: Some(Transport::WebApp),
                grant: &OAuthScope::self_grant(),
                role: CallerRole::default(),
                readers: chat_catalogue::TurnReaders::CallerAlone,
                member_reads: MemberReads::default(),
            },
            &resources.common.config,
        ))
        .function_declarations
        .into_iter()
        .map(|decl| decl.name)
        .collect()
    };
    let declared = declared_under(&switched_off);
    assert!(
        !declared.iter().any(|name| name == TARGET_TOOL),
        "the declarations must leave out a tool switched off for the user"
    );
    // Against what the same caller is offered with nothing switched off:
    // the registry's chat set also holds the tools no plain athlete is
    // offered (one that requires an athlete to name, carnet#1511).
    assert_eq!(
        declared.len() + 1,
        declared_under(&SwitchedOff::default()).len(),
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
        .await
        .call(TARGET_TOOL, &json!({}))
        .await;
    assert!(refused.is_error, "the switched-off tool must not run");
    assert!(
        refused.text.contains(TENANT_DISABLED_ERROR_CODE),
        "the refusal must be the tool-disabled one, got: {}",
        refused.text
    );

    let allowed = loopback_surface(&resources, tenant, other, &nothing_hidden)
        .await
        .call(TARGET_TOOL, &json!({}))
        .await;
    assert!(
        !allowed.text.contains(TENANT_DISABLED_ERROR_CODE),
        "another user of the tenant must not be refused as disabled, got: {}",
        allowed.text
    );
}

/// The A2A extended card lists what an A2A call would run: a tool switched
/// off for the caller is not listed there, as its call is refused, while
/// another user keeps it.
#[tokio::test]
async fn the_a2a_card_lists_no_tool_switched_off_for_the_caller() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (tenant, owner, other) = tenant_with_one_user_switched_off(&resources, "a2a-card").await;
    let other_tenant = resources
        .agent
        .database
        .repositories()
        .tenants
        .list_for_user(other)
        .await
        .unwrap()
        .first()
        .expect("the other user has a tenant")
        .id;
    let card_skills = |user: Uuid, tenant: TenantId| {
        let resources = Arc::clone(&resources);
        async move {
            let token =
                common::delegated_test_token_for_tenant(&resources, user, Some(tenant.to_string()));
            let card: Value = AxumTestRequest::post("/a2a/jsonrpc")
                .header("A2A-Version", "1.0")
                .header("authorization", &format!("Bearer {token}"))
                .json(&json!({"jsonrpc": "2.0", "method": "GetExtendedAgentCard", "id": 1}))
                .send(ProviderToolRouter::build_http_app(&resources))
                .await
                .json();
            card["result"]["skills"]
                .as_array()
                .unwrap_or_else(|| panic!("{card}"))
                .iter()
                .map(|skill| skill["id"].as_str().unwrap().to_owned())
                .collect::<Vec<String>>()
        }
    };

    let offered = card_skills(owner, tenant).await;
    assert!(
        !offered.iter().any(|id| id == TARGET_TOOL),
        "the card must not list a tool switched off for the caller: {offered:?}"
    );
    assert!(
        card_skills(other, other_tenant)
            .await
            .iter()
            .any(|id| id == TARGET_TOOL),
        "another user keeps '{TARGET_TOOL}' on their card"
    );
}
