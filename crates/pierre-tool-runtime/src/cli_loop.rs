// ABOUTME: Request construction and the provider call for the text-simulation CLI tool loop
// ABOUTME: Keeps the catalog gate and the servers that stand in for it together
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The one place a text-simulation CLI turn's request is built and sent.
//!
//! `run_cli_tool_loop` withholds the prose tool catalog whenever MCP servers
//! are present, because handing a model both lets it pick the prose — which
//! never opens an MCP session, so the server's `initialize` instructions never
//! reach the model's system prompt. That gate is only correct while the servers
//! themselves travel on the request; a request built without them leaves the
//! runner with neither surface, and the turn still returns `Ok` with prose.

use std::time::Instant;

use pierre_core::errors::AppError;
use pierre_core::tokens::join_prompt_text;
use pierre_llm::call_record::{emit_call_record_with_text, CallRecordInputs};
use pierre_llm::served_tier::observe_served_tier;
use pierre_llm::{ChatMessage, ChatRequest, ChatResponse, McpServerConfig};
use tracing::info;

use crate::tool_loop_io::ToolLoopParams;
use crate::tool_loop_telemetry::{log_wire_shape, millis_elapsed, served_provider_name};

/// Builds one iteration's request for the text-simulation CLI loop.
///
/// The MCP servers ride on every request. [`run_cli_tool_loop`] withholds the
/// prose tool catalog exactly when they are present, because the runner reaches
/// the same tools through them — `copilot` turns them into
/// `--additional-mcp-config` — so a request that dropped them would leave that
/// runner with no tool surface at all: no catalog and no servers.
#[must_use]
pub fn cli_loop_request(
    messages: Vec<ChatMessage>,
    model: &str,
    temperature: Option<f32>,
    mcp_servers: Vec<McpServerConfig>,
) -> ChatRequest {
    let req = ChatRequest::new(messages)
        .with_model(model)
        .with_mcp_servers(mcp_servers);
    match temperature {
        Some(t) => req.with_temperature(t),
        None => req,
    }
}

/// The provider that served a text-loop call, and how long it took.
pub(crate) struct CliLoopCall {
    pub(crate) served_provider: &'static str,
    pub(crate) latency_ms: i64,
}

/// Ask the provider for one text-loop iteration.
///
/// A failed call is recorded here, charged for the prompt it sent; a served one
/// is returned for the loop to record once it has parsed the reply's tool calls.
pub(crate) async fn cli_loop_call(
    params: &ToolLoopParams<'_>,
    llm_messages: &[ChatMessage],
    call_seq: Option<i64>,
) -> Result<(ChatResponse, CliLoopCall), AppError> {
    log_wire_shape("cli_tool_loop", llm_messages);
    let llm_request = cli_loop_request(
        llm_messages.to_vec(),
        params.model,
        params.temperature,
        params.mcp_servers.clone(),
    );

    // notify: CLI/embacle provider call about to start. Same event as
    // the API loop so routing can hide both behind one rule.
    info!(
        target: "notify",
        event = "embacle.call_started",
        model = %params.model,
        "LLM call dispatching"
    );

    let call_start = Instant::now();
    let (response_result, served) =
        observe_served_tier(params.provider.complete(&llm_request)).await;
    let latency_ms = millis_elapsed(call_start);
    // After a fallback the chain's own name is still its head, so the call
    // is attributed to the tier that answered: those tokens are that
    // provider's bill, and the completion event the recorder emits keeps the
    // configured tier beside it so a fallback reads as a difference between
    // the two.
    let served_provider = served_provider_name(served, params.provider.name());
    match response_result {
        Ok(r) => {
            // The loop records the call once it has parsed the reply's tool
            // calls, so the row and the completion event name them.
            Ok((
                r,
                CliLoopCall {
                    served_provider,
                    latency_ms,
                },
            ))
        }
        Err(e) => {
            // The prompt reached the provider before it failed, so the
            // abandoned leg is charged for what it actually sent.
            let prompt_text = join_prompt_text(llm_messages.iter().map(|m| m.content.as_str()));
            emit_call_record_with_text(
                CallRecordInputs {
                    recorder: params.call_recorder.as_ref(),
                    provider: params.provider.name(),
                    model: params.model,
                    configured_provider: params.provider.name(),
                    configured_model: params.model,
                    usage: None,
                    latency_ms,
                    success: false,
                    call_sequence: call_seq,
                    tools_called: Vec::new(),
                },
                Some(&prompt_text),
                None,
            );
            Err(e)
        }
    }
}
