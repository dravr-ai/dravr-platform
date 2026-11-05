// ABOUTME: /confirm and /deny resolve only the agent draft /agent create parked
// ABOUTME: Covers the owner check, expiry, the anti-enumeration reply, and that a row that is no draft never runs

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The claim path behind `/confirm` and `/deny`.
//!
//! The table holds agent drafts, and a deploy can find a tool call an older binary
//! parked there still inside its TTL. The creation of the agent itself is pinned
//! through the slash commands in `chat_slash_commands_test`.

use crate::common;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use common::{create_test_server_resources_with_llm_and_tools, create_test_user};
use dravr_tronc::mcp::schema::{Tool as McpToolDefinition, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_commands::pending_confirm::{ConfirmHandler, DenyHandler};
use pierre_commands::{CommandHandler, ConversationRotation, PlatformCommandContext};
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_AGENT_CREATE_DISCARDED, KEY_GUARDIAN_CONFIRM_EXPIRED,
    KEY_GUARDIAN_CONFIRM_NOT_FOUND,
};
use pierre_core::models::TenantId;
use pierre_database::repositories::{ClaimOutcome, PendingGuardianAction, AGENT_PROPOSAL_ACTION};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::RuntimeTool;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::guardian_e2e_test::AnswersPlainly;

/// A destructive write that says whether anything ever ran it.
struct NoticesARun {
    ran: Arc<AtomicBool>,
}

#[async_trait]
impl McpTool<dyn ToolRuntime> for NoticesARun {
    fn definition(&self) -> McpToolDefinition {
        McpToolDefinition {
            name: "notices_a_run".to_owned(),
            description: "Records that it ran".to_owned(),
            input_schema: json!({ "type": "object" }),
            annotations: None,
            output_schema: None,
            execution: None,
        }
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA
    }

    async fn execute(
        &self,
        _state: &Arc<dyn ToolRuntime>,
        _ctx: &ToolContext,
        _args: Value,
    ) -> ToolResponse {
        self.ran.store(true, Ordering::SeqCst);
        ToolResponse::text("done".to_owned())
    }
}

pierre_tool_runtime::declare_security!(NoticesARun => IRREVERSIBLE);

/// A server that registers [`NoticesARun`], and the flag it sets.
async fn server() -> (Arc<ServerContext>, Arc<AtomicBool>) {
    let ran = Arc::new(AtomicBool::new(false));
    let tool: Arc<dyn RuntimeTool> = Arc::new(NoticesARun {
        ran: Arc::clone(&ran),
    });
    let provider = Arc::new(AnswersPlainly {
        models: vec!["plain-model".to_owned()],
    });
    let resources = create_test_server_resources_with_llm_and_tools(provider, vec![tool])
        .await
        .expect("server resources");
    (resources, ran)
}

/// A draft as `/agent create` parks it.
fn draft(user_id: Uuid, tenant: TenantId) -> PendingGuardianAction {
    PendingGuardianAction {
        id: Uuid::new_v4().simple().to_string(),
        tenant_id: tenant.to_string(),
        user_id: user_id.to_string(),
        tool_name: AGENT_PROPOSAL_ACTION.to_owned(),
        arguments: json!({
            "title": "Hill Agent",
            "description": "Coaches hill repeats.",
            "system_prompt": "You coach hill repeats.",
            "category": "training",
            "tags": [],
        }),
        deny_reason: "awaiting_confirmation".to_owned(),
    }
}

async fn park(resources: &Arc<ServerContext>, row: &PendingGuardianAction, minutes: i64) {
    resources
        .common
        .repos
        .guardian_actions
        .create_pending_action(row, Utc::now() + Duration::minutes(minutes))
        .await
        .expect("park row");
}

fn command_ctx(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant: TenantId,
    pending_id: &str,
) -> PlatformCommandContext {
    PlatformCommandContext {
        user_id,
        tenant_id: tenant,
        channel_type: "web".to_owned(),
        args: vec![pending_id.to_owned()],
        raw_text: format!("/confirm {pending_id}"),
        ctx: Arc::<ServerContext>::clone(resources),
        locale: "en".to_owned(),
        is_direct_message: true,
        ambient_group_fallback: false,
        conversation_id: None,
        conversation_tenant_id: tenant,
        sender_id: None,
        rotation: ConversationRotation::default(),
        tool_runtime: Arc::<ServerContext>::clone(resources),
    }
}

/// Whether `id` can still be claimed by its owner. Consumes it when it can.
async fn still_claimable(
    resources: &Arc<ServerContext>,
    id: &str,
    user_id: Uuid,
    tenant: TenantId,
) -> bool {
    matches!(
        resources
            .common
            .repos
            .guardian_actions
            .claim_pending_action(id, &user_id.to_string(), &tenant.to_string(), "denied")
            .await
            .expect("claim runs"),
        ClaimOutcome::Claimed(_)
    )
}

#[tokio::test]
async fn deny_discards_the_draft_and_a_stranger_sees_nothing() {
    let (resources, _) = server().await;
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let registry = MessagingStringsRegistry::new();

    let row = draft(user_id, tenant);
    park(&resources, &row, 5).await;

    // A different user probing the id gets the anti-enumeration not-found, on
    // either verb, and the row stays the owner's.
    let stranger = command_ctx(&resources, Uuid::new_v4(), tenant, &row.id);
    for answer in [
        DenyHandler.execute(&stranger).await.expect("deny runs"),
        ConfirmHandler
            .execute(&stranger)
            .await
            .expect("confirm runs"),
    ] {
        assert_eq!(
            answer.text,
            registry.render(KEY_GUARDIAN_CONFIRM_NOT_FOUND, "en", &[])
        );
    }

    // The owner denies: the localized ack, and the row is consumed.
    let owner = command_ctx(&resources, user_id, tenant, &row.id);
    let resp = DenyHandler.execute(&owner).await.expect("deny runs");
    assert_eq!(
        resp.text,
        registry.render(KEY_AGENT_CREATE_DISCARDED, "en", &[])
    );
    assert!(!still_claimable(&resources, &row.id, user_id, tenant).await);

    // A replay reads as an unknown id.
    let resp = DenyHandler.execute(&owner).await.expect("deny runs");
    assert_eq!(
        resp.text,
        registry.render(KEY_GUARDIAN_CONFIRM_NOT_FOUND, "en", &[])
    );
}

#[tokio::test]
async fn expiry_is_checked_at_resolution_and_a_missing_id_reads_as_unknown() {
    let (resources, _) = server().await;
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let registry = MessagingStringsRegistry::new();

    let stale = draft(user_id, tenant);
    park(&resources, &stale, -1).await;
    let resp = ConfirmHandler
        .execute(&command_ctx(&resources, user_id, tenant, &stale.id))
        .await
        .expect("confirm runs");
    assert_eq!(
        resp.text,
        registry.render(KEY_GUARDIAN_CONFIRM_EXPIRED, "en", &[])
    );

    // Missing argument gets the same anti-enumeration reply as an unknown id.
    let not_found = registry.render(KEY_GUARDIAN_CONFIRM_NOT_FOUND, "en", &[]);
    for id in ["", "   ", "no-such-token"] {
        let ctx = command_ctx(&resources, user_id, tenant, id);
        assert_eq!(
            ConfirmHandler
                .execute(&ctx)
                .await
                .expect("confirm runs")
                .text,
            not_found,
            "{id:?}"
        );
    }
}

/// A call an older binary parked can still be in the table after a deploy. Its
/// athlete's yes must run nothing: the row answers like an unknown id, and is spent.
#[tokio::test]
async fn a_claimed_row_that_is_no_draft_answers_not_found_and_never_runs() {
    let (resources, ran) = server().await;
    let (user_id, _) = create_test_user(&resources.agent.database)
        .await
        .expect("test user");
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    let not_found =
        MessagingStringsRegistry::new().render(KEY_GUARDIAN_CONFIRM_NOT_FOUND, "en", &[]);
    let parked_call = || PendingGuardianAction {
        tool_name: "notices_a_run".to_owned(),
        arguments: json!({}),
        deny_reason: "tainted_sink".to_owned(),
        ..draft(user_id, tenant)
    };

    let confirmed = parked_call();
    park(&resources, &confirmed, 5).await;
    let ctx = command_ctx(&resources, user_id, tenant, &confirmed.id);
    assert_eq!(
        ConfirmHandler
            .execute(&ctx)
            .await
            .expect("confirm runs")
            .text,
        not_found
    );
    assert!(!still_claimable(&resources, &confirmed.id, user_id, tenant).await);

    let denied = parked_call();
    park(&resources, &denied, 5).await;
    let ctx = command_ctx(&resources, user_id, tenant, &denied.id);
    assert_eq!(
        DenyHandler.execute(&ctx).await.expect("deny runs").text,
        not_found
    );

    assert!(!ran.load(Ordering::SeqCst), "the stored call must not run");
}
