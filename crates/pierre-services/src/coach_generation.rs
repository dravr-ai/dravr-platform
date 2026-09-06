// ABOUTME: Coach generation from a conversation — the excerpt /coach create drafts from and the persona the model proposes
// ABOUTME: Reads the transcript through the chat repository, asks the LLM for a persona, and reads the per-user coach quota
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Drafting a coach out of the conversation `/coach create` is typed in.
//!
//! The last turns are handed to the model, which proposes a persona, and
//! nothing exists until the athlete confirms it. The read and the model call
//! are two steps on purpose — an empty conversation is refused before any
//! provider is resolved, so the refusal costs nothing and needs no LLM.
//!
//! The transcript is read through the [`ChatRepository`] the rest of chat
//! uses, on either database engine. Only what the athlete and the coach
//! actually said reaches the prompt: tool rows, slash-command turns and the
//! replies the platform withheld or could not stand behind are skipped by
//! their stamp, the same rule the coaching prompt applies on replay.
//!
//! Creation itself goes through `CoachesRepository::create` under the same
//! per-user quota `POST /api/coaches` enforces, read here by [`coach_quota`]
//! so the two creation surfaces cannot drift.

use std::sync::Arc;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    MessageRecord, TenantId, UNVERIFIED_CAPABILITY_CLAIM_FINISH_REASON,
    WITHHELD_REPLY_FINISH_REASON,
};
use pierre_database::repositories::{ChatRepository, CoachesRepository};
use pierre_llm::{ChatMessage, ChatProvider, ChatRequest, LlmProvider};
use pierre_runtime_context::{AdminConfigLookup, ConfigLookupScope};
use serde::Deserialize;
use uuid::Uuid;

/// Turns read when the caller sets no bound: the last ten carry the athlete's
/// current concern without dragging the whole thread into the prompt.
pub const DEFAULT_MAX_MESSAGES: usize = 10;

/// Admin-config key for the per-user coach cap.
pub const MAX_COACHES_PER_USER_KEY: &str = "usage_quotas.max_coaches_per_user";

/// Coach cap applied when the key carries no value and when no admin config
/// is wired at all — the quota degrades to this rather than being skipped.
pub const DEFAULT_MAX_COACHES_PER_USER: i64 = 3;

/// Which conversation to draft from, and for whom.
#[derive(Debug, Clone, Copy)]
pub struct ExcerptRequest<'a> {
    /// `chat_conversations.id` to read.
    pub conversation_id: &'a str,
    /// The athlete asking; the conversation must be theirs to open.
    pub user_id: Uuid,
    /// Tenant that owns the conversation row.
    pub tenant_id: TenantId,
    /// Upper bound on the turns read, counted from the most recent.
    pub max_messages: usize,
}

/// One turn of the excerpt, as the model reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExcerptLine {
    /// `user` or `assistant`.
    pub role: String,
    /// The turn's text.
    pub content: String,
}

/// The turns a proposal is drafted from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationExcerpt {
    /// The most recent coaching turns, oldest first.
    pub lines: Vec<ExcerptLine>,
    /// How many coaching turns the conversation holds in total.
    pub total_messages: usize,
}

/// The persona the model proposed for a conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoachProposal {
    /// Display title for the coach.
    pub title: String,
    /// One-paragraph description of what the coach is for.
    pub description: String,
    /// System prompt that shapes the coach's answers.
    pub system_prompt: String,
    /// Suggested category, as the model named it.
    pub category: String,
    /// Suggested tags.
    pub tags: Vec<String>,
}

/// The fields the generation prompt asks the model to return.
#[derive(Debug, Deserialize)]
struct ProposedFields {
    title: String,
    description: String,
    system_prompt: String,
    category: String,
    #[serde(default)]
    tags: Vec<String>,
}

/// Whether a persisted row is coaching the athlete actually read.
///
/// Tool rows are not turns; a slash-command turn is the platform talking; a
/// withheld reply never reached the athlete; an unverified claim must never
/// re-enter a prompt. Each is recognised by its stamp, never by its prose.
fn is_coaching_turn(row: &MessageRecord) -> bool {
    matches!(row.role.as_str(), "user" | "assistant")
        && !row.is_command_turn()
        && !matches!(
            row.finish_reason.as_deref(),
            Some(WITHHELD_REPLY_FINISH_REASON | UNVERIFIED_CAPABILITY_CLAIM_FINISH_REASON)
        )
}

/// Read the turns a proposal would be drafted from.
///
/// Reads the conversation under the caller's identity, so a conversation
/// they cannot open is "not found" rather than someone else's transcript.
/// `Ok(None)` is the honest answer for a conversation with nothing to draft
/// from — the model is never asked to invent a coach out of silence.
///
/// # Errors
///
/// Returns not-found when the conversation is not the caller's, and the
/// repository error when it cannot be read.
pub async fn conversation_excerpt(
    chat: &dyn ChatRepository,
    request: &ExcerptRequest<'_>,
) -> AppResult<Option<ConversationExcerpt>> {
    let user_id = request.user_id.to_string();
    chat.get_conversation(request.conversation_id, &user_id, request.tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation"))?;

    let messages = chat
        .get_messages(request.conversation_id, &user_id, request.tenant_id)
        .await?;
    let turns: Vec<&MessageRecord> = messages.iter().filter(|m| is_coaching_turn(m)).collect();
    let total_messages = turns.len();
    if total_messages == 0 {
        return Ok(None);
    }

    let keep = request.max_messages.max(1);
    let lines = turns
        .iter()
        .skip(total_messages.saturating_sub(keep))
        .map(|m| ExcerptLine {
            role: m.role.clone(),
            content: m.content.clone(),
        })
        .collect();
    Ok(Some(ConversationExcerpt {
        lines,
        total_messages,
    }))
}

/// Resolve the chat provider a coach proposal runs on.
///
/// Prefers the pre-built [`ChatProvider`] singleton (it keeps any Copilot ACP
/// subprocess and OAuth cache warm across requests) and falls back to
/// wrapping a bare [`LlmProvider`] when only the lower-level provider is
/// wired.
///
/// # Errors
///
/// Returns an internal error when neither is configured — a wiring bug, not
/// a transient failure.
pub fn resolve_chat_provider(
    chat_provider: Option<&Arc<ChatProvider>>,
    llm_provider: Option<&Arc<dyn LlmProvider>>,
) -> AppResult<Arc<ChatProvider>> {
    if let Some(provider) = chat_provider {
        return Ok(Arc::clone(provider));
    }
    if let Some(provider) = llm_provider {
        return Ok(Arc::new(ChatProvider::Custom(Arc::clone(provider))));
    }
    Err(AppError::internal(
        "No chat provider configured for coach generation — chat_provider or llm_provider must be set",
    ))
}

/// Ask the model for the persona an excerpt calls for.
///
/// # Errors
///
/// Returns the provider's error when the model call fails, and an internal
/// error when the model's answer is empty or not the JSON the prompt asked
/// for.
pub async fn propose_coach(
    provider: &ChatProvider,
    system_prompt: &str,
    excerpt: &ConversationExcerpt,
) -> AppResult<CoachProposal> {
    let messages_analyzed = excerpt.lines.len();
    let total_messages = excerpt.total_messages;
    let conversation_text = excerpt
        .lines
        .iter()
        .map(|line| format!("[{}]: {}", line.role, line.content))
        .collect::<Vec<_>>()
        .join("\n\n");
    let user_prompt = format!(
        "Analyze this fitness conversation and create a specialized persona profile.\n\n\
         Conversation (last {messages_analyzed} of {total_messages} messages):\n\n\
         {conversation_text}"
    );
    let request = ChatRequest::new(vec![
        ChatMessage::system(system_prompt),
        ChatMessage::user(&user_prompt),
    ]);
    let response = provider.complete(&request).await?;
    if response.content.trim().is_empty() {
        return Err(AppError::internal(
            "Coach generation: the model returned an empty answer",
        ));
    }

    let fields: ProposedFields = serde_json::from_str(strip_code_fence(&response.content))
        .map_err(|e| {
            AppError::internal(format!(
                "Coach generation: the model's answer is not the expected JSON: {e}"
            ))
        })?;
    Ok(CoachProposal {
        title: fields.title,
        description: fields.description,
        system_prompt: fields.system_prompt,
        category: fields.category,
        tags: fields.tags,
    })
}

/// Strip the Markdown code fence a model sometimes wraps its JSON in.
///
/// The prompt asks for bare JSON, but a fenced answer is the single most
/// common way a compliant model still fails to parse, and the fence carries no
/// information.
fn strip_code_fence(content: &str) -> &str {
    let trimmed = content.trim();
    let Some(inner) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let inner = inner.strip_prefix("json").unwrap_or(inner);
    inner.strip_suffix("```").unwrap_or(inner).trim()
}

/// Where a user stands against their coach cap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoachQuota {
    /// Coaches the user has today.
    pub current: i64,
    /// The most they may have.
    pub max: i64,
}

impl CoachQuota {
    /// Whether creating one more coach would breach the cap.
    #[must_use]
    pub const fn is_full(self) -> bool {
        self.current >= self.max
    }
}

/// Read the user's coach count against `usage_quotas.max_coaches_per_user`.
///
/// One read for `POST /api/coaches` and `/coach create confirm`, so the two
/// creation surfaces enforce the same cap. Resolved per user, then tenant,
/// then system-wide; a missing value — or no admin config at all — falls to
/// [`DEFAULT_MAX_COACHES_PER_USER`].
///
/// # Errors
///
/// Returns the repository error when the coach count cannot be read.
pub async fn coach_quota(
    admin_config: Option<&dyn AdminConfigLookup>,
    coaches: &dyn CoachesRepository,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<CoachQuota> {
    let max = match admin_config {
        Some(config) => config
            .get_value(
                MAX_COACHES_PER_USER_KEY,
                ConfigLookupScope::user(&user_id.to_string(), &tenant_id.to_string()),
            )
            .await
            .ok()
            .flatten()
            .and_then(|value| value.as_i64())
            .unwrap_or(DEFAULT_MAX_COACHES_PER_USER),
        None => DEFAULT_MAX_COACHES_PER_USER,
    };
    let current = i64::from(coaches.count(user_id, tenant_id).await?);
    Ok(CoachQuota { current, max })
}
