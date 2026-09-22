// ABOUTME: Agent introduction — an agent names itself and its role the first time it answers in a thread
// ABOUTME: Decided from a per-thread ledger and the store's localized title; recorded once a reply names the agent
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The agent introduction (carnet#501).
//!
//! An athlete talking to an agent is owed one line saying who is answering and
//! what it is there for. On 2026-09-21 23:46Z a web chat with the Half Marathon
//! Agent opened on « Montre-moi mes sorties de mars 2026 avec le dénivelé. »
//! and the reply went straight into « Mars a été un mois de fond… » — no name,
//! no role. No platform stage and no dravr-contremaitre prompt had ever asked
//! for one, so whether a reply opened with it was left to the model.
//!
//! The platform decides it now, from two facts the model does not hold.
//!
//! **Has this agent been introduced in this thread?** The transcript cannot
//! say. A messaging DM is one long-lived conversation that is re-pointed at
//! whichever agent the athlete selected last, `/agent add` rebinds a web
//! thread in place, and an `@handle` hands a single turn to another agent — so
//! "no reply yet" is true of the thread's first agent only. Platform-authored
//! rows (an interrupted-turn notice, a guided walk's opener, a backfill notice)
//! are `assistant` rows that introduced nobody. And a shared room keeps one
//! conversation row per member. So the fact is kept per agent per thread in
//! `agent_introductions`, where the thread is the room for a group-bound
//! conversation and the conversation otherwise.
//!
//! **What is the agent called, in this athlete's language?** The store's own
//! answer: the `agent_translations` overlay for the turn's locale, the title
//! the athlete tapped, with `agents.title` for an agent that has none.
//!
//! The introduction is recorded only once a delivered reply has actually
//! named the agent in its opening. A reply the Guardian or the reconnect
//! re-challenge replaced, one withheld at the response boundary, or one whose
//! model simply ignored the instruction introduced nobody, so the next reply
//! is asked again — and once one has, no later reply in the thread is.
//!
//! A turn with no agent bound is Dravr answering in its own voice under the
//! identity anchor, as it always has, and carries no introduction.

use pierre_core::models::{AgentRuntimeContext, ConversationRecord, TenantId};
use pierre_core::uuid_utils::parse_uuid;
use tracing::{info, warn};

use crate::turn::TurnInput;
use crate::ChatPipelineContext;

/// How much of a reply's head is read for the agent's name.
///
/// The introduction is one short sentence asked to open the reply. A title
/// that only turns up further down is the agent mentioned in passing, and a
/// short title ("Coach") would otherwise match almost any reply.
const OPENING_WINDOW_CHARS: usize = 300;

/// An introduction the turn's reply is asked to open with, and what is
/// recorded once a reply delivers it.
#[derive(Debug, Clone)]
pub(crate) struct PendingIntroduction {
    thread_id: String,
    agent_id: String,
    title: String,
}

impl PendingIntroduction {
    /// The system-prompt line asking for the introduction.
    pub(crate) fn directive(&self) -> String {
        introduction_directive(&self.title)
    }
}

/// The thread an introduction belongs to.
///
/// A group-bound conversation is one member's row in a shared room, and the
/// room is what heard the agent, so it is the group id. Any other
/// conversation is its own thread.
#[must_use]
pub fn introduction_thread(conv: &ConversationRecord) -> &str {
    conv.group_id.as_deref().unwrap_or(&conv.id)
}

/// The title an agent introduces itself by: the localized one when the store
/// has it, else the canonical `agents.title`. `None` when both are blank.
///
/// Whitespace is collapsed to single spaces, because the title is spliced into
/// a one-sentence instruction and a line break inside it would split that.
#[must_use]
pub fn display_title(localized: Option<&str>, canonical: &str) -> Option<String> {
    let localized = localized
        .map(collapse_whitespace)
        .filter(|title| !title.is_empty());
    let title = localized.unwrap_or_else(|| collapse_whitespace(canonical));
    (!title.is_empty()).then_some(title)
}

/// The instruction that opens a reply with the agent's introduction.
///
/// Held to the constraints of the turn directives it follows: it states a task
/// for this reply and asserts no identity, sets no format and names no
/// language (Stage 7g.3b owns that). It introduces the agent as Dravr's, so
/// it asks for nothing the identity anchor at the tail of the prompt forbids.
/// It follows whichever directive owns Stage 7g.3 — an ordinary turn, a guided
/// probe, the release after an interview — so the rest of the reply is still
/// that directive's. A persona whose first turn starts with tool calls keeps
/// them first: the sentence opens the text, not the turn.
#[must_use]
pub fn introduction_directive(title: &str) -> String {
    format!(
        "\nYou have not introduced yourself in this conversation yet. Open the text of this \
         reply with one short sentence introducing yourself as Dravr's {title} and saying what \
         you help with, then carry on with this turn as the instructions above ask. Any tool \
         call you need still comes before that sentence."
    )
}

/// Whether the opening of `reply` names the agent titled `title`.
///
/// Compared on letters and digits alone, case-folded, so « l'Agent
/// Semi Marathon » names « Agent Semi-Marathon ».
#[must_use]
pub fn reply_names_agent(reply: &str, title: &str) -> bool {
    let wanted = fold(title);
    if wanted.is_empty() {
        return false;
    }
    let opening: String = reply.chars().take(OPENING_WINDOW_CHARS).collect();
    fold(&opening).contains(&wanted)
}

/// The introduction this turn's reply is asked to open with, or `None`.
///
/// `None` when no agent answers the turn, when that agent has already been
/// introduced in this thread, or when the ledger cannot be read — a reply
/// without an introduction is recoverable on the next turn, a repeated one is
/// not.
pub(crate) async fn resolve(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    conv: &ConversationRecord,
    agent: Option<&AgentRuntimeContext>,
    locale: &str,
) -> Option<PendingIntroduction> {
    let agent = agent?;
    let agent_id = input.turn_agent_id(conv)?;
    let thread_id = introduction_thread(conv);
    match ctx
        .repos
        .chat
        .has_agent_introduction(thread_id, agent_id, input.conversation_tenant_id)
        .await
    {
        Ok(false) => {}
        Ok(true) => return None,
        Err(e) => {
            warn!(error = %e, agent_id, "introduction ledger unreadable; this reply carries no introduction");
            return None;
        }
    }
    let localized = localized_title(ctx, input, agent_id, locale).await;
    let Some(title) = display_title(localized.as_deref(), &agent.title) else {
        warn!(agent_id, "the agent has no title to introduce itself by");
        return None;
    };
    Some(PendingIntroduction {
        thread_id: thread_id.to_owned(),
        agent_id: agent_id.to_owned(),
        title,
    })
}

/// Record the introduction once `reply` — the text the thread received — has
/// named the agent in its opening. A reply that did not leaves the agent
/// unintroduced, so the next one is asked again.
pub(crate) async fn record_if_named(
    ctx: &ChatPipelineContext,
    introduction: &PendingIntroduction,
    reply: &str,
    tenant_id: TenantId,
) {
    if !reply_names_agent(reply, &introduction.title) {
        info!(
            agent_id = %introduction.agent_id,
            "the reply did not open by naming the agent; its next reply is asked again"
        );
        return;
    }
    if let Err(e) = ctx
        .repos
        .chat
        .record_agent_introduction(&introduction.thread_id, &introduction.agent_id, tenant_id)
        .await
    {
        warn!(
            error = %e,
            agent_id = %introduction.agent_id,
            "introduction not recorded; the agent's next reply introduces it again"
        );
    }
}

/// The agent's title as the store shows it in `locale`, or `None` when the
/// agent cannot be read, in which case the runtime context's canonical title
/// stands in.
///
/// Read under the tenant the agent resolved in for this turn: the athlete's
/// own for an `@handle`, the conversation's for the bound agent — the same
/// split `resolve_turn_agent_ctx` makes.
async fn localized_title(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    agent_id: &str,
    locale: &str,
) -> Option<String> {
    let user = parse_uuid(&input.user_id).ok()?;
    let tenant = if input.mentioned_agent.is_some() {
        input.tool_tenant_id
    } else {
        input.conversation_tenant_id
    };
    let agent = match ctx.repos.agents.get_by_id(agent_id, user, tenant).await {
        Ok(agent) => agent?,
        Err(e) => {
            warn!(error = %e, agent_id, "agent unreadable for its introduction title");
            return None;
        }
    };
    let mut agents = [agent];
    if let Err(e) = ctx.repos.agents.translate_agents(&mut agents, locale).await {
        warn!(error = %e, agent_id, locale, "agent title translation unreadable");
        return None;
    }
    let [agent] = agents;
    Some(agent.title)
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn fold(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}
