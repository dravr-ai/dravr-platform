// ABOUTME: First-reply introduction — an agent names itself and its role on its first turn
// ABOUTME: Decided by the platform from the transcript and the agent's localized title, never left to the model
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The first-reply introduction (carnet#501).
//!
//! An athlete opening a conversation with an agent is owed one line saying who
//! is answering and what it is there for. On 2026-09-21 23:46Z a web chat with
//! the Half Marathon Agent opened on « Montre-moi mes sorties de mars 2026 avec
//! le dénivelé. » and the reply went straight into « Mars a été un mois de
//! fond… » — no name, no role.
//!
//! Nothing in the prompt stack had ever asked for one. No platform stage and no
//! dravr-contremaitre prompt carried an introduction rule at any point in
//! either history, so whether a first reply opened with one was left to the
//! model. A behaviour the model may or may not choose is not one the product
//! has.
//!
//! So the platform decides, because it holds the two facts the rule turns on
//! and the model holds neither reliably:
//!
//! - **Is this the first reply?** The transcript says so: no `assistant` row
//!   yet. A model asked to work that out would infer it from a history it may
//!   only see part of.
//! - **What is the agent called, in this athlete's language?** A contremaitre
//!   persona file carries its localized `title:` — « Agent Semi-Marathon » for
//!   a French athlete — which the canonical `agents.title` column does not.
//!
//! The directive rides in the Stage 7g.3 slot, after the ordinary turn's task,
//! because it is a task for this one turn. It names no language: Stage 7g.3b is
//! the only language rule in the stack, and a title already in the athlete's
//! locale is what lets the line come out in it.
//!
//! Two scopes are left out on purpose:
//! - a turn with no agent bound, which is Dravr answering in its own voice
//!   under the identity anchor, as it always has;
//! - a shared room, where each member has their own conversation row, so the
//!   first reply in a member's row is not the first time the room heard the
//!   agent — it would re-introduce itself every time a new member spoke.

use pierre_agent_parser::parse_frontmatter;
use pierre_core::models::AgentRuntimeContext;
use pierre_database::database::MessageRecord;
use tracing::warn;

/// Role a reply row is written under: the model's answer (`lib.rs`), a
/// platform-authored one (`deterministic_reply`) and a slash command's answer
/// (`command_persistence`) alike.
const ASSISTANT_ROLE: &str = "assistant";

/// `agents.source` of a catalogue agent, whose persona is a contremaitre
/// markdown file with frontmatter — the value the seeder stamps and
/// [`super::prompt_assembly::resolve_agent_base_prompt`] branches on.
const CONTREMAITRE_SOURCE: &str = "contremaitre";

/// Whether the reply this turn produces is the conversation's first.
///
/// `history` is the transcript as prompt assembly receives it: the current
/// message included, slash-command rows already removed by
/// [`super::persistence::get_conversation_history`]. A command's answer is the
/// platform talking, so an `/agent add` before the first question leaves the
/// agent still unintroduced. Any `assistant` row left means the athlete has
/// already been answered in this thread.
#[must_use]
pub fn is_first_reply(history: &[MessageRecord]) -> bool {
    !history.iter().any(|row| row.role == ASSISTANT_ROLE)
}

/// The agent's title in the turn's locale, or `None` when it has none.
///
/// `persona_prompt` is the persona text this turn loaded. For a contremaitre
/// agent that is the markdown file for the turn's locale, and its `title:` is
/// the localized name the store shows; the `agents.title` column holds the
/// canonical English one. Any other agent has only the column. The column is
/// also the answer when the file cannot be parsed, so a persona edit can never
/// leave the agent nameless.
///
/// Whitespace is collapsed to single spaces, because the title is spliced into
/// a one-sentence instruction and a line break inside it would split that.
#[must_use]
pub fn agent_display_title(agent: &AgentRuntimeContext, persona_prompt: &str) -> Option<String> {
    let localized = if agent.source == CONTREMAITRE_SOURCE {
        parse_frontmatter(persona_prompt)
            .ok()
            .map(|frontmatter| collapse_whitespace(&frontmatter.title))
            .filter(|title| !title.is_empty())
    } else {
        None
    };
    let title = localized.unwrap_or_else(|| collapse_whitespace(&agent.title));
    (!title.is_empty()).then_some(title)
}

/// The instruction that opens an agent's first reply with its introduction.
///
/// Held to the constraints of the [`super::prompt_assembly::TURN_DIRECTIVE`] it
/// is appended to: it states a task for this reply and asserts no identity,
/// because text shaped like an identity override is what provoked the refusals
/// that block was measured against, and who the model is already belongs to
/// the identity anchor at the tail. It sets no format for the reply either;
/// "one short sentence" bounds the introduction, not the answer after it.
#[must_use]
pub fn introduction_directive(title: &str) -> String {
    format!(
        "\nThis is your first reply in this conversation, and the athlete has not met you yet. \
         Open it with one short sentence introducing yourself by your title, {title}, and what \
         you help them with; then answer their question in the same reply."
    )
}

/// The introduction this turn carries, or `None` when it carries none.
///
/// Only a reply that is the first in a one-to-one conversation, answered by a
/// bound agent, is introduced. `is_direct_message` is the turn's audience
/// flag: `false` for a messaging group or an in-app thread bound to a coaching
/// group.
#[must_use]
pub fn first_reply_introduction(
    history: &[MessageRecord],
    agent: Option<&AgentRuntimeContext>,
    persona_prompt: &str,
    is_direct_message: bool,
) -> Option<String> {
    if !is_direct_message || !is_first_reply(history) {
        return None;
    }
    let agent = agent?;
    let Some(title) = agent_display_title(agent, persona_prompt) else {
        warn!(
            slug = %agent.slug,
            "bound agent has no title to introduce itself by; its first reply carries no introduction"
        );
        return None;
    };
    Some(introduction_directive(&title))
}

fn collapse_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
