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
//! the anti-fabrication gate on agent visuals checks a cited `source_tool`
//! against, so it must contain nothing a tool did not actually produce.
//!
//! A turn the athlete watches also sees each call here as a step: a progress
//! frame when the call starts and another when it ends, in the ACP words the
//! Copilot turn reports its own calls in, so a client draws one trail
//! whichever loop ran. A call the Guardian blocked or parked never ran, and
//! its closing frame says so ([`TOOL_STATUS_NOT_RUN`]) instead of checking it
//! off.

use std::collections::BTreeSet;

use tracing::{info, warn};
use uuid::Uuid;

use pierre_core::errors::AppError;
use pierre_core::models::TenantId;
use pierre_llm::{FunctionCall, FunctionResponse};
use pierre_providers::ai_scope;
use pierre_services::chat_stream::{TurnEventSink, TOOL_STATUS_IN_PROGRESS};

use crate::chat_catalogue;
use crate::protocol::types::{
    auth_required_provider, failed_dispatch, guardian_block_reason, metadata_str,
};
use crate::protocol::{UniversalExecutor, UniversalRequest, UniversalResponse};
use crate::tool_execution::{build_function_response, log_tool_response_size};
use crate::tool_loop_io::{GuardianConfirmRequest, GuardianDenial};
use crate::tool_progress::{closing_status, report_call};

/// Output of [`execute_function_calls`].
///
/// Carries the function responses for the LLM plus an out-of-band signal
/// for the tool loop when one of the calls failed with
/// `AppError::ProviderAuthRequired`. The signal travels separately
/// because `FunctionResponse` drops the underlying
/// `UniversalResponse::metadata` to keep the LLM-visible payload minimal.
#[derive(Debug)]
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
    /// hallucinated name all appear there. The anti-fabrication gate on agent
    /// visuals checks a cited `source_tool` against this, and a citation met by
    /// a merely-attempted call is not evidence.
    pub executed: Vec<String>,
    /// The group members whose data the batch read under their consent
    /// (carnet#1174): what the round's rows are about.
    pub members_read: BTreeSet<Uuid>,
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
    // The tool-disable allowlist (formerly enforced here, chat-only) now runs
    // inside `UniversalExecutor::execute_tool` via the Guardian, so every
    // transport — not just chat — honours it, per tenant and per user. See
    // `guardian::caller_tool_enabled`.
    let request = UniversalRequest {
        tool_name: function_call.name.clone(),
        parameters: chat_catalogue::chat_arguments(parameters),
        user_id: user_id.to_owned(),
        protocol: "chat".to_owned(),
        tenant_id: Some(tenant_id.to_string()),
    };

    executor
        .execute_tool(request)
        .await
        .unwrap_or_else(|e| failed_dispatch(&e))
}

/// Read a Guardian block out of a failed tool response, if one is stamped
/// ([`guardian_block_reason`]'s trust rule).
fn capture_guardian_block(
    tool_name: &str,
    tool_response: &UniversalResponse,
) -> (Option<GuardianDenial>, Option<GuardianConfirmRequest>) {
    let meta_str = |key: &str| metadata_str(tool_response, key);
    match guardian_block_reason(tool_response) {
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
/// With a `sink`, each call is reported to it as a step while it runs: a
/// started frame before dispatch, a closing one after (see the module docs).
/// The call's id is minted here, unique across the turn, since the provider's
/// own call id does not survive into a [`FunctionCall`].
///
/// # Errors
///
/// Returns error if any tool execution produces an unrecoverable failure.
pub async fn execute_function_calls(
    executor: &UniversalExecutor,
    function_calls: &[FunctionCall],
    user_id: &str,
    tenant_id: TenantId,
    sink: Option<&TurnEventSink>,
) -> Result<ExecutedFunctionCalls, AppError> {
    let (executed, members_read) = ai_scope::reading_members(run_function_calls(
        executor,
        function_calls,
        user_id,
        tenant_id,
        sink,
    ))
    .await;
    let mut executed = executed?;
    executed.members_read = members_read;
    Ok(executed)
}

/// [`execute_function_calls`]' body, inside the scope that collects the
/// members the batch read.
async fn run_function_calls(
    executor: &UniversalExecutor,
    function_calls: &[FunctionCall],
    user_id: &str,
    tenant_id: TenantId,
    sink: Option<&TurnEventSink>,
) -> Result<ExecutedFunctionCalls, AppError> {
    let mut responses = Vec::with_capacity(function_calls.len());
    let mut executed: Vec<String> = Vec::new();
    let mut dead_provider: Option<String> = None;
    let mut guardian_denied: Option<GuardianDenial> = None;
    let mut guardian_confirm: Option<GuardianConfirmRequest> = None;
    for function_call in function_calls {
        info!(
            tool_name = %function_call.name,
            args = %function_call.args,
            "Executing tool"
        );
        let call_id = Uuid::new_v4().to_string();
        report_call(sink, &call_id, &function_call.name, TOOL_STATUS_IN_PROGRESS);
        let tool_response = execute_mcp_tool(executor, function_call, user_id, tenant_id).await;
        report_call(
            sink,
            &call_id,
            &function_call.name,
            closing_status(&tool_response),
        );

        // Capture the auth-required provider before building the
        // `FunctionResponse`, which intentionally drops `metadata` (the LLM
        // doesn't need it). First tool to trip wins so we don't lose it across
        // a multi-tool batch.
        if dead_provider.is_none() {
            dead_provider = auth_required_provider(&tool_response);
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
        auth_required_provider: dead_provider,
        guardian_denied,
        guardian_confirm,
        executed,
        members_read: BTreeSet::new(),
    })
}
