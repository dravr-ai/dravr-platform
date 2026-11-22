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
//!
//! A write the turn made before the stop went through, so it still earns its
//! envelope warning (D3, carnet#1519). The findings are read where they live
//! at the stop: those the tool loop handed back once it settled, kept on this
//! handle because the turn's future is dropped with its result, and those
//! still held under the turn's key in the store the dispatch chokepoint fills
//! for every loop. The notice rides on the stopped row itself, as it rides on
//! every other reply the platform writes in the model's place.

use pierre_core::transport::TransportPolicy;
use std::future::pending;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_contremaitre::anchor_strings::AnchorReader;
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    ConversationTurnId, LlmUsageRecord, PersistedReplyBlock, RoomQuote, StopState, TenantId,
    UnitSystem, STOPPED_TURN_FINISH_REASON,
};
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::database::{ConversationRecord, MessageRecord};
use pierre_database::repositories::ChatRepository;
use pierre_llm::TokenUsage;
use pierre_tool_runtime::envelope_notices::{merge_notice_days, NoticeDay};
use pierre_tool_runtime::guardian::TurnKey;
use serde_json::Value;
use tokio::runtime::Handle;
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::{self, sleep};
use tracing::{debug, info, warn};

use crate::envelope::{build_envelope, QuotaState, TurnEnvelope, TurnState, TurnTelemetry};
use crate::hooks::PipelineHooks;
use crate::stages::envelope_notice::{notice_reader, stop_notice_blocks};
use crate::stages::units::reader_units;
use crate::supervised::{spawn_supervised, TaskOwner};
use crate::surface_profile::SurfaceProfile;
use crate::turn::TurnInput;
use crate::ChatPipelineContext;

/// How often a running turn re-reads its conversation for a stop.
///
/// The athlete's client learns the outcome from the turn's own stream, so
/// this bounds how long the server keeps working on a reply nobody will read.
const STOP_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// How long the stop request waits for the stopped turn to settle its notice:
/// several of the turn's own polls, past which no turn is reading it.
const STOP_SETTLE_WAIT: Duration = Duration::from_secs(5);

/// How often the stop request re-reads while it waits.
const STOP_SETTLE_POLL: Duration = Duration::from_millis(100);

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

/// The `content_blocks` value a `user` row is stored with: the entry naming
/// its author, then the room words it quotes, when it quotes any
/// (carnet#1249).
#[must_use]
pub fn question_blocks(user_id: &str, quote: Option<&RoomQuote>) -> Option<String> {
    let author = PersistedReplyBlock::TurnAuthor {
        user_id: user_id.to_owned(),
    };
    let mut entries = vec![author];
    entries.extend(quote.cloned().map(PersistedReplyBlock::RoomQuote));
    serde_json::to_string(&entries)
        .inspect_err(|e| warn!(error = %e, "question blocks could not be encoded"))
        .ok()
}

/// The `content_blocks` value a stopped notice is stored with, naming the
/// question it closes.
#[must_use]
pub fn stop_marker(question_id: &str) -> Option<String> {
    encode_marker(&stop_entry(question_id, StopState::Pending))
}

/// The `content_blocks` value a stopped notice holds once its turn settled it
/// and the turn's writes found nothing.
pub(crate) fn settled_stop_marker(question_id: &str) -> Option<String> {
    encode_marker(&stop_entry(question_id, StopState::Settled))
}

/// The entry naming the question a stopped notice closes.
fn stop_entry(question_id: &str, state: StopState) -> PersistedReplyBlock {
    PersistedReplyBlock::TurnStop {
        question_id: question_id.to_owned(),
        state,
    }
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
            |marker| matches!(marker, PersistedReplyBlock::TurnStop { question_id: closed, .. } if closed == question_id),
        )
}

/// Whether `row` is the stopped notice closing `question_id`, settled by its
/// turn.
fn settles(row: &MessageRecord, question_id: &str) -> bool {
    is_stop_notice(row)
        && markers(row).iter().any(|marker| {
            matches!(marker, PersistedReplyBlock::TurnStop { question_id: closed, state: StopState::Settled } if closed == question_id)
        })
}

/// Whether nothing more will be written for the stop closing `question_id`.
///
/// Read among `rows` (oldest first): its turn settled the notice, or answered
/// the question before it read the stop and will never settle it. A question
/// no longer among them has nothing left to wait for.
fn stop_is_final(rows: &[MessageRecord], question_id: &str) -> bool {
    let Some(position) = rows.iter().position(|row| row.id == question_id) else {
        return true;
    };
    let after = &rows[position + 1..];
    after.iter().any(|row| settles(row, question_id))
        || after
            .iter()
            .take_while(|row| row.role != "user")
            .any(|row| row.role == "assistant" && !is_stop_notice(row))
}

/// Wait until the turn the athlete stopped has settled its notice.
///
/// The stop request then answers with the notice whole, and a re-read right
/// after it shows the envelope notice the turn's writes earned (carnet#1519).
///
/// A live turn watching its question reads the stop within
/// `STOP_POLL_INTERVAL` and settles it at once, or meets it at its last look
/// before storing a reply. The wait ends there, or when the turn answered
/// before it read the stop, or after `STOP_SETTLE_WAIT` when no turn reads
/// the question any more — an instance that died, a surface whose turns are
/// not watched — which leaves nothing more to be written. A read that fails
/// is retried by the next look.
pub async fn wait_for_the_stop_to_settle(
    chat: &dyn ChatRepository,
    conversation_id: &str,
    user_id: &str,
    tenant_id: TenantId,
    question_id: &str,
) {
    // The cap bounds the whole wait, a read that hangs included.
    let settled = time::timeout(STOP_SETTLE_WAIT, async {
        loop {
            match recent_rows(chat, conversation_id, user_id, tenant_id).await {
                Ok(rows) if stop_is_final(&rows, question_id) => return,
                Ok(_) => {}
                Err(e) => {
                    warn!(error = %e, conversation_id, "could not read the conversation for the stop's settling; retrying");
                }
            }
            sleep(STOP_SETTLE_POLL).await;
        }
    })
    .await;
    if settled.is_err() {
        warn!(
            conversation_id,
            question_id, "no turn settled the stop; answering with the notice as written"
        );
    }
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

/// The question of `user_id`'s whose turn may still be running among `rows`
/// (oldest first): their newest question, while it has neither a reply nor a
/// stop and was asked less than `ceiling` before `now`.
///
/// The same question `POST …/stop` would close, so a client told a reply is
/// coming can always stop it. `ceiling` is the longest a live turn can run:
/// past it, an unanswered question is a turn that died without writing a row
/// (an instance killed past the shutdown drain), and saying a reply is coming
/// would keep a client waiting on nothing. A question whose `created_at` does
/// not parse cannot be aged, so it is not reported as awaiting either.
#[must_use]
pub fn awaiting_question<'a>(
    rows: &'a [MessageRecord],
    user_id: &str,
    now: Timestamp,
    ceiling: Duration,
) -> Option<&'a MessageRecord> {
    let question = latest_question_by(rows, user_id)?;
    if !matches!(
        question_state(rows, &question.id),
        Some(QuestionState::Unanswered)
    ) {
        return None;
    }
    let asked_at = Timestamp::parse_from_rfc3339(&question.created_at)
        .inspect_err(|e| {
            warn!(error = %e, question_id = question.id.as_str(), "question timestamp does not parse; not reporting it as awaiting");
        })
        .ok()?;
    let age = now.signed_duration_since(asked_at);
    let ceiling = Elapsed::from_std(ceiling).unwrap_or(Elapsed::MAX);
    (age <= ceiling).then_some(question)
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
    chat.get_recent_messages(
        conversation_id,
        user_id,
        tenant_id,
        STOP_SCAN_ROWS,
        TransportPolicy::FirstPartyOnly,
    )
    .await
}

/// A running turn's handle on its own stop.
///
/// Built by the surface that lets an athlete stop a turn ([`Self::armed`]) and
/// handed to the pipeline on [`PipelineHooks::stop`]. A surface with no stop
/// control that still closes a failed turn's question builds one that only
/// records it ([`Self::recording`]); its turns never read for a stop. A
/// surface needing neither leaves the hook empty.
#[derive(Debug, Default)]
pub struct TurnStop {
    /// The question this turn answers, once the turn has stored it.
    question: OnceLock<MessageRecord>,
    /// Set once the turn has decided to store its reply.
    committed: AtomicBool,
    /// Whether the turn only records its question or also watches it for a
    /// stopped notice.
    mode: StopMode,
    /// The envelope findings the turn's writes reported that reach its stop:
    /// what the settled tool loop handed back (a stop drops the turn's future
    /// and its result with it, and the findings already left the store when
    /// the loop settled), what the store held at the stop, and what a
    /// headless call still running reports after it.
    findings: Mutex<Vec<NoticeDay>>,
    /// Held while the stopped row is written, so the last write is the one
    /// made from every finding: the watch, the turn and a late headless
    /// finding can each write it.
    row: AsyncMutex<()>,
}

/// What a turn does with its [`TurnStop`].
#[derive(Debug, Default, Clone, Copy)]
enum StopMode {
    /// Only records the question the turn stores; never reads for a stop.
    #[default]
    Recording,
    /// Watches the stored question for the athlete's stopped notice.
    Armed,
}

/// What a stopped turn needs to describe itself as an envelope.
pub(crate) struct StopScope<'a> {
    pub(crate) ctx: &'a ChatPipelineContext,
    pub(crate) profile: &'a SurfaceProfile,
    pub(crate) conversation_id: &'a str,
    pub(crate) user_id: &'a str,
    pub(crate) tenant_id: TenantId,
    /// The tenant the turn's tools ran in, which keys what they recorded.
    pub(crate) tool_tenant_id: TenantId,
    pub(crate) turn_id: ConversationTurnId,
    pub(crate) quota: &'a QuotaState,
}

impl TurnStop {
    /// A handle for one turn the athlete may stop.
    #[must_use]
    pub fn armed() -> Arc<Self> {
        Arc::new(Self {
            mode: StopMode::Armed,
            ..Self::default()
        })
    }

    /// A handle that only records the question a turn stores, on a surface
    /// with no stop control: the turn never reads for a stop, and the surface
    /// learns whether a failed turn left a question to close (carnet#1362).
    #[must_use]
    pub fn recording() -> Arc<Self> {
        Arc::new(Self {
            mode: StopMode::Recording,
            ..Self::default()
        })
    }

    /// Whether the turn watches its question for the athlete's stop.
    #[must_use]
    pub const fn watches(&self) -> bool {
        matches!(self.mode, StopMode::Armed)
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

    /// The question this turn stored, once it has stored one.
    ///
    /// `None` after a turn refused before its question was written — a quota,
    /// a thread that could not be reopened — which leaves nothing on file to
    /// close.
    #[must_use]
    pub fn question(&self) -> Option<&MessageRecord> {
        self.question.get()
    }

    /// Fold `days` into the findings that reach this turn's stop, each later
    /// report replacing what it supersedes.
    fn fold(&self, days: Vec<NoticeDay>) {
        if days.is_empty() {
            return;
        }
        let mut held = self.findings.lock().unwrap_or_else(PoisonError::into_inner);
        merge_notice_days(&mut held, days);
    }

    /// Every finding that reached this turn's stop so far. Read without being
    /// emptied: the watch and the turn can both reach the stop — the turn is
    /// polled while the watch builds its envelope — and each must write the
    /// row from every finding.
    fn findings(&self) -> Vec<NoticeDay> {
        self.findings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
    pub(crate) async fn stopped(
        self: &Arc<Self>,
        scope: &StopScope<'_>,
    ) -> AppResult<TurnEnvelope> {
        loop {
            sleep(STOP_POLL_INTERVAL).await;
            let stop = self.stop_on_file(scope).await;
            // Read again after the look, not only before it: the turn may have
            // committed while the read was in flight.
            if self.committed.load(Ordering::SeqCst) {
                pending::<()>().await;
            }
            if let Some((question, notice)) = stop {
                return stopped_envelope(scope, self, question, notice).await;
            }
        }
    }
}

/// Record the question this turn answers, so its stop can be found.
pub(crate) fn question_stored(hooks: &PipelineHooks<'_>, question: &MessageRecord) {
    if let Some(stop) = &hooks.stop {
        // Set once per turn; a second call has nothing to add.
        if stop.question.set(question.clone()).is_err() {
            debug!("turn question already recorded; keeping the first");
        }
    }
}

/// Keep the envelope findings the settled tool loop handed back, so a stop
/// that lands before the reply is stored still reaches them.
pub(crate) fn tool_loop_settled(hooks: &PipelineHooks<'_>, days: &[NoticeDay]) {
    if let Some(stop) = hooks.stop.as_ref().filter(|stop| stop.watches()) {
        stop.fold(days.to_vec());
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
    let Some(stop) = hooks.stop.as_ref().filter(|stop| stop.watches()) else {
        return Ok(None);
    };
    let scope = StopScope {
        ctx,
        profile,
        conversation_id: &input.conversation_id,
        user_id: &input.user_id,
        tenant_id: input.conversation_tenant_id,
        tool_tenant_id: input.tool_tenant_id,
        turn_id: input.turn_id,
        quota: &input.quota,
    };
    if let Some((question, notice)) = stop.stop_on_file(&scope).await {
        return stopped_envelope(&scope, stop, question, notice)
            .await
            .map(Some);
    }
    stop.committed.store(true, Ordering::SeqCst);
    Ok(None)
}

/// Describe a stopped turn as the envelope every turn ends on.
async fn stopped_envelope(
    scope: &StopScope<'_>,
    stop: &Arc<TurnStop>,
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
    let notice_blocks = stopped_notice_blocks(scope, stop, &conversation, &question, &notice).await;
    Ok(build_envelope(
        scope.profile,
        TurnState {
            turn_id: scope.turn_id,
            // The markers are bookkeeping; the envelope's block walk would
            // read them as charts. The notice carries the envelope notice
            // alone, when the turn's writes earned one.
            user_message: MessageRecord {
                content_blocks: None,
                ..question
            },
            assistant_message: MessageRecord {
                content_blocks: notice_blocks,
                ..notice
            },
            conversation,
            content,
            finish_reason: Some(STOPPED_TURN_FINISH_REASON.to_owned()),
            activity_list: None,
            introduction_end: None,
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

/// Settle the stopped row: store the envelope notice for the writes the
/// stopped turn made — `None` for the envelope when they found nothing — and
/// mark the notice settled, which is what the stop request waits for.
///
/// The findings are what the settled loop handed back and what the store
/// held under the turn's key, which the stop takes and seals: a headless tool
/// call still running reports into the sink it leaves, which settles the row
/// again, so a late finding is neither dropped nor left in the store.
async fn stopped_notice_blocks(
    scope: &StopScope<'_>,
    stop: &Arc<TurnStop>,
    conversation: &ConversationRecord,
    question: &MessageRecord,
    notice: &MessageRecord,
) -> Option<String> {
    // A pace is written in the units the notice's reader reads, by the rule
    // the reply's notice follows (`recovery`).
    let units = reader_units(
        &scope.ctx.repos,
        parse_uuid(scope.user_id).unwrap_or_default(),
        conversation.group_id.is_some(),
    )
    .await;
    let row = Arc::new(StopRow {
        chat: Arc::clone(&scope.ctx.repos.chat),
        strings: Arc::clone(&scope.ctx.messaging_strings_registry),
        locale: scope.profile.locale.clone(),
        reader: notice_reader(conversation),
        units,
        question_id: question.id.clone(),
        notice_id: notice.id.clone(),
        conversation_id: scope.conversation_id.to_owned(),
        user_id: scope.user_id.to_owned(),
        tenant_id: scope.tenant_id,
        turn_id: scope.turn_id,
    });
    let key = TurnKey::new(
        Some(scope.tool_tenant_id.as_uuid()),
        scope.turn_id.0.to_string(),
    );
    let late = {
        let (row, stop) = (Arc::clone(&row), Arc::clone(stop));
        move |days: Vec<NoticeDay>| row.settle_late(&stop, days)
    };
    stop.fold(
        scope
            .ctx
            .tool_runtime
            .envelope_notices()
            .take_and_seal(&key, late),
    );
    row.settle(stop).await
}

/// A stopped row and what writing its notice needs, owned, so a finding a
/// headless call reports after the turn ended can settle it again.
struct StopRow {
    chat: Arc<dyn ChatRepository>,
    strings: Arc<MessagingStringsRegistry>,
    locale: String,
    reader: AnchorReader,
    units: UnitSystem,
    question_id: String,
    notice_id: String,
    conversation_id: String,
    user_id: String,
    tenant_id: TenantId,
    /// The stopped turn, named when a late settle fails.
    turn_id: ConversationTurnId,
}

impl StopRow {
    /// Write the row from every finding that reached `stop`, and return the
    /// envelope's blocks.
    ///
    /// The row is set whole — its settled marker, then the notice when the
    /// writes earned one — under the handle's row lock, after reading the
    /// findings: whichever writer goes last writes every finding, and the
    /// notice is stored once. A write that fails is logged: the live envelope
    /// still carries the notice, and the plan card's per-day badge shows the
    /// finding on a reload.
    async fn settle(&self, stop: &TurnStop) -> Option<String> {
        let writing = stop.row.lock().await;
        let blocks = stop_notice_blocks(
            &self.strings,
            &self.locale,
            self.reader,
            self.units,
            &self.question_id,
            &stop.findings(),
        );
        let stored = blocks
            .as_ref()
            .map(|blocks| blocks.row.clone())
            .or_else(|| settled_stop_marker(&self.question_id));
        if let Some(stored) = stored {
            match self
                .chat
                .set_stopped_notice_blocks(
                    &self.notice_id,
                    &self.conversation_id,
                    &self.user_id,
                    self.tenant_id,
                    &stored,
                )
                .await
            {
                Ok(true) => {}
                Ok(false) => warn!(
                    conversation_id = self.conversation_id.as_str(),
                    notice_id = self.notice_id.as_str(),
                    "the stopped notice was not found to settle"
                ),
                Err(e) => warn!(
                    error = %e,
                    conversation_id = self.conversation_id.as_str(),
                    "the stopped notice could not be settled"
                ),
            }
        }
        // The row is written; the next writer may read the findings now.
        drop(writing);
        blocks.map(|blocks| blocks.envelope)
    }

    /// A finding a headless call reported after the stop: fold it in and
    /// settle the row again, on the runtime the call reported from.
    fn settle_late(self: &Arc<Self>, stop: &Arc<TurnStop>, days: Vec<NoticeDay>) {
        stop.fold(days);
        let (row, stop) = (Arc::clone(self), Arc::clone(stop));
        match Handle::try_current() {
            Ok(runtime) => {
                let owner = TaskOwner::Turn(row.turn_id);
                spawn_supervised(&runtime, "stopped_notice_late_settle", owner, async move {
                    row.settle(&stop).await;
                });
            }
            Err(e) => warn!(
                error = %e,
                conversation_id = row.conversation_id.as_str(),
                "a finding reported after the stop found no runtime to settle the notice on"
            ),
        }
    }
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
        .with_cache_write_1h(Some(sum(|call| call.cached_write_1h_tokens)))
        .with_reasoning(Some(sum(|call| call.reasoning_tokens))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_tool_runtime::envelope_notices::{EnvelopeNotices, NoticeSource};

    const QUESTION: &str = "construis-moi un plan pour le marathon";

    fn question_at(created_at: &str) -> MessageRecord {
        MessageRecord {
            id: "q".to_owned(),
            conversation_id: "conv".to_owned(),
            role: "user".to_owned(),
            content: QUESTION.to_owned(),
            token_count: None,
            prompt_tokens: None,
            model: None,
            finish_reason: None,
            content_blocks: question_blocks("alice", None),
            created_at: created_at.to_owned(),
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: None,
            about_user_id: None,
            turn_attempted_write: None,
        }
    }

    /// The ceiling is inclusive, and a question that cannot be aged is not
    /// reported: a client told to wait must have a turn that can still answer.
    #[test]
    fn the_ceiling_is_measured_from_the_questions_own_timestamp() {
        let asked = Timestamp::parse_from_rfc3339("2026-10-26T10:00:00Z").unwrap();
        let ceiling = Duration::from_mins(16);
        let rows = vec![question_at("2026-10-26T10:00:00Z")];

        let at_ceiling = asked + Elapsed::minutes(16);
        assert_eq!(
            awaiting_question(&rows, "alice", at_ceiling, ceiling).map(|q| q.id.as_str()),
            Some("q"),
            "a question exactly at the ceiling is still awaited"
        );
        let past = at_ceiling + Elapsed::seconds(1);
        assert!(
            awaiting_question(&rows, "alice", past, ceiling).is_none(),
            "a question past the ceiling is a turn that died"
        );
        assert!(
            awaiting_question(&rows, "bob", asked, ceiling).is_none(),
            "only the author waits on a question"
        );

        let unreadable = vec![question_at("yesterday")];
        assert!(
            awaiting_question(&unreadable, "alice", asked, ceiling).is_none(),
            "a question that cannot be aged is not reported"
        );
    }

    fn warned_day(date: &str) -> NoticeDay {
        let finding = serde_json::from_value(serde_json::json!({
            "code": "cycle_mean_floor", "action": "warn", "message": "English, for the model.",
            "measured": 71.6, "bound": 80.0
        }))
        .expect("a finding");
        NoticeDay {
            date: date.to_owned(),
            findings: vec![finding],
            relabelled_purpose: None,
            anchor: None,
            source: NoticeSource::PlanDay,
        }
    }

    fn dates(days: &[NoticeDay]) -> Vec<&str> {
        days.iter().map(|day| day.date.as_str()).collect()
    }

    /// The findings of a stopped turn are what its settled loop handed back,
    /// what the store held under its key, and what a headless call reports
    /// after the stop; the stop empties the store and seals the key, and the
    /// handle is read without being emptied, so the watch and the turn both
    /// see every finding and nothing outlives the turn.
    #[test]
    fn a_stop_gathers_every_finding_and_leaves_no_entry_behind() {
        let stop = TurnStop::armed();
        let hooks = PipelineHooks {
            stop: Some(Arc::clone(&stop)),
            ..PipelineHooks::none()
        };
        tool_loop_settled(&hooks, &[warned_day("2026-07-14")]);

        let notices = EnvelopeNotices::new();
        let key = TurnKey::new(None, "stopped-turn");
        let other = TurnKey::new(None, "other-turn");
        notices.record(&key, vec![warned_day("2026-07-16")]);
        notices.record(&other, vec![warned_day("2026-07-18")]);

        let late = Arc::clone(&stop);
        stop.fold(notices.take_and_seal(&key, move |days| late.fold(days)));
        assert_eq!(
            dates(&stop.findings()),
            vec!["2026-07-14", "2026-07-16"],
            "bug: the stop reads the settled loop's findings, then the store's"
        );
        notices.record(&key, vec![warned_day("2026-07-20")]);
        assert!(
            notices.take(&key).is_empty(),
            "bug: a stopped turn leaves no entry in the store, a late one included"
        );
        assert_eq!(
            dates(&stop.findings()),
            vec!["2026-07-14", "2026-07-16", "2026-07-20"],
            "bug: a finding reported after the stop reaches the stopped turn"
        );
        assert_eq!(
            dates(&stop.findings()),
            vec!["2026-07-14", "2026-07-16", "2026-07-20"],
            "bug: the second reader of the same stop must see what the first saw"
        );
        assert_eq!(
            dates(&notices.take(&other)),
            vec!["2026-07-18"],
            "bug: a stop takes only its own turn's findings"
        );
    }

    /// Only a turn that watches for a stop keeps its findings on the handle:
    /// a recording one is never stopped, so it has nothing to keep them for.
    #[test]
    fn only_a_watched_turn_keeps_its_settled_findings() {
        let recording = TurnStop::recording();
        let hooks = PipelineHooks {
            stop: Some(Arc::clone(&recording)),
            ..PipelineHooks::none()
        };
        tool_loop_settled(&hooks, &[warned_day("2026-07-14")]);
        assert!(
            recording.findings().is_empty(),
            "bug: a recording handle keeps no findings"
        );
    }

    fn row(
        id: &str,
        role: &str,
        finish_reason: Option<&str>,
        blocks: Option<String>,
    ) -> MessageRecord {
        MessageRecord {
            id: id.to_owned(),
            role: role.to_owned(),
            finish_reason: finish_reason.map(ToOwned::to_owned),
            content_blocks: blocks,
            ..question_at("2026-10-26T10:00:00Z")
        }
    }

    /// The stop request waits for exactly this: the turn settled the notice,
    /// or answered before it read the stop; an unsettled notice is still owed
    /// its final blocks.
    #[test]
    fn a_stop_is_final_once_its_turn_settled_it_or_answered() {
        let question = question_at("2026-10-26T10:00:00Z");
        let stopped = |state: StopState| {
            let blocks = match state {
                StopState::Settled => settled_stop_marker("q"),
                StopState::Pending => stop_marker("q"),
            };
            row(
                "notice",
                "assistant",
                Some(STOPPED_TURN_FINISH_REASON),
                blocks,
            )
        };
        assert!(
            !stop_is_final(&[question.clone(), stopped(StopState::Pending)], "q"),
            "bug: a notice its turn has not settled may still gain the envelope notice"
        );
        assert!(
            stop_is_final(&[question.clone(), stopped(StopState::Settled)], "q"),
            "bug: a notice its turn settled is final; the stop request stops waiting"
        );
        assert!(
            stop_is_final(
                &[
                    question.clone(),
                    row("reply", "assistant", Some("stop"), None),
                    stopped(StopState::Pending)
                ],
                "q"
            ),
            "bug: a turn that answered before the stop never settles it; nothing is owed"
        );
        assert!(
            matches!(
                question_state(&[question, stopped(StopState::Settled)], "q"),
                Some(QuestionState::Stopped(_))
            ),
            "bug: a settled notice still closes its question"
        );
        assert!(
            stop_is_final(&[stopped(StopState::Pending)], "q"),
            "a question out of the rows read has nothing left to wait for"
        );
    }

    /// The stop marker's wire shape: pending is the field left out, as every
    /// notice stored before the state existed reads, and settled names it.
    #[test]
    fn the_stop_marker_omits_a_pending_state_and_names_a_settled_one() {
        let pending: Value =
            serde_json::from_str(&stop_marker("q").expect("a marker")).expect("json");
        assert_eq!(
            pending,
            serde_json::json!([{ "type": "turn_stop", "question_id": "q" }]),
            "bug: the stop request's marker reads as before the state existed"
        );
        let settled: Value =
            serde_json::from_str(&settled_stop_marker("q").expect("a marker")).expect("json");
        assert_eq!(
            settled,
            serde_json::json!([{ "type": "turn_stop", "question_id": "q", "state": "settled" }]),
            "bug: a settled marker names its state in snake_case"
        );
        let stored: PersistedReplyBlock =
            serde_json::from_value(serde_json::json!({ "type": "turn_stop", "question_id": "q" }))
                .expect("a marker stored before the state existed");
        assert_eq!(
            stored,
            stop_entry("q", StopState::Pending),
            "bug: a marker with no state reads as pending"
        );
    }
}
