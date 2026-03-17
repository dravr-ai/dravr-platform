// ABOUTME: Chat orchestration domain service for multi-step chat operations
// ABOUTME: Extracts conversation creation, message dispatch, and model validation from routes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use tracing::info;

use crate::config::LlmProviderType;
use crate::errors::{AppError, AppResult};
use crate::llm::ChatMessage;
use crate::llm::{get_messaging_context_prompt, get_pierre_system_prompt};
use crate::mcp::resources::ServerResources;
use crate::models::TenantId;
use crate::protocols::universal::UniversalExecutor;
use crate::routes::chat::ChatRoutes;
use crate::routes::chat_tool_loop::{self, ToolLoopParams};
use crate::routes::create_chat_provider;
use pierre_core::models::AddMessageParams;
use pierre_database::database::repositories::ChatRepository;
use pierre_database::database::{ConversationRecord, MessageRecord};
use pierre_database::plugins::factory::Database;

/// Result of creating a new conversation, including validated model
pub struct CreateConversationResult {
    /// The created conversation record
    pub conversation: ConversationRecord,
}

/// Result of persisting a user message
pub struct UserMessageResult {
    /// The persisted user message
    pub message: MessageRecord,
    /// The conversation record (for model/`system_prompt` access)
    pub conversation: ConversationRecord,
}

/// Validate the model and create a conversation.
///
/// Business rules:
/// - Uses requested model if provided
/// - Falls back to `PIERRE_LLM_MODEL` environment variable
/// - Fails if no model can be determined
///
/// # Errors
///
/// Returns `AppError::Config` if no model is specified and `PIERRE_LLM_MODEL` is not set.
/// Returns database errors on conversation creation failure.
pub async fn create_conversation(
    database: &Database,
    user_id: &str,
    tenant_id: TenantId,
    title: &str,
    requested_model: Option<&str>,
    system_prompt: Option<&str>,
) -> AppResult<CreateConversationResult> {
    let model = match requested_model {
        Some(m) => m.to_owned(),
        None => LlmProviderType::model_from_env().ok_or_else(|| {
            AppError::config("No model specified and PIERRE_LLM_MODEL environment variable not set")
        })?,
    };

    let conversation = database
        .create_conversation(user_id, tenant_id, title, &model, system_prompt)
        .await?;

    Ok(CreateConversationResult { conversation })
}

/// Verify conversation ownership and persist user message.
///
/// Business rules:
/// - Conversation must exist and belong to the user/tenant
/// - Message is persisted before LLM dispatch (crash-safe)
/// - Returns both message and conversation (for model/prompt access in LLM step)
///
/// # Errors
///
/// Returns `AppError::NotFound` if the conversation does not exist or belongs to another user.
/// Returns database errors on message persistence failure.
pub async fn persist_user_message(
    database: &Database,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
    content: &str,
) -> AppResult<UserMessageResult> {
    // Verify ownership and get conversation details
    let conversation = database
        .get_conversation(conversation_id, user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation not found"))?;

    // Persist user message before LLM dispatch
    let user_msg_params = AddMessageParams {
        conversation_id,
        user_id,
        role: "user",
        content,
        token_count: None,
        finish_reason: None,
        prompt_tokens: None,
        model: None,
    };
    let message = database.add_message(&user_msg_params).await?;

    Ok(UserMessageResult {
        message,
        conversation,
    })
}

/// Get conversation history for LLM context building.
///
/// Returns all messages in the conversation for the given user.
///
/// # Errors
///
/// Returns database errors on message retrieval failure.
pub async fn get_conversation_history(
    database: &Database,
    conversation_id: &str,
    user_id: &str,
) -> AppResult<Vec<MessageRecord>> {
    database.get_messages(conversation_id, user_id).await
}

/// Persist the assistant's response message.
///
/// Called after LLM dispatch + tool execution completes.
/// Returns the persisted message record and updated conversation.
///
/// # Errors
///
/// Returns `AppError::Internal` if the conversation cannot be retrieved after saving.
/// Returns database errors on message persistence failure.
pub async fn persist_assistant_response(
    database: &Database,
    params: &AddMessageParams<'_>,
    tenant_id: TenantId,
) -> AppResult<(MessageRecord, ConversationRecord)> {
    let message = database.add_message(params).await?;

    let conversation = database
        .get_conversation(params.conversation_id, params.user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::internal("Failed to get updated conversation"))?;

    Ok((message, conversation))
}

/// Default max tool iterations for messaging conversations
const MESSAGING_MAX_TOOL_ITERATIONS: usize = 5;

/// Dispatch a user message through the LLM pipeline and return the assistant's text response.
///
/// Simplified version of the chat route's `send_message` flow, used by the messaging
/// gateway to process inbound channel messages. Persists user message, builds LLM
/// context, runs tool loop, persists assistant response, and returns the text content.
///
/// # Errors
///
/// Returns errors from message persistence, LLM provider initialization, or tool execution.
pub async fn dispatch_and_get_response(
    resources: &Arc<ServerResources>,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
    content: &str,
) -> AppResult<String> {
    let database = resources.database.as_ref();

    // Persist user message (crash-safe: saved before LLM dispatch)
    let msg_result =
        persist_user_message(database, conversation_id, user_id, tenant_id, content).await?;

    let conv = msg_result.conversation;

    // Get conversation history for LLM context
    let history = get_conversation_history(database, conversation_id, user_id).await?;

    // Build LLM messages with system prompt and history.
    // Use the conversation's system prompt (set when a coach was selected) if available,
    // otherwise fall back to the default Pierre fitness assistant prompt.
    // Append the messaging context prompt to constrain response length and formatting.
    let has_coach_prompt = conv.system_prompt.is_some();
    let base_prompt = conv
        .system_prompt
        .as_deref()
        .unwrap_or_else(|| get_pierre_system_prompt());
    let system_prompt = format!("{base_prompt}\n\n{}", get_messaging_context_prompt());
    info!(
        conversation_id = %conversation_id,
        has_coach_prompt,
        system_prompt_len = system_prompt.len(),
        system_prompt_preview = %if system_prompt.len() > 200 { &system_prompt[..200] } else { &system_prompt },
        "Messaging dispatch: system prompt selected"
    );
    let mut llm_messages = build_llm_messages(Some(&system_prompt), &history);

    // Build MCP tools and get LLM provider
    let tools = ChatRoutes::build_mcp_tools();
    let provider = create_chat_provider().await?;
    let executor = Arc::new(UniversalExecutor::new(Arc::clone(resources)));

    // Run multi-turn tool execution loop
    let tool_params = ToolLoopParams {
        provider: &provider,
        executor,
        tools: &tools,
        model: &conv.model,
        user_id,
        tenant_id,
        max_iterations: MESSAGING_MAX_TOOL_ITERATIONS,
    };
    let result = chat_tool_loop::run_tool_loop(&tool_params, &mut llm_messages).await?;

    info!(
        conversation_id = %conversation_id,
        content_len = result.content.len(),
        tool_calls = result.tool_calls_count,
        "Messaging LLM dispatch completed"
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
    persist_assistant_response(database, &assistant_params, tenant_id).await?;

    Ok(result.content)
}

/// Build LLM messages from conversation history and optional system prompt
fn build_llm_messages(system_prompt: Option<&str>, history: &[MessageRecord]) -> Vec<ChatMessage> {
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
