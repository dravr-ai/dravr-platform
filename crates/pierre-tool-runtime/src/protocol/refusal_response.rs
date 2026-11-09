// ABOUTME: The in-band responses the universal executor answers with when it refuses a call
// ABOUTME: A Guardian denial and a tenant-disabled tool, each shaped like a tool error

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use crate::guardian::{self, DenyReason};
use crate::protocol::types::UniversalResponse;

/// Build the in-band response for a Guardian-denied tool call.
///
/// Shaped exactly like a tool's own `is_error` response so the LLM observes a
/// refusal it can adapt to — it is deliberately **not** re-raised as a
/// `ProtocolError`. The chat render layer keys on
/// `result.error_code == "guardian_denied"` to localize the user-facing message.
pub(super) fn guardian_denied_response(tool_name: &str, reason: DenyReason) -> UniversalResponse {
    let mut metadata = HashMap::new();
    metadata.insert(
        "blocked_reason".to_owned(),
        serde_json::Value::String("guardian".to_owned()),
    );
    metadata.insert(
        "guardian_reason".to_owned(),
        serde_json::Value::String(reason.as_str().to_owned()),
    );
    UniversalResponse {
        success: false,
        result: Some(serde_json::json!({
            "error_code": "guardian_denied",
            "reason": reason.as_str(),
        })),
        error: Some(format!("Tool '{tool_name}' was blocked by security policy")),
        metadata: Some(metadata),
    }
}

/// Build the in-band refusal for a tool a tenant has disabled by configuration.
///
/// Unlike [`guardian_denied_response`], this carries **no** `blocked_reason`
/// guardian metadata and a distinct `error_code`, so the chat pipeline treats it
/// as an ordinary tool refusal the LLM adapts to (pick another tool, answer from
/// memory) rather than a "blocked for safety" security incident that abandons
/// the turn. A tenant disabling a tool is config, not a security event.
pub(super) fn tenant_disabled_response(tool_name: &str) -> UniversalResponse {
    UniversalResponse {
        success: false,
        result: Some(serde_json::json!({
            "error_code": guardian::TENANT_DISABLED_ERROR_CODE,
            "reason": guardian::TENANT_DISABLED_REASON,
        })),
        error: Some(guardian::tenant_disabled_message(tool_name)),
        metadata: None,
    }
}
