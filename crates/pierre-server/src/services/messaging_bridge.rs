// ABOUTME: Bridge service routing messages between external providers and the Dravr chat system
// ABOUTME: Handles incoming webhook messages, LLM dispatch, and response delivery back to providers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;
use std::time::Instant;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::AddMessageParams;
use pierre_database::database::repositories::MessagingRepository;
use pierre_messaging::types::IncomingMessage;
use pierre_messaging::MessagingProvider;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::llm::{get_pierre_system_prompt, ChatMessage, ChatProvider, FunctionDeclaration, Tool};
use crate::mcp::resources::ServerResources;
use crate::protocols::universal::UniversalExecutor;
use crate::routes::chat_tool_loop::{self, ToolLoopParams};
use crate::services::chat_orchestration;

/// Process an incoming message from an external messaging provider
///
/// This is the core bridge function that:
/// 1. Looks up the channel binding to find the Dravr conversation
/// 2. Persists the user message in the conversation
/// 3. Runs the LLM tool loop (same code path as web chat)
/// 4. Sends the AI response back through the messaging provider
///
/// # Errors
///
/// Returns an error if the channel binding is not found, the conversation
/// is inaccessible, the LLM call fails, or the response cannot be sent.
pub async fn process_incoming_message(
    resources: &Arc<ServerResources>,
    provider: &dyn MessagingProvider,
    message: &IncomingMessage,
    connection_id: &str,
) -> AppResult<()> {
    let start_time = Instant::now();

    // Look up channel binding to find the target conversation
    let binding = resources
        .database
        .get_channel_binding_by_channel(connection_id, &message.channel_id)
        .await?
        .ok_or_else(|| {
            AppError::not_found(format!(
                "No channel binding for channel {} on connection {connection_id}",
                message.channel_id
            ))
        })?;

    if !binding.active {
        debug!(
            channel_id = %message.channel_id,
            "Ignoring message for inactive channel binding"
        );
        return Ok(());
    }

    let user_id = &binding.user_id;
    let conversation_id = &binding.conversation_id;
    let tenant_id_uuid = Uuid::parse_str(&binding.tenant_id)
        .map_err(|e| AppError::internal(format!("Invalid tenant_id in binding: {e}")))?;
    let tenant_id = pierre_core::models::TenantId::from(tenant_id_uuid);

    info!(
        channel = %message.channel_id,
        conversation = %conversation_id,
        provider = %provider.name(),
        "Bridging message from external provider to Dravr conversation"
    );

    // Persist the incoming message in the conversation
    let msg_result = chat_orchestration::persist_user_message(
        resources.database.as_ref(),
        conversation_id,
        user_id,
        tenant_id,
        &message.text,
    )
    .await?;

    let conv = msg_result.conversation;

    // Get conversation history for LLM context
    let history =
        chat_orchestration::get_conversation_history(resources.database.as_ref(), conversation_id, user_id)
            .await?;

    // Build system prompt and LLM messages
    let system_prompt = conv
        .system_prompt
        .as_deref()
        .unwrap_or_else(|| get_pierre_system_prompt());
    let mut llm_messages = build_llm_messages(Some(system_prompt), &history);

    // Build tool definitions and get LLM provider
    let tools = build_bridge_tools();
    let llm_provider = crate::routes::create_chat_provider().await?;

    // Create MCP executor for tool calls
    let executor = Arc::new(UniversalExecutor::new(resources.clone()));

    // Run multi-turn tool execution loop
    let tool_params = ToolLoopParams {
        provider: &llm_provider,
        executor: Arc::clone(&executor),
        tools: &tools,
        model: &conv.model,
        user_id,
        tenant_id,
        max_iterations: 5,
    };
    let result = chat_tool_loop::run_tool_loop(&tool_params, &mut llm_messages).await?;

    // Safe cast: execution time will never exceed u64::MAX milliseconds
    #[allow(clippy::cast_possible_truncation)]
    let execution_time_ms = start_time.elapsed().as_millis() as u64;

    info!(
        content_len = result.content.len(),
        tool_calls = result.tool_calls_count,
        execution_ms = execution_time_ms,
        "Bridge LLM processing completed"
    );

    // Persist assistant response
    let token_count = result.usage.as_ref().map(|u| u.completion_tokens);
    let prompt_tokens = result.usage.as_ref().map(|u| u.prompt_tokens);

    let assistant_params = AddMessageParams {
        conversation_id,
        user_id,
        role: "assistant",
        content: &result.content,
        token_count,
        finish_reason: result.finish_reason.as_deref(),
        prompt_tokens,
        model: Some(&conv.model),
    };
    chat_orchestration::persist_assistant_response(
        resources.database.as_ref(),
        &assistant_params,
        tenant_id,
    )
    .await?;

    // Send the AI response back to the external channel
    let outgoing = pierre_messaging::types::OutgoingMessage::text(
        &message.channel_id,
        &result.content,
    );

    if let Err(e) = provider.send_message(&outgoing).await {
        warn!(
            error = %e,
            channel = %message.channel_id,
            provider = %provider.name(),
            "Failed to send response to external provider"
        );
        return Err(e);
    }

    info!(
        channel = %message.channel_id,
        provider = %provider.name(),
        execution_ms = execution_time_ms,
        "Bridge message processed and response delivered"
    );

    Ok(())
}

/// Build LLM messages from conversation history
fn build_llm_messages(
    system_prompt: Option<&str>,
    history: &[pierre_database::database::MessageRecord],
) -> Vec<ChatMessage> {
    let mut messages = Vec::with_capacity(history.len() + 1);

    if let Some(prompt) = system_prompt {
        messages.push(ChatMessage::system(prompt));
    }

    for msg in history {
        let chat_msg = match msg.role.as_str() {
            "user" => ChatMessage::user(&msg.content),
            "assistant" => ChatMessage::assistant(&msg.content),
            "system" => ChatMessage::system(&msg.content),
            _ => continue,
        };
        messages.push(chat_msg);
    }

    messages
}

/// Build tool definitions for bridged conversations
///
/// Uses the same tool definitions as web chat, providing connection status,
/// activity data, and analysis tools.
fn build_bridge_tools() -> Tool {
    let declarations = vec![
        FunctionDeclaration {
            name: "get_connection_status".to_owned(),
            description: "Check which fitness providers are connected".to_owned(),
            parameters: Some(serde_json::json!({"type": "object", "properties": {}})),
        },
        FunctionDeclaration {
            name: "get_activities".to_owned(),
            description: "Get user's recent fitness activities".to_owned(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {
                    "provider": {"type": "string"},
                    "limit": {"type": "integer"},
                    "offset": {"type": "integer"}
                },
                "required": ["provider"]
            })),
        },
        FunctionDeclaration {
            name: "get_athlete".to_owned(),
            description: "Get user's athlete profile information".to_owned(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {"provider": {"type": "string"}},
                "required": ["provider"]
            })),
        },
        FunctionDeclaration {
            name: "get_stats".to_owned(),
            description: "Get user's overall fitness statistics".to_owned(),
            parameters: Some(serde_json::json!({
                "type": "object",
                "properties": {"provider": {"type": "string"}},
                "required": ["provider"]
            })),
        },
    ];

    Tool {
        function_declarations: declarations,
    }
}
