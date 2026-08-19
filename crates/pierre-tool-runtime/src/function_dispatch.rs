// ABOUTME: Dispatches a batch of model-requested tool calls and reports what actually ran
// ABOUTME: The executed set is evidence; the requested set is only what the model asked for

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Batch tool dispatch, and the distinction between asked-for and ran.
//!
//! Every tool loop hands its model's requested calls here. What comes back
//! separates three things the loops used to conflate: the responses the model
//! is owed (one per request, including failures, so it can see its own
//! mistake), the out-of-band control signals that short-circuit the turn
//! (auth-required, Guardian denial, Guardian confirm), and the names of the
//! tools that actually succeeded.
//!
//! That last one is a security surface, not bookkeeping. It is the evidence
//! the anti-fabrication gate on coach visuals checks a cited `source_tool`
//! against, so it must contain nothing a tool did not actually produce.

use tracing::info;

use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_llm::{FunctionCall, FunctionResponse};

use crate::protocol::types::META_AUTH_REQUIRED_PROVIDER;
use crate::protocol::UniversalExecutor;
use crate::tool_execution::{
    build_function_response, execute_mcp_tool, log_tool_response_size, GuardianConfirmRequest,
    GuardianDenial,
};

/// Output of [`execute_function_calls`].
///
/// Carries the function responses for the LLM plus an out-of-band signal
/// for the tool loop when one of the calls failed with
/// `AppError::ProviderAuthRequired`. The signal travels separately
/// because `FunctionResponse` drops the underlying
/// `UniversalResponse::metadata` to keep the LLM-visible payload minimal.
pub struct ExecutedFunctionCalls {
    /// LLM-visible function responses, one per call in input order.
    pub responses: Vec<FunctionResponse>,
    /// Provider slug of the first tool that returned `ProviderAuthRequired`,
    /// or `None` if every call landed cleanly. The tool loop short-circuits
    /// on `Some(_)` and the chat pipeline mints a hosted-login URL.
    pub auth_required_provider: Option<String>,
    /// The first tool the runtime Guardian blocked in `enforce` mode, or
    /// `None` if no call was denied. The tool loop short-circuits on `Some(_)`
    /// and the chat pipeline renders a localized "blocked for safety" reply.
    /// Travels separately from `responses` because the in-band denial would
    /// otherwise be fed back to the LLM as an ordinary failed tool result.
    pub guardian_denied: Option<GuardianDenial>,
    /// The first tool the Guardian parked pending user confirmation, or
    /// `None`. Same out-of-band contract as `guardian_denied`; the chat
    /// pipeline renders the localized confirmation ask.
    pub guardian_confirm: Option<GuardianConfirmRequest>,
    /// Names of the tools that actually ran and succeeded, in call order.
    ///
    /// NOT the requested `function_calls`: a denial, a refusal, an error and a
    /// hallucinated name all appear there. The anti-fabrication gate on coach
    /// visuals checks a cited `source_tool` against this, and a citation met by
    /// a merely-attempted call is not evidence.
    pub executed: Vec<String>,
}

/// Execute a batch of function calls via the MCP executor and return responses.
///
/// # Errors
///
/// Returns error if any tool execution produces an unrecoverable failure.
pub async fn execute_function_calls(
    executor: &UniversalExecutor,
    function_calls: &[FunctionCall],
    user_id: &str,
    tenant_id: TenantId,
) -> Result<ExecutedFunctionCalls, AppError> {
    let mut responses = Vec::with_capacity(function_calls.len());
    let mut executed: Vec<String> = Vec::new();
    let mut auth_required_provider: Option<String> = None;
    let mut guardian_denied: Option<GuardianDenial> = None;
    let mut guardian_confirm: Option<GuardianConfirmRequest> = None;
    for function_call in function_calls {
        info!(
            tool_name = %function_call.name,
            args = %function_call.args,
            "Executing tool"
        );
        let tool_response = execute_mcp_tool(executor, function_call, user_id, tenant_id).await;

        // Capture the auth-required provider before building the
        // `FunctionResponse`, which intentionally drops `metadata` (the LLM
        // doesn't need it). First tool to trip wins so we don't lose it across
        // a multi-tool batch.
        if auth_required_provider.is_none() {
            if let Some(meta) = tool_response.metadata.as_ref() {
                if let Some(serde_json::Value::String(p)) = meta.get(META_AUTH_REQUIRED_PROVIDER) {
                    auth_required_provider = Some(p.clone());
                }
            }
        }

        // Capture the first Guardian denial (enforce mode) before
        // `build_function_response` reshapes the payload. Key on the
        // operator-stamped `metadata.blocked_reason == "guardian"` (set ONLY by
        // the chokepoint's `guardian_denied_response`, never emittable by a tool
        // body) AND require `!success` — NOT the tool-controllable
        // `result.error_code`, so attacker-influenced tool output carrying
        // `{"error_code":"guardian_denied"}` cannot spuriously abort the turn
        // (S12: data-plane must not drive the control-plane). First to trip wins.
        if guardian_denied.is_none() && guardian_confirm.is_none() && !tool_response.success {
            let blocked_reason = tool_response
                .metadata
                .as_ref()
                .and_then(|m| m.get("blocked_reason"))
                .and_then(serde_json::Value::as_str);
            match blocked_reason {
                Some("guardian") => {
                    let reason = tool_response
                        .metadata
                        .as_ref()
                        .and_then(|m| m.get("guardian_reason"))
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("denied")
                        .to_owned();
                    guardian_denied = Some(GuardianDenial {
                        tool_name: function_call.name.clone(),
                        reason,
                    });
                }
                // A parked confirm-required call, stamped by the chokepoint's
                // `guardian_confirm_response` — same operator-set channel, so
                // the same S12 guarantee applies.
                Some("guardian_confirm") => {
                    if let Some(pending_id) = tool_response
                        .metadata
                        .as_ref()
                        .and_then(|m| m.get("pending_id"))
                        .and_then(serde_json::Value::as_str)
                    {
                        guardian_confirm = Some(GuardianConfirmRequest {
                            tool_name: function_call.name.clone(),
                            pending_id: pending_id.to_owned(),
                        });
                    }
                }
                _ => {}
            }
        }

        let func_response = build_function_response(function_call, &tool_response);

        log_tool_response_size(&func_response);

        // Beside the notify event reporting the same flag, so evidence and
        // operator log cannot disagree about what ran.
        if tool_response.success {
            executed.push(function_call.name.clone());
        }

        responses.push(func_response);

        // P4: once the Guardian denies or parks a tool in this batch, stop —
        // do not execute the remaining siblings (they would commit side
        // effects while the user is shown a "blocked"/"confirm" reply). The
        // turn short-circuits anyway.
        if guardian_denied.is_some() || guardian_confirm.is_some() {
            break;
        }
    }
    Ok(ExecutedFunctionCalls {
        responses,
        auth_required_provider,
        guardian_denied,
        guardian_confirm,
        executed,
    })
}
