// ABOUTME: Tests the Guardian-confirm chat stage — a parked tool renders the localized
// ABOUTME: confirmation prompt (tool name + claim token) and short-circuits; a clean turn is a no-op.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Guardian confirm-required render stage.
//!
//! Pins the Confirm HITL UX: when the tool loop surfaces a
//! `ToolLoopResult::guardian_confirm` (a destructive tool parked at the
//! runtime chokepoint under `TaintedDestructive::Confirm`), the stage replaces
//! the reply with the locale-resolved `KEY_GUARDIAN_CONFIRM_PROMPT` carrying
//! the tool name and the `/confirm` claim token, and reports that it fired. A
//! clean turn must be a no-op.

use std::sync::Arc;

use pierre_chat_pipeline::stages::guardian_confirm::apply_guardian_confirm;
use pierre_chat_pipeline::turn::TurnInput;
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_tool_runtime::tool_execution::{GuardianConfirmRequest, ToolLoopResult};
use uuid::Uuid;

fn turn_input(locale: Option<&str>) -> TurnInput {
    let tenant = TenantId::from_uuid(Uuid::new_v4());
    TurnInput {
        conversation_id: "conv-1".to_owned(),
        user_id: Uuid::new_v4().to_string(),
        conversation_tenant_id: tenant,
        tool_tenant_id: tenant,
        content: "disconnect my strava".to_owned(),
        locale: locale.map(ToOwned::to_owned),
        turn_id: ConversationTurnId::new(),
        ambient_context: None,
    }
}

fn loop_result(confirm: Option<GuardianConfirmRequest>) -> ToolLoopResult {
    ToolLoopResult {
        content: String::new(),
        usage: None,
        finish_reason: Some("guardian_confirm".to_owned()),
        activity_list: None,
        tool_calls_count: 1,
        tools_called: vec!["disconnect_provider".to_owned()],
        pending_provider_auth_required: None,
        guardian_denied: None,
        guardian_confirm: confirm,
    }
}

#[test]
fn park_renders_localized_prompt_with_tool_and_token() {
    let registry = Arc::new(MessagingStringsRegistry::new());
    let input = turn_input(Some("en"));
    let mut result = loop_result(Some(GuardianConfirmRequest {
        tool_name: "disconnect_provider".to_owned(),
        pending_id: "abc123def456".to_owned(),
    }));

    let fired = apply_guardian_confirm(&registry, &input, &mut result);

    assert!(fired, "the stage must fire when a tool was parked");
    assert!(
        result.content.contains("disconnect_provider"),
        "the prompt must name the parked tool, got: {}",
        result.content
    );
    assert!(
        result.content.contains("/confirm abc123def456"),
        "the prompt must carry the /confirm claim token, got: {}",
        result.content
    );
    assert!(
        result.content.contains("/deny abc123def456"),
        "the prompt must carry the /deny claim token, got: {}",
        result.content
    );
}

#[test]
fn park_respects_resolved_locale() {
    let registry = Arc::new(MessagingStringsRegistry::new());
    let input = turn_input(Some("fr"));
    let mut result = loop_result(Some(GuardianConfirmRequest {
        tool_name: "disconnect_provider".to_owned(),
        pending_id: "abc123".to_owned(),
    }));

    let fired = apply_guardian_confirm(&registry, &input, &mut result);

    assert!(fired);
    assert!(
        result.content.contains("Par sécurité"),
        "a French turn must get the French prompt, got: {}",
        result.content
    );
}

#[test]
fn clean_turn_is_a_no_op() {
    let registry = Arc::new(MessagingStringsRegistry::new());
    let input = turn_input(Some("en"));
    let mut result = loop_result(None);
    result.content = "normal reply".to_owned();

    let fired = apply_guardian_confirm(&registry, &input, &mut result);

    assert!(!fired, "no park, no fire");
    assert_eq!(result.content, "normal reply", "content must be untouched");
}
