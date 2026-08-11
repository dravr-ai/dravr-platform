// ABOUTME: Tests the Guardian-denied chat stage — a blocked tool renders the localized
// ABOUTME: "blocked for safety" reply and short-circuits; a clean turn is a no-op.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Guardian-denied render stage.
//!
//! Pins the Phase 2 enforcement UX: when the tool loop surfaces a
//! `ToolLoopResult::guardian_denied` (a tool blocked at the runtime chokepoint
//! in `enforce` mode), the stage replaces the reply with the locale-resolved
//! `KEY_GUARDIAN_DENIED` string and reports that it fired (so the pipeline
//! skips LLM post-processing). A clean turn (`guardian_denied == None`) must be
//! a no-op so `observe` mode and ordinary turns are untouched.

use std::sync::Arc;

use pierre_chat_pipeline::stages::guardian_denied::apply_guardian_denied;
use pierre_chat_pipeline::turn::TurnInput;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, EN_GUARDIAN_DENIED, FR_GUARDIAN_DENIED,
};
use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_tool_runtime::tool_execution::{GuardianDenial, ToolLoopResult};
use uuid::Uuid;

/// A `TurnInput` carrying only the locale the stage reads; the rest are inert
/// placeholders.
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

/// A `ToolLoopResult` with an empty body, optionally carrying a Guardian
/// denial — the shape the tool loop hands the recovery stages.
fn loop_result(denied: Option<GuardianDenial>) -> ToolLoopResult {
    ToolLoopResult {
        content: String::new(),
        usage: None,
        finish_reason: Some("guardian_denied".to_owned()),
        activity_list: None,
        tool_calls_count: 1,
        tools_called: vec!["disconnect_provider".to_owned()],
        pending_provider_auth_required: None,
        guardian_denied: denied,
        guardian_confirm: None,
        capability_claim_unverified: false,
    }
}

#[test]
fn denial_renders_localized_reply_and_short_circuits() {
    let registry = Arc::new(MessagingStringsRegistry::new());
    let input = turn_input(Some("en"));
    let mut result = loop_result(Some(GuardianDenial {
        tool_name: "disconnect_provider".to_owned(),
        reason: "budget_exceeded".to_owned(),
    }));

    let fired = apply_guardian_denied(&registry, &input, &mut result);

    assert!(fired, "the stage must fire when a tool was guardian-denied");
    assert_eq!(
        result.content, EN_GUARDIAN_DENIED,
        "the reply must be the locale-resolved guardian-denied string"
    );
}

#[test]
fn denial_respects_resolved_locale() {
    let registry = Arc::new(MessagingStringsRegistry::new());
    let input = turn_input(Some("fr"));
    let mut result = loop_result(Some(GuardianDenial {
        tool_name: "delete_coach".to_owned(),
        reason: "tainted_sink".to_owned(),
    }));

    assert!(apply_guardian_denied(&registry, &input, &mut result));
    assert_eq!(
        result.content, FR_GUARDIAN_DENIED,
        "a French turn must get the French guardian-denied string"
    );
}

#[test]
fn missing_locale_falls_back_to_default_not_empty() {
    let registry = Arc::new(MessagingStringsRegistry::new());
    let input = turn_input(None);
    let mut result = loop_result(Some(GuardianDenial {
        tool_name: "disconnect_provider".to_owned(),
        reason: "egress_forbidden".to_owned(),
    }));

    assert!(apply_guardian_denied(&registry, &input, &mut result));
    assert!(
        !result.content.is_empty(),
        "a None locale must fall back to the default-locale string, never empty"
    );
}

#[test]
fn clean_turn_is_a_no_op() {
    // observe mode (and every ordinary turn) never sets guardian_denied — the
    // stage must not fire and must not touch the (empty) content the LLM path
    // will fill in downstream.
    let registry = Arc::new(MessagingStringsRegistry::new());
    let input = turn_input(Some("en"));
    let mut result = loop_result(None);

    let fired = apply_guardian_denied(&registry, &input, &mut result);

    assert!(
        !fired,
        "a clean turn must not trip the guardian-denied stage"
    );
    assert!(
        result.content.is_empty(),
        "the stage must leave content untouched when no tool was denied"
    );
}
