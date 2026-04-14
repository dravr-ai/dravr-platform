// ABOUTME: Chat orchestration domain service for multi-step chat operations
// ABOUTME: Extracts conversation creation, message dispatch, and model validation from routes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::models::RefreshConfig;
use pierre_core::uuid_utils::parse_uuid;
use std::fmt::Write as _;
use std::sync::Arc;

use tracing::info;

use crate::config::LlmProviderType;
use crate::errors::{AppError, AppResult};
use crate::llm::{ChatMessage, ChatProvider, TokenUsage};
use crate::mcp::resources::ServerResources;
use crate::models::TenantId;
use crate::protocols::universal::UniversalExecutor;
use crate::routes::chat::ChatRoutes;
use crate::routes::chat_tool_loop::{self, ToolLoopParams};
use crate::routes::create_chat_provider;
#[cfg(feature = "tools-groups")]
use crate::services::group_fitness::{fetch_member_snapshots, MemberSnapshot};
use pierre_core::models::AddMessageParams;
use pierre_database::database::repositories::ChatRepository;
use pierre_database::database::{ConversationRecord, MessageRecord};

/// Result of creating a new conversation, including validated model
pub struct CreateConversationResult {
    /// The created conversation record
    pub conversation: ConversationRecord,
}

/// Result of persisting a user message
pub struct UserMessageResult {
    /// The persisted user message
    pub message: MessageRecord,
    /// The conversation record (for model/`coach_id` access)
    pub conversation: ConversationRecord,
}

/// Result of dispatching a message through the LLM pipeline
///
/// Contains the response text along with usage metadata needed for
/// recording LLM usage and incrementing counters.
pub struct DispatchResult {
    /// Final text content from the LLM
    pub content: String,
    /// Token usage statistics if available
    pub usage: Option<TokenUsage>,
    /// Number of tool calls executed during the dispatch
    pub tool_calls_count: u32,
    /// Model identifier used for this completion
    pub model: String,
    /// Name of the LLM provider used (e.g., "gemini", "groq")
    pub provider_name: String,
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
    database: &dyn ChatRepository,
    user_id: &str,
    tenant_id: TenantId,
    title: &str,
    requested_model: Option<&str>,
    coach_id: Option<&str>,
) -> AppResult<CreateConversationResult> {
    let model = match requested_model {
        Some(m) => m.to_owned(),
        None => LlmProviderType::model_from_env().ok_or_else(|| {
            AppError::config("No model specified and PIERRE_LLM_MODEL environment variable not set")
        })?,
    };

    let conversation = database
        .create_conversation(user_id, tenant_id, title, &model, coach_id)
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
    database: &dyn ChatRepository,
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
    database: &dyn ChatRepository,
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
    database: &dyn ChatRepository,
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

/// Resolve group context for a user in a conversation.
///
/// Uses conversation's `group_id` if set, otherwise finds group from user membership.
/// Returns resolved group ID and member snapshots.
#[cfg(feature = "tools-groups")]
async fn resolve_group_context(
    resources: &Arc<ServerResources>,
    user_id: &str,
    conversation_group_id: Option<&str>,
    tool_tenant_id: TenantId,
) -> AppResult<(Option<String>, Vec<MemberSnapshot>)> {
    let user_uuid = parse_uuid(user_id).unwrap_or_default();

    // Resolve group: use conversation group_id or find from membership
    let resolved_group_id = if let Some(gid) = conversation_group_id {
        Some(gid.to_owned())
    } else {
        match resources.repos.groups.list_groups_for_user(user_uuid).await {
            Ok(groups) if groups.len() == 1 => Some(groups[0].id.to_string()),
            Ok(_) => None,
            Err(e) => {
                tracing::debug!("Failed to check group membership: {e}");
                None
            }
        }
    };

    // Fetch member snapshots if we have a resolved group
    let snapshots = if let Some(ref gid) = resolved_group_id {
        let member_ids: Vec<uuid::Uuid> = resources
            .repos
            .groups
            .list_members(gid)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.user_id)
            .collect();
        if member_ids.is_empty() {
            Vec::new()
        } else {
            fetch_member_snapshots(resources, &member_ids, tool_tenant_id).await
        }
    } else {
        Vec::new()
    };

    Ok((resolved_group_id, snapshots))
}

/// Dispatch a user message through the LLM pipeline and return the assistant's text response.
///
/// `conversation_tenant_id` is used for conversation/message DB lookups.
/// `tool_tenant_id` is used for tool execution (OAuth, activities, etc.).
/// These differ when a messaging user belongs to a different tenant than the
/// bot that owns the channel webhook.
///
/// # Errors
///
/// Returns database errors on conversation/message retrieval or persistence failures.
/// Returns LLM provider errors on chat completion failures.
/// Returns tool execution errors from the tool loop.
pub async fn dispatch_and_get_response_with_tool_tenant(
    resources: &Arc<ServerResources>,
    conversation_id: &str,
    user_id: &str,
    conversation_tenant_id: TenantId,
    tool_tenant_id: TenantId,
    content: &str,
) -> AppResult<DispatchResult> {
    let database: &dyn ChatRepository = resources.repos.chat.as_ref();

    // Persist user message (uses conversation tenant for DB lookup)
    let msg_result = persist_user_message(
        database,
        conversation_id,
        user_id,
        conversation_tenant_id,
        content,
    )
    .await?;

    let conv = msg_result.conversation;

    // Tier 4: ensure a long-lived coach session exists for (user, coach) and
    // attach it to the conversation. Idempotent and best-effort.
    let conv = ensure_coach_session_attached(resources, conv, conversation_tenant_id).await;

    // Get conversation history for LLM context
    let history = get_conversation_history(database, conversation_id, user_id).await?;

    // Build LLM messages with system prompt and history.
    // Resolve the coach's system prompt via coach_id lookup (single source of truth
    // on `coaches.system_prompt`). Fall back to the default Pierre fitness assistant
    // prompt when the conversation has no coach attached.
    // Append the messaging context prompt to constrain response length and formatting.
    let coach_ctx = match conv.coach_id.as_deref() {
        Some(coach_id) => {
            resources
                .repos
                .coaches
                .get_coach_runtime_context(coach_id, conversation_tenant_id)
                .await?
        }
        None => None,
    };
    let base_prompt = coach_ctx.as_ref().map_or_else(
        || resources.pierre_system_prompt(),
        |c| c.system_prompt.clone(),
    );

    // Inject group coaching context before appending messaging constraints.
    // Resolve group from user membership when conversation has no group_id,
    // matching the pattern used in chat.rs::inject_group_context_if_applicable.
    #[cfg(feature = "tools-groups")]
    let base_prompt = {
        let group_service = resources.group_service();
        let (resolved_group_id, snapshots) =
            resolve_group_context(resources, user_id, conv.group_id.as_deref(), tool_tenant_id)
                .await?;
        let user_uuid = parse_uuid(user_id).unwrap_or_default();

        group_service
            .inject_group_context(
                &base_prompt,
                "",
                user_uuid,
                tool_tenant_id,
                resolved_group_id.as_deref(),
                &snapshots,
            )
            .await
            .unwrap_or(base_prompt)
    };

    // Trigger non-blocking provider refresh for stale data and inject freshness hint.
    let base_prompt = inject_refresh_context(resources, user_id, tool_tenant_id, base_prompt).await;

    // Tier 2: inject recalled user memory facts into the prompt.
    let base_prompt = inject_memory_recall(
        resources,
        conversation_tenant_id,
        user_id,
        conv.coach_id.as_deref(),
        base_prompt,
    )
    .await;

    // Tier 4: render any pending coach followups so the coach honors its
    // commitments. Followups are rendered as a system block; once the turn
    // succeeds we mark them delivered.
    let (base_prompt, pending_followup_ids) = inject_pending_followups(
        resources,
        conversation_tenant_id,
        user_id,
        conv.coach_id.as_deref(),
        base_prompt,
    )
    .await;

    let raw_system_prompt = format!("{base_prompt}\n\n{}", resources.messaging_context_prompt());
    // Sprint C11: harden the prompt with a per-turn canary token.
    // The canary appears only in a hidden instruction block; if the
    // LLM repeats it verbatim that is conclusive exfiltration proof.
    let prompt_guard = super::prompt_leak::harden_system_prompt(
        conversation_tenant_id,
        conv.coach_id.as_deref(),
        &raw_system_prompt,
    );
    let mut llm_messages = build_llm_messages(Some(&prompt_guard.hardened_prompt), &history);

    // Build MCP tools and get LLM provider
    let tools = ChatRoutes::build_mcp_tools();
    let provider = create_chat_provider().await?;
    let provider_name = provider.name().to_owned();
    let executor = Arc::new(UniversalExecutor::new(Arc::clone(resources)));

    // Tier 1 harness: compact the conversation if the prompt is approaching
    // the context window.
    apply_tier1_compaction(
        resources,
        &provider,
        conversation_tenant_id,
        conversation_id,
        &history,
        &mut llm_messages,
    )
    .await;

    // Run multi-turn tool execution loop (uses user's tenant for tool execution)
    let tool_params = ToolLoopParams {
        provider: &provider,
        executor,
        tools: &tools,
        model: &conv.model,
        user_id,
        tenant_id: tool_tenant_id,
        max_iterations: MESSAGING_MAX_TOOL_ITERATIONS,
    };
    let mut result = chat_tool_loop::run_tool_loop(&tool_params, &mut llm_messages).await?;

    info!(
        conversation_id = %conversation_id,
        content_len = result.content.len(),
        tool_calls = result.tool_calls_count,
        "Messaging LLM dispatch completed"
    );

    // Sprint C9/C11: scan the assistant reply for verbatim system-prompt
    // leaks and for the hidden canary token before guardrails run.
    // Canary hits are conclusive exfiltration — observational for now
    // at the error-log level until a leak audit table is wired.
    super::prompt_leak::scan_assistant_reply(
        &prompt_guard,
        &result.content,
        conversation_tenant_id,
        conv.coach_id.as_deref(),
    );

    // Tier 6 text guardrails: prepend a disclaimer when triggered, reject
    // overly long or blocked-topic responses. Defaults to a safe profile;
    // per-tenant overrides are a Tier 6 admin GUI follow-up.
    result.content = apply_text_guardrails(&result.content);

    // Tier 5.5 bullshit detector: run the heuristic pipeline over the
    // guardrailed reply, persist verdicts, and react per the coach's
    // VerificationConfig (parsed from YAML frontmatter on the coach's
    // system prompt). Pure-Rust in Phase A, no measurable latency.
    #[cfg(feature = "tools-verification")]
    {
        let verification_config = coach_ctx
            .as_ref()
            .map(|c| pierre_evals::VerificationConfig::parse_from_system_prompt(&c.system_prompt))
            .unwrap_or_default();
        result.content = apply_claim_verification(
            resources,
            &result.content,
            user_id,
            conversation_id,
            conv.coach_id.as_deref(),
            conversation_tenant_id,
            &verification_config,
        )
        .await;
    }

    // Persist assistant response (uses conversation tenant for DB lookup)
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
    let (assistant_msg, _updated_conv) =
        persist_assistant_response(database, &assistant_params, conversation_tenant_id).await?;

    // Tier 4: touch the session and mark surfaced followups delivered.
    finalize_session_state(
        resources,
        conv.session_id.as_deref(),
        &pending_followup_ids,
        conversation_tenant_id,
    )
    .await;

    // Tier 2: kick off a background memory extraction for the completed turn.
    // Runs after persistence so extraction failures never affect the stored
    // reply; detached from this function's lifetime via `tokio::spawn`.
    super::memory_extraction::spawn_extract_for_turn(
        Arc::clone(resources),
        super::memory_extraction::SpawnedExtractionRequest {
            tenant_id: conversation_tenant_id,
            user_id: user_id.to_owned(),
            coach_id: conv.coach_id.clone(),
            user_message: content.to_owned(),
            assistant_reply: result.content.clone(),
            source_msg_id: Some(assistant_msg.id),
        },
    );

    Ok(DispatchResult {
        content: result.content,
        usage: result.usage,
        tool_calls_count: result.tool_calls_count,
        model: conv.model,
        provider_name,
    })
}

/// Check provider data freshness, trigger background refresh for stale providers,
/// and append a freshness hint to the system prompt so the coach is aware.
async fn inject_refresh_context(
    resources: &Arc<ServerResources>,
    user_id: &str,
    tenant_id: TenantId,
    base_prompt: String,
) -> String {
    let config = RefreshConfig::default();
    if !config.on_chat_enabled {
        return base_prompt;
    }

    let Ok(user_uuid) = parse_uuid(user_id) else {
        return base_prompt;
    };

    let refresh_service = super::provider_refresh::RefreshService::new(
        resources.repos.clone(),
        #[cfg(feature = "health-sync")]
        resources.sync_orchestrator.clone(),
        resources.sse_manager.clone(),
    );

    let status = refresh_service
        .check_and_refresh(user_uuid, tenant_id, &config)
        .await;

    if !config.inject_coach_hint {
        return base_prompt;
    }

    match super::provider_refresh::RefreshService::build_coach_hint(&status.details) {
        Some(hint) => format!("{base_prompt}\n\n{hint}"),
        None => base_prompt,
    }
}

/// Build LLM messages from conversation history and optional system prompt.
///
/// Shared between the HTTP `/api/chat` path in `routes::chat` and the
/// messaging dispatch path in this module.
#[must_use]
pub fn build_llm_messages(
    system_prompt: Option<&str>,
    history: &[MessageRecord],
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

/// Tier 2 helper: append a recalled-memory block to the system prompt.
///
/// Errors and absent memories both pass through quietly — recall is a
/// best-effort enhancement, not a hard dependency of the dispatch path.
async fn inject_memory_recall(
    resources: &Arc<ServerResources>,
    tenant_id: TenantId,
    user_id: &str,
    coach_id: Option<&str>,
    base_prompt: String,
) -> String {
    let request = super::memory_recall::RecallRequest {
        tenant_id,
        user_id,
        coach_id,
        limit: None,
        token_budget: None,
    };
    match super::memory_recall::build_user_memory_context(resources.repos.memory.as_ref(), &request)
        .await
    {
        Ok(Some(ctx)) => format!("{base_prompt}{}", ctx.block),
        Ok(None) => base_prompt,
        Err(e) => {
            tracing::warn!(error = %e, "memory recall failed; continuing without user facts");
            base_prompt
        }
    }
}

/// Tier 1 helper: run the conversation compactor in place.
///
/// Failures log and continue — a failed compaction never blocks a turn.
async fn apply_tier1_compaction(
    resources: &Arc<ServerResources>,
    provider: &ChatProvider,
    tenant_id: TenantId,
    conversation_id: &str,
    history: &[MessageRecord],
    llm_messages: &mut Vec<ChatMessage>,
) {
    let compactor = super::conversation_compaction::ConversationCompactor::new(
        super::conversation_compaction::CompactionConfig::default(),
    );
    let ctx = super::conversation_compaction::CompactionContext {
        repo: resources.repos.memory.as_ref(),
        provider,
        tenant_id,
        conversation_id,
        history,
        llm_messages,
    };
    match compactor.compact_if_needed(ctx).await {
        Ok(outcome) => {
            if !matches!(
                outcome,
                super::conversation_compaction::CompactionOutcome::NoOp { .. }
            ) {
                tracing::debug!(?outcome, "compaction applied before messaging dispatch");
            }
        }
        Err(e) => {
            tracing::warn!(error = %e, "compaction failed; continuing with uncompacted prompt");
        }
    }
}

/// Tier 4 helper: ensure the conversation has a coach session attached.
///
/// Idempotent — returns the original conversation untouched when there's no
/// coach, when a session is already attached, or when the underlying
/// repository operations fail. The conversation row is mutated in memory and
/// in the database so the rest of the dispatch path can rely on
/// `conv.session_id` being set.
async fn ensure_coach_session_attached(
    resources: &Arc<ServerResources>,
    mut conv: ConversationRecord,
    tenant_id: TenantId,
) -> ConversationRecord {
    let Some(coach_id) = conv.coach_id.clone() else {
        return conv;
    };
    if conv.session_id.is_some() {
        return conv;
    }

    let session = match resources
        .repos
        .memory
        .get_or_open_coach_session(tenant_id, &conv.user_id, &coach_id)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "coach session resolution failed; continuing without session");
            return conv;
        }
    };

    if let Err(e) = resources
        .repos
        .chat
        .set_conversation_session_id(&conv.id, &session.id, tenant_id)
        .await
    {
        tracing::warn!(error = %e, "failed to attach session_id to conversation");
        return conv;
    }

    conv.session_id = Some(session.id);
    conv
}

/// Tier 4 helper: render pending coach followups into the prompt.
///
/// Returns the prompt with an injected followups block (when there are any)
/// plus the list of followup IDs that were surfaced this turn so the
/// dispatcher can mark them delivered after the assistant reply lands.
async fn inject_pending_followups(
    resources: &Arc<ServerResources>,
    tenant_id: TenantId,
    user_id: &str,
    coach_id: Option<&str>,
    base_prompt: String,
) -> (String, Vec<String>) {
    let Some(coach_id) = coach_id else {
        return (base_prompt, Vec::new());
    };
    let followups = match resources
        .repos
        .memory
        .list_pending_followups(tenant_id, user_id, coach_id)
        .await
    {
        Ok(list) => list,
        Err(e) => {
            tracing::warn!(error = %e, "failed to list pending followups");
            return (base_prompt, Vec::new());
        }
    };
    if followups.is_empty() {
        return (base_prompt, Vec::new());
    }

    let mut block = String::from("\n\n## Pending followups you committed to\n\n");
    let mut ids = Vec::with_capacity(followups.len());
    for f in &followups {
        let due = f
            .due_at
            .map(|d| format!(" (due {})", d.to_rfc3339()))
            .unwrap_or_default();
        let _ = writeln!(block, "- {}{}", f.content, due);
        ids.push(f.id.clone());
    }
    block.push_str(
        "\nAddress these now if relevant; otherwise acknowledge them and explain why later.",
    );
    (format!("{base_prompt}{block}"), ids)
}

/// Tier 6 helper: pass the assistant reply through the configured text
/// guardrails. Returns the (possibly disclaimer-prepended) reply, or a
/// safe fallback string when the guardrails reject the response.
fn apply_text_guardrails(reply: &str) -> String {
    use crate::config::text_guardrails::{GuardrailOutcome, GuardrailRejection, TextGuardrails};
    let rules = TextGuardrails::safe_default();
    match rules.apply(reply) {
        GuardrailOutcome::Allowed(text) => text,
        GuardrailOutcome::Rejected(GuardrailRejection::TooLong { length, cap }) => {
            tracing::warn!(
                length,
                cap,
                "guardrails: trimming over-long response to safe fallback"
            );
            "I have a longer response prepared but it exceeds the configured length cap. \
             Want me to break it into a shorter summary?"
                .to_owned()
        }
        GuardrailOutcome::Rejected(GuardrailRejection::BlockedTopic { topic }) => {
            tracing::warn!(topic, "guardrails: blocked topic in coach response");
            "I'd rather not get into that here. Let's stay focused on your training and \
             recovery. Is there something specific I can help with?"
                .to_owned()
        }
    }
}

/// Tier 5.5 helper: run the heuristic bullshit detector pipeline over the
/// finalized reply, persist verdicts, and react to unsupported or
/// contradicted claims per the coach's [`VerificationConfig::fallback_behavior`].
///
/// Verdicts are persisted on a best-effort basis — database failures are
/// logged and swallowed so dispatch never fails on an audit write.
#[cfg(feature = "tools-verification")]
async fn apply_claim_verification(
    resources: &Arc<ServerResources>,
    reply: &str,
    user_id: &str,
    conversation_id: &str,
    coach_id: Option<&str>,
    tenant_id: TenantId,
    config: &pierre_evals::VerificationConfig,
) -> String {
    use pierre_database::repositories::InsertClaimVerdictParams;
    use pierre_evals::VerificationFallback;
    use pierre_memory::claims::ClaimStatus;

    if !config.enabled {
        return reply.to_owned();
    }

    let corpus = super::claim_verification::resolve_corpus(resources);
    let verdicts =
        super::claim_verification::verify_reply_with_config_and_corpus(reply, config, &corpus);
    if verdicts.is_empty() {
        return reply.to_owned();
    }

    let mut problems: Vec<String> = Vec::new();
    for (claim, outcome) in &verdicts {
        if matches!(
            outcome.status,
            ClaimStatus::Unsupported | ClaimStatus::Contradicted
        ) {
            problems.push(claim.text.clone());
        }
        let params = InsertClaimVerdictParams {
            tenant_id,
            user_id,
            coach_id,
            conversation_id: Some(conversation_id),
            message_id: None,
            claim_text: &claim.text,
            category: claim.category,
            status: outcome.status,
            evidence_strength: outcome.evidence_strength,
            confidence: outcome.confidence,
            layer_fired: outcome.layer_fired,
            explanation: Some(&outcome.explanation),
            evidence_refs: outcome.evidence_refs.as_deref(),
        };
        if let Err(e) = resources
            .repos
            .claim_verdicts
            .insert_claim_verdict(&params)
            .await
        {
            tracing::warn!(error = %e, "failed to persist claim verdict");
        }
    }

    if problems.is_empty() {
        return reply.to_owned();
    }

    match config.fallback_behavior {
        VerificationFallback::Warn => format!(
            "{reply}\n\n---\n⚠️ Heads up — I'm not fully confident in {} claim(s) above. \
             Ask me to back them up if you want the evidence.",
            problems.len()
        ),
        VerificationFallback::Silent => reply.to_owned(),
        VerificationFallback::Block => {
            tracing::warn!(
                flagged_claims = problems.len(),
                "Tier 5.5 block fallback fired — replacing reply"
            );
            "I started to answer, but a couple of the claims I was about to make \
             didn't match the evidence I trust. Let me reword that — can you \
             ask me again with a bit more context on what you're trying to figure out?"
                .to_owned()
        }
    }
}

/// Tier 4 helper: post-turn cleanup of session state.
///
/// Touches the active coach session (so "continue where you left off" UI
/// surfaces a fresh timestamp) and marks any followups we surfaced this
/// turn as delivered. Errors are logged and swallowed.
async fn finalize_session_state(
    resources: &Arc<ServerResources>,
    session_id: Option<&str>,
    delivered_followup_ids: &[String],
    tenant_id: TenantId,
) {
    if let Some(session_id) = session_id {
        if let Err(e) = resources
            .repos
            .memory
            .touch_coach_session(session_id, tenant_id)
            .await
        {
            tracing::warn!(error = %e, "failed to touch coach session");
        }
    }
    for followup_id in delivered_followup_ids {
        if let Err(e) = resources
            .repos
            .memory
            .mark_followup_delivered(followup_id, tenant_id)
            .await
        {
            tracing::warn!(error = %e, followup_id, "failed to mark followup delivered");
        }
    }
}
