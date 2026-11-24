// ABOUTME: Closes a chat turn that ended without a reply, by writing the localized interrupted notice on its thread
// ABOUTME: Shared by the in-app send route and the messaging dispatcher, which both write into the personal thread

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The row that closes a turn which produced no reply.
//!
//! A turn that stored the sender's question and then ended without an answer
//! — refused by the LLM health gate, failed mid-turn, dropped by the shutdown
//! drain — leaves that question unanswered on the thread, and a reload reads
//! it as a reply still on its way for as long as the turn watchdog allows.
//! The localized `messaging.turn_interrupted` notice, stored as the
//! assistant's reply with finish reason `interrupted`, closes it instead.
//! A turn refused because no LLM tier answers gets
//! `messaging.turn_unavailable`: "send it again" is the wrong advice for a
//! turn that would be refused the same way (carnet#1709).
//!
//! The thread may be the athlete's personal one, which the app and their
//! messaging DM both write into (carnet#840), so both surfaces close their
//! failed turns here, the same way (carnet#1362). Only the transcript is
//! written: what a surface tells its own channel is that surface's business.

use pierre_chat_pipeline::stages::persistence::persist_assistant_response;
use pierre_chat_pipeline::turn_stop::TurnStop;
use pierre_chat_pipeline::TurnOrigin;
use pierre_contremaitre::messaging_strings::{KEY_TURN_INTERRUPTED, KEY_TURN_UNAVAILABLE};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::{AddMessageParams, TenantId};
use pierre_core::transport::TransportPolicy;
use tracing::warn;

use crate::mcp::resources::ServerContext;

/// The thread a turn ran on, as the notice is written into it.
pub(crate) struct TurnThread<'a> {
    /// Server resources: the chat and group repositories, the strings registry.
    pub(crate) resources: &'a ServerContext,
    /// The conversation the turn's question was stored in.
    pub(crate) conversation_id: &'a str,
    /// The participant whose turn it was.
    pub(crate) user_id: String,
    /// The tenant the conversation lives under.
    pub(crate) tenant_id: TenantId,
}

/// Why a turn ended without a reply, as the notice that closes it says it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FailedTurn {
    /// Cut short or failed mid-turn: sending the question again may work.
    Interrupted,
    /// Refused because the service behind the reply is unavailable: the LLM
    /// health gate while no tier answers, a provider that timed out, a chain
    /// whose last tier is out of quota or credit. Sending the question again
    /// at once would end the same way.
    Unavailable,
}

impl FailedTurn {
    /// The failure `error` is: `Unavailable` for the codes the health gate and
    /// an unavailable provider answer with — a chain whose every tier is out
    /// of quota or credit comes back as its last tier's `ExternalRateLimited`
    /// while the health status still reads `Healthy` — and `Interrupted` for
    /// anything else.
    pub(crate) fn of(error: &AppError) -> Self {
        if matches!(
            error.code,
            ErrorCode::ResourceUnavailable | ErrorCode::ExternalRateLimited
        ) {
            Self::Unavailable
        } else {
            Self::Interrupted
        }
    }

    /// The messaging string that words this failure.
    const fn notice_key(self) -> &'static str {
        match self {
            Self::Interrupted => KEY_TURN_INTERRUPTED,
            Self::Unavailable => KEY_TURN_UNAVAILABLE,
        }
    }
}

/// Close a failed turn whose question is on file.
///
/// A turn refused before it stored anything — a quota, a thread that could
/// not be reopened — has nothing on file to close, which `stop` says by
/// holding no question.
pub(crate) async fn close_failed_turn(
    thread: &TurnThread<'_>,
    stop: &TurnStop,
    why: FailedTurn,
    locale: &str,
) {
    if stop.question().is_none() {
        return;
    }
    close_turn_unless_answered(thread, why, locale).await;
}

/// Write the interrupted notice unless the thread's latest message is already
/// an assistant reply.
///
/// When an answer already landed (the turn ended inside its post-persist
/// stages), nothing is written: the answer is there.
pub(crate) async fn close_unanswered_turn(thread: &TurnThread<'_>, locale: &str) {
    close_turn_unless_answered(thread, FailedTurn::Interrupted, locale).await;
}

async fn close_turn_unless_answered(thread: &TurnThread<'_>, why: FailedTurn, locale: &str) {
    if turn_already_answered(thread).await {
        return;
    }
    write_notice(thread, why, locale).await;
}

/// Whether the conversation's latest message is already an assistant reply.
///
/// An unreadable conversation counts as unanswered: writing a notice next
/// to an answer is a duplicate line, leaving a question with nothing is the
/// silence this exists to end.
async fn turn_already_answered(thread: &TurnThread<'_>) -> bool {
    match thread
        .resources
        .common
        .repos
        .chat
        .get_recent_messages(
            thread.conversation_id,
            &thread.user_id,
            thread.tenant_id,
            1,
            TransportPolicy::FirstPartyOnly,
        )
        .await
    {
        Ok(latest) => latest.first().is_some_and(|m| m.role == "assistant"),
        Err(e) => {
            warn!(error = %e, conversation_id = thread.conversation_id, "could not read the interrupted turn's conversation; writing the notice anyway");
            false
        }
    }
}

/// Persist the localized notice for `why` as the assistant's reply. The
/// finish reason is `interrupted` either way: it says the row is a notice,
/// not an answer, which is all a reader of the thread acts on.
async fn write_notice(thread: &TurnThread<'_>, why: FailedTurn, locale: &str) {
    let repos = &thread.resources.common.repos;
    let notice = thread
        .resources
        .mcp
        .messaging_strings_registry
        .get(why.notice_key(), locale);
    let params = AddMessageParams {
        tenant_id: thread.tenant_id,
        conversation_id: thread.conversation_id,
        user_id: &thread.user_id,
        role: "assistant",
        content: &notice,
        token_count: None,
        finish_reason: Some("interrupted"),
        prompt_tokens: None,
        model: None,
        content_blocks: None,
        // Platform text that quotes nothing the turn read.
        transport_policy: TransportPolicy::AnyTransport,
        channel_type: None,
        about_user_id: None,
        turn_attempted_write: None,
        ai_consent_sources: &[],
    };
    if let Err(e) = persist_assistant_response(
        repos.chat.as_ref(),
        repos.groups.as_ref(),
        &params,
        thread.tenant_id,
        // The notice closes a turn the athlete sent and is waiting on.
        TurnOrigin::Athlete,
    )
    .await
    {
        warn!(error = %e, conversation_id = thread.conversation_id, "the interrupted-turn notice could not be written; the question stays unanswered on file");
    }
}

#[cfg(test)]
mod tests {
    use pierre_core::errors::{AppError, ErrorCode};

    use super::FailedTurn;

    /// The health gate's refusal and a provider that is unavailable are
    /// worded as unavailable; every other failure keeps the interrupted
    /// notice, whose "send it again" is then the right advice.
    #[test]
    fn only_an_unavailable_service_is_worded_as_unavailable() {
        let refused = AppError::new(
            ErrorCode::ResourceUnavailable,
            "Our AI provider (anthropic) is temporarily unavailable.",
        );
        assert_eq!(FailedTurn::of(&refused), FailedTurn::Unavailable);

        // A chain whose every tier is out of quota or credit fails the turn
        // with its last tier's code while the health status reads `Healthy`.
        let exhausted = AppError::new(
            ErrorCode::ExternalRateLimited,
            "LLM provider rate limited or out of quota: credit balance exhausted",
        );
        assert_eq!(FailedTurn::of(&exhausted), FailedTurn::Unavailable);

        for error in [
            AppError::internal("a stage panicked"),
            AppError::invalid_input("malformed request"),
        ] {
            assert_eq!(FailedTurn::of(&error), FailedTurn::Interrupted);
        }
    }
}
