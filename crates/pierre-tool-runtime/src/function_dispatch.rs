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

use std::collections::HashMap;

use tracing::{info, warn};

use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_llm::{FunctionCall, FunctionResponse};

use crate::protocol::types::META_AUTH_REQUIRED_PROVIDER;
use crate::protocol::{UniversalExecutor, UniversalRequest, UniversalResponse};
use crate::tool_execution::{build_function_response, log_tool_response_size};
use crate::tool_loop_io::{GuardianConfirmRequest, GuardianDenial};

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

/// Lift a provider-emitted `{"parameters": {...}}` envelope off a tool call's
/// arguments, or `None` when the arguments are already flat.
///
/// Cohere's v1 tool-call format keyed arguments under `parameters`, and
/// command models still emit that shape on the v2 OpenAI-compatible surface.
/// Observed 2026-08-22 on the headless-fallback path: four consecutive
/// `get_group_member_activities` calls each carried
/// `{"parameters":{"member":...}}`, so the tool saw no `member` argument and
/// every retry died on the same "Missing required 'member'" error the model
/// could not repair. The envelope is lifted only when it is the sole key with
/// an object value and `tool_takes_parameters` is false, so a tool whose
/// schema really declares a top-level `parameters` argument (e.g.
/// `validate_configuration`) receives its call untouched.
#[must_use]
pub fn unwrap_parameters_envelope(
    args: &serde_json::Value,
    tool_takes_parameters: bool,
) -> Option<serde_json::Value> {
    if tool_takes_parameters {
        return None;
    }
    let obj = args.as_object()?;
    if obj.len() != 1 {
        return None;
    }
    let inner = obj.get("parameters")?;
    inner.is_object().then(|| inner.clone())
}

/// Execute a single MCP tool call and return the response.
///
/// Runs the Sprint C10 post-LLM allowlist check before dispatch so a
/// prompt-injected tool name that slipped past the catalog filter cannot
/// actually reach the tool handler. Tool execution errors are converted
/// to failed responses so the LLM can observe them in the next turn.
async fn execute_mcp_tool(
    executor: &UniversalExecutor,
    function_call: &FunctionCall,
    user_id: &str,
    tenant_id: TenantId,
) -> UniversalResponse {
    let declares_parameters = executor.tool_declares_property(&function_call.name, "parameters");
    let parameters = unwrap_parameters_envelope(&function_call.args, declares_parameters)
        .map_or_else(
            || function_call.args.clone(),
            |inner| {
                info!(
                    tool_name = %function_call.name,
                    "Lifted provider-emitted 'parameters' envelope off tool-call arguments"
                );
                inner
            },
        );
    // The tenant tool-disable allowlist (formerly enforced here, chat-only) now
    // runs inside `UniversalExecutor::execute_tool` via the Guardian, so every
    // transport — not just chat — honours it. See `guardian::tenant_tool_enabled`.
    let request = UniversalRequest {
        tool_name: function_call.name.clone(),
        parameters,
        user_id: user_id.to_owned(),
        protocol: "chat".to_owned(),
        tenant_id: Some(tenant_id.to_string()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    };

    match executor.execute_tool(request).await {
        Ok(response) => response,
        Err(e) => {
            // Preserve the `ProviderAuthRequired` signal across the
            // `ProtocolError → UniversalResponse` boundary by stuffing the
            // provider slug into `metadata` under `META_AUTH_REQUIRED_PROVIDER`.
            // The tool loop scans for this key and exits early; the chat
            // pipeline mints a hosted-login URL and surfaces it deterministically.
            let metadata = e.provider_auth_required_provider().map(|provider| {
                let mut m: HashMap<String, serde_json::Value> = HashMap::new();
                m.insert(
                    META_AUTH_REQUIRED_PROVIDER.to_owned(),
                    serde_json::Value::String(provider.to_owned()),
                );
                m
            });
            UniversalResponse {
                success: false,
                result: None,
                error: Some(format!("Tool execution failed: {e}")),
                metadata,
            }
        }
    }
}

/// Read a Guardian block out of a failed tool response, if one is stamped.
///
/// Keys on the operator-stamped `metadata.blocked_reason` (set ONLY by the
/// chokepoint's `guardian_denied_response` / `guardian_confirm_response`,
/// never emittable by a tool body) AND requires `!success` — NOT the
/// tool-controllable `result.error_code`, so attacker-influenced tool output
/// carrying `{"error_code":"guardian_denied"}` cannot spuriously abort the
/// turn (S12: data-plane must not drive the control-plane).
fn capture_guardian_block(
    tool_name: &str,
    tool_response: &UniversalResponse,
) -> (Option<GuardianDenial>, Option<GuardianConfirmRequest>) {
    if tool_response.success {
        return (None, None);
    }
    let meta_str = |key: &str| {
        tool_response
            .metadata
            .as_ref()
            .and_then(|m| m.get(key))
            .and_then(serde_json::Value::as_str)
    };
    match meta_str("blocked_reason") {
        Some("guardian") => {
            let reason = meta_str("guardian_reason").unwrap_or("denied").to_owned();
            (
                Some(GuardianDenial {
                    tool_name: tool_name.to_owned(),
                    reason,
                }),
                None,
            )
        }
        Some("guardian_confirm") => (
            None,
            meta_str("pending_id").map(|pending_id| GuardianConfirmRequest {
                tool_name: tool_name.to_owned(),
                pending_id: pending_id.to_owned(),
            }),
        ),
        _ => (None, None),
    }
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

        // Capture the first Guardian denial or parked confirmation (enforce
        // mode) before `build_function_response` reshapes the payload. First
        // to trip wins across the batch.
        if guardian_denied.is_none() && guardian_confirm.is_none() {
            let (denied, confirm) = capture_guardian_block(&function_call.name, &tool_response);
            guardian_denied = denied;
            guardian_confirm = confirm;
        }

        let func_response = build_function_response(function_call, &tool_response);

        log_tool_response_size(&func_response);

        // A failed call's error payload travels only to the model, which may
        // silently retry around it — without this line operators see four
        // `success=false` events and an 88-byte response size but never the
        // reason (the 2026-08-22 Cohere `parameters`-envelope outage took a
        // log-archaeology session to diagnose for exactly this gap). Tool
        // error strings are already model-visible user-facing text, so they
        // are safe at WARN.
        if !tool_response.success {
            warn!(
                tool_name = %function_call.name,
                error = %func_response.response,
                "Tool call failed; error payload returned to the model"
            );
        }

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
