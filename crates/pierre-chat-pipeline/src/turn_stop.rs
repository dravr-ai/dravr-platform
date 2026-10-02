// ABOUTME: How a running turn learns the athlete stopped it — the watch it keeps on its own question, and the envelope it ends on
// ABOUTME: Also the row vocabulary the stop request and the turn share: who wrote a question, which question a notice closed
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Stopping a turn on purpose.
//!
//! A turn is detached from the request that started it: a dropped stream
//! must not end it, so an explicit stop is its own signal, and one that
//! crosses instances — the stop request is a second HTTP call nothing routes
//! to the instance holding the turn. The signal is therefore a row: the
//! request writes the stopped notice, stamped
//! [`STOPPED_TURN_FINISH_REASON`], and the turn reads it.
//!
//! The notice is tied to one turn, not to the conversation's tail. Two
//! entries on the `content_blocks` rail carry the link, because
//! `chat_messages` has neither an author nor a turn column:
//!
//! - every athlete question is stored with a
//!   [`PersistedReplyBlock::TurnAuthor`] entry naming who wrote it, so a stop
//!   request resolves to the caller's own latest question and to no other
//!   participant's;
//! - the notice carries a [`PersistedReplyBlock::TurnStop`] entry naming the
//!   question it closes, so the turn answering that question finds its stop
//!   whatever tool rows, or other members' rows, landed in between.
//!
//! The turn looks for its stop in two places. While it works, a watch
//! re-reads the conversation every [`STOP_POLL_INTERVAL`] and ends the turn
//! by dropping its future. And at the point the reply is about to be stored
//! it checks once more, synchronously with the write, so a stop that landed
//! since the last poll still wins; past that check the turn is committed and
//! the watch stands down, so a reply that was stored is always delivered.
//!
//! A stopped turn ends on an ordinary envelope — the question, the notice as
//! the assistant's message — which is what lets the turn service run the
//! same usage accounting it runs for every served turn.

use std::future::pending;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    ConversationTurnId, LlmUsageRecord, PersistedReplyBlock, TenantId, STOPPED_TURN_FINISH_REASON,
};
use pierre_database::database::MessageRecord;
use pierre_database::repositories::ChatRepository;
use pierre_llm::TokenUsage;
use serde_json::Value;
use tokio::time::sleep;
use tracing::{info, warn};

use crate::envelope::{build_envelope, QuotaState, TurnEnvelope, TurnState, TurnTelemetry};
use crate::hooks::PipelineHooks;
use crate::surface_profile::SurfaceProfile;
use crate::turn::TurnInput;
use crate::ChatPipelineContext;

/// How often a running turn re-reads its conversation for a stop.
///
/// The athlete's client learns the outcome from the turn's own stream, so
/// this bounds how long the server keeps working on a reply nobody will read.
const STOP_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// How far back from the conversation's tail a stop is looked for.
///
/// A turn's question is followed by two rows per tool round and whatever the
/// other participants wrote meanwhile; this is several times the longest tool
/// loop. A question older than the window is not stoppable, and reads as such.
const STOP_SCAN_ROWS: i64 = 200;

/// Model label recorded for a stopped turn: no model's reply was delivered.
const STOPPED_MODEL: &str = "stopped";
/// Provider label for the same — the platform wrote the closing row.
const STOPPED_PROVIDER: &str = "platform";

/// Where one question stands, read off the rows that follow it.
#[derive(Debug, Clone)]
pub enum QuestionState {
    /// No reply and no stop: its turn may still be running.
    Unanswered,
    /// An assistant row answered it.
    Answered,
    /// A stopped notice names it. Carries the notice.
    Stopped(Box<MessageRecord>),
}

/// The `content_blocks` value a `user` row is stored with, naming its author.
#[must_use]
pub fn author_marker(user_id: &str) -> Option<String> {
    encode_marker(&PersistedReplyBlock::TurnAuthor {
        user_id: user_id.to_owned(),
    })
}

/// The `content_blocks` value a stopped notice is stored with, naming the
/// question it closes.
#[must_use]
pub fn stop_marker(question_id: &str) -> Option<String> {
    encode_marker(&PersistedReplyBlock::TurnStop {
        question_id: question_id.to_owned(),
    })
}

fn encode_marker(marker: &PersistedReplyBlock) -> Option<String> {
    serde_json::to_string(&[marker])
        .inspect_err(|e| warn!(error = %e, "turn marker could not be encoded"))
        .ok()
}

/// The non-visual entries a row's `content_blocks` holds. Visual specs fail
/// to decode as one and are skipped.
fn markers(row: &MessageRecord) -> Vec<PersistedReplyBlock> {
    row.content_blocks
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Vec<Value>>(raw).ok())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| serde_json::from_value(entry).ok())
        .collect()
}

/// Whether `row` is a question `user_id` wrote.
fn is_question_by(row: &MessageRecord, user_id: &str) -> bool {
    row.role == "user"
        && markers(row).iter().any(
            |marker| matches!(marker, PersistedReplyBlock::TurnAuthor { user_id: author } if author == user_id),
        )
}

/// Whether `row` is a stopped notice at all.
fn is_stop_notice(row: &MessageRecord) -> bool {
    row.role == "assistant" && row.finish_reason.as_deref() == Some(STOPPED_TURN_FINISH_REASON)
}

/// Whether `row` is the stopped notice closing `question_id`.
fn stops(row: &MessageRecord, question_id: &str) -> bool {
    is_stop_notice(row)
        && markers(row).iter().any(
            |marker| matches!(marker, PersistedReplyBlock::TurnStop { question_id: closed } if closed == question_id),
        )
}

/// The newest question `user_id` wrote among `rows` (oldest first).
#[must_use]
pub fn latest_question_by<'a>(
    rows: &'a [MessageRecord],
    user_id: &str,
) -> Option<&'a MessageRecord> {
    rows.iter().rev().find(|row| is_question_by(row, user_id))
}

/// Where the question `question_id` stands among `rows` (oldest first), or
/// `None` when it is not one of them.
///
/// A stop is recognised anywhere after the question: the notice names it, so
/// nothing that landed in between can hide it. A reply is recognised by
/// position — the first assistant row before the next `user` row — because a
/// reply row names nothing; tool rows and other questions' stopped notices
/// are passed over.
#[must_use]
pub fn question_state(rows: &[MessageRecord], question_id: &str) -> Option<QuestionState> {
    let position = rows.iter().position(|row| row.id == question_id)?;
    let after = &rows[position + 1..];
    if let Some(notice) = after.iter().find(|row| stops(row, question_id)) {
        return Some(QuestionState::Stopped(Box::new(notice.clone())));
    }
    let answered = after
        .iter()
        .take_while(|row| row.role != "user")
        .any(|row| row.role == "assistant" && !is_stop_notice(row));
    Some(if answered {
        QuestionState::Answered
    } else {
        QuestionState::Unanswered
    })
}

/// The conversation's tail, oldest first, as far back as a stop is looked for.
///
/// # Errors
///
/// Returns the repository's error when the conversation cannot be read.
pub async fn recent_rows(
    chat: &dyn ChatRepository,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
) -> AppResult<Vec<MessageRecord>> {
    chat.get_recent_messages(conversation_id, user_id, tenant_id, STOP_SCAN_ROWS)
        .await
}

/// A running turn's handle on its own stop.
///
/// Built by the surface that lets an athlete stop a turn and handed to the
/// pipeline on [`PipelineHooks::stop`]; a surface with no stop control leaves
/// the hook empty and its turns never read for one.
#[derive(Debug, Default)]
pub struct TurnStop {
    /// The question this turn answers, once the turn has stored it.
    question: OnceLock<MessageRecord>,
    /// Set once the turn has decided to store its reply.
    committed: AtomicBool,
}

/// What a stopped turn needs to describe itself as an envelope.
pub(crate) struct StopScope<'a> {
    pub(crate) ctx: &'a ChatPipelineContext,
    pub(crate) profile: &'a SurfaceProfile,
    pub(crate) conversation_id: &'a str,
    pub(crate) user_id: &'a str,
    pub(crate) tenant_id: TenantId,
    pub(crate) turn_id: ConversationTurnId,
    pub(crate) quota: &'a QuotaState,
}

impl TurnStop {
    /// A handle for one turn the athlete may stop.
    #[must_use]
    pub fn armed() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The question and the notice that closed it, when the stop is on file.
    ///
    /// A conversation that cannot be read is retried by the next look: a turn
    /// is never stopped on a database hiccup.
    async fn stop_on_file(&self, scope: &StopScope<'_>) -> Option<(MessageRecord, MessageRecord)> {
        let question = self.question.get()?;
        let rows = recent_rows(
            scope.ctx.repos.chat.as_ref(),
            scope.conversation_id,
            scope.user_id,
            scope.tenant_id,
        )
        .await
        .inspect_err(|e| warn!(error = %e, conversation_id = scope.conversation_id, "could not read the conversation for a stop; retrying"))
        .ok()?;
        match question_state(&rows, &question.id)? {
            QuestionState::Stopped(notice) => Some((question.clone(), *notice)),
            QuestionState::Unanswered | QuestionState::Answered => None,
        }
    }

    /// Resolve once the athlete has stopped this turn, with the envelope the
    /// stopped turn ends on.
    ///
    /// Never resolves once the turn has committed to its reply: from there the
    /// reply is the outcome, and a stop that lands later closes nothing.
    ///
    /// # Errors
    ///
    /// Returns an error when the stopped turn's conversation cannot be read
    /// back for the envelope.
    pub(crate) async fn stopped(&self, scope: &StopScope<'_>) -> AppResult<TurnEnvelope> {
        loop {
            sleep(STOP_POLL_INTERVAL).await;
            let stop = self.stop_on_file(scope).await;
            // Read again after the look, not only before it: the turn may have
            // committed while the read was in flight.
            if self.committed.load(Ordering::SeqCst) {
                pending::<()>().await;
            }
            if let Some((question, notice)) = stop {
                return stopped_envelope(scope, question, notice).await;
            }
        }
    }
}

/// Record the question this turn answers, so its stop can be found.
pub(crate) fn question_stored(hooks: &PipelineHooks<'_>, question: &MessageRecord) {
    if let Some(stop) = &hooks.stop {
        // Set once per turn; a second call has nothing to add.
        let _ = stop.question.set(question.clone());
    }
}

/// The last look before the reply is stored.
///
/// `Some` when the athlete stopped the turn: the caller returns the envelope
/// and stores nothing. `None` commits the turn to its reply and stands the
/// watch down.
///
/// # Errors
///
/// Returns an error when the stopped turn's conversation cannot be read back
/// for the envelope.
pub(crate) async fn stopped_before_reply(
    ctx: &ChatPipelineContext,
    hooks: &PipelineHooks<'_>,
    profile: &SurfaceProfile,
    input: &TurnInput,
) -> AppResult<Option<TurnEnvelope>> {
    let Some(stop) = &hooks.stop else {
        return Ok(None);
    };
    let scope = StopScope {
        ctx,
        profile,
        conversation_id: &input.conversation_id,
        user_id: &input.user_id,
        tenant_id: input.conversation_tenant_id,
        turn_id: input.turn_id,
        quota: &input.quota,
    };
    if let Some((question, notice)) = stop.stop_on_file(&scope).await {
        return stopped_envelope(&scope, question, notice).await.map(Some);
    }
    stop.committed.store(true, Ordering::SeqCst);
    Ok(None)
}

/// Describe a stopped turn as the envelope every turn ends on.
async fn stopped_envelope(
    scope: &StopScope<'_>,
    question: MessageRecord,
    notice: MessageRecord,
) -> AppResult<TurnEnvelope> {
    let conversation = scope
        .ctx
        .repos
        .chat
        .get_conversation(scope.conversation_id, scope.user_id, scope.tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found("Conversation not found"))?;
    info!(
        conversation_id = scope.conversation_id,
        turn_id = %scope.turn_id,
        "chat turn ended by the athlete's stop"
    );
    let content = notice.content.clone();
    Ok(build_envelope(
        scope.profile,
        TurnState {
            turn_id: scope.turn_id,
            // The markers are bookkeeping; the envelope's block walk would
            // read them as charts.
            user_message: MessageRecord {
                content_blocks: None,
                ..question
            },
            assistant_message: MessageRecord {
                content_blocks: None,
                ..notice
            },
            conversation,
            content,
            finish_reason: Some(STOPPED_TURN_FINISH_REASON.to_owned()),
            activity_list: None,
            telemetry: TurnTelemetry {
                model: STOPPED_MODEL.to_owned(),
                provider_name: STOPPED_PROVIDER.to_owned(),
                tools_called: Vec::new(),
                tool_calls_count: 0,
                activity_list_captured: false,
                activities_prefetched: false,
                usage: consumed_usage(scope).await,
                identity_leak: None,
                provider_warnings: Vec::new(),
            },
            quota: scope.quota.clone(),
            reconnect: None,
            verdict_chips: Vec::new(),
            scene_images: Vec::new(),
            actions: Vec::new(),
            actions_title: None,
            // The platform wrote this row, not the agent.
            answered_by: None,
        },
    ))
}

/// The tokens the turn's finished model calls spent before it was stopped.
///
/// Each call writes its own `llm_usage` row as it returns, keyed on the turn,
/// so the rows on file are exactly what the athlete's stop did not save them.
/// `None` when no call had finished — the turn service then charges the
/// question and the notice by estimate, as it does for any reply without a
/// provider count.
async fn consumed_usage(scope: &StopScope<'_>) -> Option<TokenUsage> {
    let calls = scope
        .ctx
        .repos
        .llm_usage
        .find_llm_usage_by_turn_id(scope.turn_id)
        .await
        .inspect_err(|e| warn!(error = %e, turn_id = %scope.turn_id, "could not read the stopped turn's usage rows"))
        .ok()?;
    if calls.is_empty() {
        return None;
    }
    let sum = |field: fn(&LlmUsageRecord) -> i64| {
        u32::try_from(calls.iter().map(field).sum::<i64>().max(0)).unwrap_or(u32::MAX)
    };
    Some(
        TokenUsage::new(
            sum(|call| call.prompt_tokens),
            sum(|call| call.completion_tokens),
            sum(|call| call.total_tokens),
        )
        .with_cache(
            Some(sum(|call| call.cached_tokens)),
            Some(sum(|call| call.cached_write_tokens)),
        )
        .with_reasoning(Some(sum(|call| call.reasoning_tokens))),
    )
}
