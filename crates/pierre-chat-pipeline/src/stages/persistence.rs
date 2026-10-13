// ABOUTME: Conversation and message persistence stages — create, verify ownership, append, retrieve
// ABOUTME: Single entry point both routes/chat and messaging_ingress use to open a new conversation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Conversation and message persistence.
//!
//! Four responsibilities:
//!
//! 1. Create conversation — validates the model selection against the
//!    `PIERRE_LLM_CHAIN` head's model, persists the new conversation record.
//! 2. Persist user message — verifies conversation ownership (tenant +
//!    user), persists the user turn before LLM dispatch (crash-safe: a
//!    server failure mid-dispatch still leaves the user's question visible
//!    on reload).
//! 3. Get conversation history — retrieves the recent messages for the
//!    conversation, ordered oldest-first, for LLM context assembly. Slash
//!    command rows are left out here: they are transcript, never prompt.
//! 4. Persist assistant response — appends the LLM reply with full
//!    token usage and model metadata, then re-reads the conversation
//!    record so the caller can access updated `updated_at` / `summary`
//!    fields.
//! 5. Resolve a platform-composed turn — the same ownership check with no
//!    write at all, for a prompt the athlete did not send. See
//!    [`resolve_platform_turn`] and [`TurnOrigin`].
//!
//! Both message writers also fan the row out to the group's shared room
//! transcript (`group_transcript_entries`) when the conversation is
//! group-bound, so every surface's group turns land in one queryable,
//! consent-gated room view — and both advance the participant's read
//! marker past the row they wrote, so an athlete's own turn never counts
//! as unread for them. One rule for web, mobile and messaging.
//!
//! A reply the platform posted on its own initiative ([`TurnOrigin::Platform`])
//! is the exception to both: nobody was in front of it when it landed, so it
//! stays unread until the athlete opens the thread, and it is one athlete's
//! own coaching, never a room's.

use pierre_core::models::AddMessageParams;
use pierre_core::transport::TransportPolicy;
use pierre_database::database::repositories::ChatRepository;
use pierre_database::database::{ConversationRecord, MessageRecord};
use pierre_database::repositories::CoachingGroupRepository;
use pierre_providers::ai_scope;

use crate::turn::{TurnInput, UserMessageResult};
use crate::turn_stop::author_marker;
use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{NewGroupTranscriptEntry, TranscriptSpeaker};
use pierre_core::models::{TenantId, TurnOrigin};
use pierre_core::uuid_utils::parse_uuid;
use tracing::warn;
use uuid::Uuid;

/// Advance the participant's read marker past a row they just wrote or were
/// just shown. Best-effort: the row is already durable, and a marker that
/// failed to move costs one spurious unread badge, not the turn.
async fn advance_read_marker(
    database: &dyn ChatRepository,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
    message_id: &str,
) {
    match database
        .mark_conversation_read(conversation_id, user_id, tenant_id, Some(message_id))
        .await
    {
        Ok(true) => {}
        Ok(false) => warn!(
            conversation_id,
            message_id, "read marker did not advance: the row is not visible to its own writer"
        ),
        Err(e) => {
            warn!(error = %e, conversation_id, "read marker could not advance after the turn");
        }
    }
}

/// Fan a just-persisted user/assistant row out to the group's shared room
/// transcript when the conversation is group-bound. The entry carries the
/// row's stamp (carnet#769), so a room reader is held to the same terms as a
/// reader of the row itself.
///
/// The entry is attributed to the conversation's member (`user_id`) for both
/// speakers: an agent row is the reply that member received, so consent
/// withholding hides the pair together. Non-group conversations append
/// nothing. Shared with the turn service, which fans out a room-visible
/// slash-command turn the same way, so a plan posted with `/plan share` is
/// in the room history a later turn reads.
///
/// # Errors
///
/// Returns database errors from the transcript append — the room transcript
/// is turn persistence, and a silently missing row would replay as a hole in
/// every member's shared view.
pub(crate) async fn fan_out_to_group_transcript(
    groups: &dyn CoachingGroupRepository,
    conversation: &ConversationRecord,
    tenant_id: TenantId,
    user_id: &str,
    speaker: TranscriptSpeaker,
    message: &MessageRecord,
) -> AppResult<()> {
    let Some(group_id) = conversation.group_id.as_deref() else {
        return Ok(());
    };
    let tenant_str = tenant_id.to_string();
    let entry = NewGroupTranscriptEntry {
        group_id,
        tenant_id: &tenant_str,
        author_user_id: parse_uuid(user_id)?,
        speaker,
        content: &message.content,
        source_conversation_id: Some(&conversation.id),
        source_message_id: Some(&message.id),
        transport_policy: message.transport_policy,
    };
    groups.append_transcript_entry(&entry).await
}

/// Verify conversation ownership and persist the user message.
///
/// Business rules:
/// - Conversation must exist and belong to the user/tenant
/// - Message is persisted before LLM dispatch (crash-safe)
/// - Group-bound conversations fan the row out to the shared room transcript
/// - Returns both message and conversation (for model/prompt access in LLM step)
///
/// # Errors
///
/// Returns `AppError::NotFound` if the conversation does not exist or
/// belongs to another user. Returns database errors on message persistence
/// failure.
pub async fn persist_user_message(
    database: &dyn ChatRepository,
    groups: &dyn CoachingGroupRepository,
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

    // Persist user message before LLM dispatch. The row names its author:
    // `chat_messages` has no such column, and a stop request has to tell the
    // caller's own question from another participant's.
    let author = author_marker(user_id);
    let user_msg_params = AddMessageParams {
        tenant_id,
        conversation_id,
        user_id,
        role: "user",
        content,
        token_count: None,
        finish_reason: None,
        prompt_tokens: None,
        model: None,
        content_blocks: author.as_deref(),
        // The athlete's own words, derived from no provider data.
        transport_policy: TransportPolicy::AnyTransport,
    };
    let message = database.add_message(&user_msg_params).await?;
    advance_read_marker(database, conversation_id, user_id, tenant_id, &message.id).await;

    fan_out_to_group_transcript(
        groups,
        &conversation,
        tenant_id,
        user_id,
        TranscriptSpeaker::Member,
        &message,
    )
    .await?;

    Ok(UserMessageResult {
        message,
        conversation,
    })
}

/// Resolve the prompt a turn answers, persisting it only when the athlete sent it.
///
/// An athlete's message is written before dispatch, so a crash mid-turn still
/// leaves their question on the thread. A platform-composed prompt is not
/// theirs to write: the row would be attributed to them in their own thread,
/// advance their read marker past a message they never saw, and fan out to a
/// group room they never posted in. It reaches the model through the history
/// instead — see [`append_platform_prompt`] — and only the reply is persisted,
/// because only the reply is real.
///
/// # Errors
///
/// Returns `AppError::NotFound` when the conversation does not exist or the
/// user is not a participant, on both paths, and database errors on a failed
/// write.
pub async fn resolve_turn_prompt(
    database: &dyn ChatRepository,
    groups: &dyn CoachingGroupRepository,
    input: &TurnInput,
) -> AppResult<UserMessageResult> {
    let conversation_id = input.conversation_id.as_str();
    let user_id = input.user_id.as_str();
    let tenant_id = input.conversation_tenant_id;
    let content = input.content.as_str();
    if input.origin.persists_user_row() {
        persist_user_message(
            database,
            groups,
            conversation_id,
            user_id,
            tenant_id,
            content,
        )
        .await
    } else {
        resolve_platform_turn(database, conversation_id, user_id, tenant_id, content).await
    }
}

/// Put a platform-composed prompt in front of the model.
///
/// An athlete's message is already the last row of the loaded history, having
/// been written before the load. A platform-composed one was deliberately not
/// written, so it is appended here: the model has to read the question to
/// answer it, and this copy lives only for the length of the turn. A no-op on
/// an athlete turn, whose prompt is already there.
pub fn append_platform_prompt(
    history: &mut Vec<MessageRecord>,
    prompt: &MessageRecord,
    origin: TurnOrigin,
) {
    if !origin.persists_user_row() {
        history.push(prompt.clone());
    }
}

/// Resolve the conversation for a turn the platform composed, writing nothing.
///
/// The athlete-authored path ([`persist_user_message`]) writes the prompt as a
/// `user` row and returns it. A platform-authored turn must not: the row would
/// be attributed to the athlete in their own thread, advance their read marker
/// past a message they never saw, and fan out to a group room they never
/// posted in. So this performs the same ownership check and hands back an
/// unpersisted [`MessageRecord`] carrying the prompt, which the caller appends
/// to the loaded history so the model still reads the question.
///
/// The returned record's `id` is a fresh UUID that matches no row on purpose.
/// Nothing on the turn path writes it: the assistant reply, the verdicts and
/// the background learning all key on the ASSISTANT message id, and the
/// `@handle` rewrite that does read a user-message id is inert here, because a
/// platform-composed prompt never carries a mention.
///
/// # Errors
///
/// Returns `AppError::NotFound` when the conversation does not exist or the
/// user is not a participant — the same refusal the athlete path gives, so a
/// proactive turn cannot answer into a thread its subject cannot read.
pub async fn resolve_platform_turn(
    database: &dyn ChatRepository,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
    content: &str,
) -> AppResult<UserMessageResult> {
    let conversation = database
        .get_conversation(conversation_id, user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation not found"))?;

    let message = MessageRecord {
        id: Uuid::new_v4().to_string(),
        conversation_id: conversation_id.to_owned(),
        role: "user".to_owned(),
        content: content.to_owned(),
        token_count: None,
        prompt_tokens: None,
        model: None,
        finish_reason: None,
        content_blocks: None,
        transport_policy: TransportPolicy::AnyTransport,
        created_at: Utc::now().to_rfc3339(),
    };

    Ok(UserMessageResult {
        message,
        conversation,
    })
}

/// Get the per-turn conversation history for LLM context building.
///
/// Returns the most recent `limit` messages in chronological order. The load is
/// bounded (not the full unbounded history) so a long thread doesn't build a
/// thousand-message vector before compaction trims it to the cap; `limit` is
/// derived from the compaction message cap with headroom, and is always
/// `>= max_messages` so the message-count backstop still governs the in-prompt
/// size. Read-only: the full history remains available for the UI and export
/// paths via `get_messages`.
///
/// Slash-command rows never leave this loader: a `/agent` picker or a
/// `/status` listing is the platform talking, and replayed as history it
/// would teach the model to answer in the platform's voice. The rows stay in
/// the transcript the UI reads; only the prompt is blind to them.
/// [`super::prompt_builder`] drops them by the same stamp for any caller that
/// assembles messages from rows it loaded itself.
///
/// Nor does anything before the newest divider: "+" and `/reset` open a new
/// topic inside the personal thread instead of a new thread (carnet#840), and
/// the model starts that topic over exactly as it would a fresh conversation.
/// A compaction summary of an earlier topic falls out with the rows it
/// covers, since a block is spliced only when its last row is in the window.
///
/// # Errors
///
/// Returns database errors on message retrieval failure.
pub async fn get_conversation_history(
    database: &dyn ChatRepository,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
    limit: i64,
) -> AppResult<Vec<MessageRecord>> {
    let mut history = database
        .get_recent_messages(
            conversation_id,
            user_id,
            tenant_id,
            limit,
            ai_scope::readable_policy(),
        )
        .await?;
    since_last_divider(&mut history);
    history.retain(|row| !row.is_command_turn());
    // A row derived from first-party-only data never reaches an external
    // turn's prompt; on Dravr's own surfaces it does, and stamps the turn
    // that replays it (carnet#769).
    ai_scope::retain_admitted(&mut history, |row| row.transport_policy);
    Ok(history)
}

/// Drop every row up to and including the newest divider in `history`, which
/// is in chronological order. A window with no divider is kept whole: either
/// the thread has none, or the newest one is older than the window, and every
/// row loaded is then after it.
fn since_last_divider(history: &mut Vec<MessageRecord>) {
    if let Some(divider) = history.iter().rposition(MessageRecord::is_divider) {
        history.drain(..=divider);
    }
}

/// Persist the assistant's response message.
///
/// Called after LLM dispatch + tool execution completes. Returns the
/// persisted message record and updated conversation so the caller can
/// observe `updated_at` / `summary` fields that moved as a result.
///
/// A reply to something the athlete sent advances their read marker past
/// itself, since they are in front of it, and a group-bound conversation fans
/// it out to the shared room transcript, attributed to the member it answered.
/// A reply the platform posted on its own initiative (`origin` is
/// [`TurnOrigin::Platform`]) does neither: the athlete has not seen it, so it
/// stays unread and badges the conversation, and it is their own coaching,
/// which no room asked for.
///
/// # Errors
///
/// Returns `AppError::Internal` if the conversation cannot be retrieved
/// after saving. Returns database errors on message persistence failure.
pub async fn persist_assistant_response(
    database: &dyn ChatRepository,
    groups: &dyn CoachingGroupRepository,
    params: &AddMessageParams<'_>,
    tenant_id: TenantId,
    origin: TurnOrigin,
) -> AppResult<(MessageRecord, ConversationRecord)> {
    let message = database.add_message(params).await?;
    let answers_the_athlete = origin == TurnOrigin::Athlete;
    if answers_the_athlete {
        advance_read_marker(
            database,
            params.conversation_id,
            params.user_id,
            tenant_id,
            &message.id,
        )
        .await;
    }

    let conversation = database
        .get_conversation(params.conversation_id, params.user_id, tenant_id)
        .await?
        .ok_or_else(|| AppError::internal("Failed to get updated conversation"))?;

    if answers_the_athlete {
        fan_out_to_group_transcript(
            groups,
            &conversation,
            tenant_id,
            params.user_id,
            TranscriptSpeaker::Coach,
            &message,
        )
        .await?;
    }

    Ok((message, conversation))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::models::DIVIDER_FINISH_REASON;

    fn row(id: &str, role: &str, finish_reason: Option<&str>) -> MessageRecord {
        MessageRecord {
            id: id.to_owned(),
            conversation_id: "thread".to_owned(),
            role: role.to_owned(),
            content: String::new(),
            token_count: None,
            prompt_tokens: None,
            model: None,
            finish_reason: finish_reason.map(str::to_owned),
            content_blocks: None,
            created_at: String::new(),
            transport_policy: TransportPolicy::AnyTransport,
        }
    }

    fn ids(history: &[MessageRecord]) -> Vec<&str> {
        history.iter().map(|m| m.id.as_str()).collect()
    }

    #[test]
    fn a_topic_starts_after_the_newest_divider() {
        let mut history = vec![
            row("q1", "user", None),
            row("a1", "assistant", None),
            row("d1", "system", Some(DIVIDER_FINISH_REASON)),
            row("q2", "user", None),
            row("d2", "system", Some(DIVIDER_FINISH_REASON)),
            row("q3", "user", None),
            row("a3", "assistant", None),
        ];
        since_last_divider(&mut history);
        assert_eq!(ids(&history), ["q3", "a3"]);
    }

    #[test]
    fn a_window_without_a_divider_is_kept_whole() {
        let mut history = vec![row("q1", "user", None), row("a1", "assistant", None)];
        since_last_divider(&mut history);
        assert_eq!(ids(&history), ["q1", "a1"]);
    }

    #[test]
    fn a_divider_with_nothing_after_it_leaves_an_empty_topic() {
        let mut history = vec![
            row("q1", "user", None),
            row("d1", "system", Some(DIVIDER_FINISH_REASON)),
        ];
        since_last_divider(&mut history);
        assert!(history.is_empty());
    }
}
