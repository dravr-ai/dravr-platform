// ABOUTME: Per-turn @handle routing — an installed agent named in the message answers that turn only
// ABOUTME: Scans the text for @handle tokens, resolves the first installed one and strips it for the model
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `@handle` mentions.
//!
//! An athlete hands one turn to a different agent by naming it:
//! `@recovery-agent comment gérer ma récup cette semaine ?`. The shape is
//! Claude Code's `ultracode` keyword — a token in the message that changes how
//! *this* turn is served and nothing after it. The conversation keeps its own
//! agent: a mention never writes `chat_conversations.agent_id`, so the next
//! plain message is answered by the agent the conversation is bound to.
//!
//! A mention resolves only against the athlete's **installed** agents
//! ([`AgentsRepository::find_installed_by_handle`]): a catalogue agent the
//! athlete never installed does not route, and neither does a token that is
//! not a handle at all. In both cases the turn proceeds with its text
//! untouched.
//!
//! Context stays per discussion. History, compaction blocks and the persisted
//! rows are all keyed by conversation, never by agent, so the same handle
//! carries independent context in each conversation — the way Claude behaves
//! in Slack or GitHub.
//!
//! The token grammar is [`AgentHandle::parse`]: lowercase letters, digits,
//! `-` and `_`, opened by a `@` that starts the text or follows whitespace or
//! an opening bracket or quote. `jf@dravr.ai` is an address, not a mention.

use pierre_core::models::agents::AgentHandle;
use pierre_core::models::{AgentRuntimeContext, TenantId};
use pierre_database::database::MessageRecord;
use pierre_database::repositories::AgentsRepository;
use tracing::{info, warn};
use uuid::Uuid;

use super::super::turn::TurnInput;

/// An agent the athlete addressed by `@handle` for this turn only.
#[derive(Debug, Clone)]
pub struct MentionedAgent {
    /// `agents.id` of the row that answered to the handle — the athlete's
    /// own installed copy when they have one, otherwise the origin they were
    /// assigned.
    pub agent_id: String,
    /// The handle as it resolved, without the `@`.
    pub handle: AgentHandle,
    /// The agent's runtime context, resolved in the athlete's own tenant —
    /// the one their install lives in, which on a shared messaging room is
    /// not the tenant the conversation is filed under.
    pub runtime: AgentRuntimeContext,
    /// The message as the model reads it: the resolved `@handle` token
    /// removed, everything else verbatim. The persisted row keeps the
    /// athlete's raw text.
    pub prompt_text: String,
}

/// One `@token` occurrence in the text, as byte offsets into it.
struct MentionSpan<'a> {
    /// Offset of the `@`.
    start: usize,
    /// Offset one past the last token character.
    end: usize,
    /// The token after the `@`.
    token: &'a str,
}

/// Whether a `@` preceded by `preceding` opens a mention rather than sitting
/// inside a word.
fn opens_mention(preceding: Option<char>) -> bool {
    preceding.is_none_or(|c| {
        c.is_whitespace() || matches!(c, '(' | '[' | '{' | '"' | '\'' | '«' | '“' | '‘')
    })
}

/// The characters a mention token is made of. Wider than the handle alphabet
/// on purpose: `@Recovery-Agent` is consumed as one token and then refused by
/// [`AgentHandle::parse`], rather than resolving `@` + nothing.
fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

/// Every `@token` in `text`, in reading order.
fn mention_spans(text: &str) -> Vec<MentionSpan<'_>> {
    let mut spans = Vec::new();
    let mut preceding: Option<char> = None;
    let mut chars = text.char_indices().peekable();
    while let Some((idx, c)) = chars.next() {
        if c == '@' && opens_mention(preceding) {
            let token_start = idx + c.len_utf8();
            let mut end = token_start;
            let mut last = c;
            while let Some(&(next_idx, next)) = chars.peek() {
                if !is_token_char(next) {
                    break;
                }
                end = next_idx + next.len_utf8();
                last = next;
                chars.next();
            }
            if end > token_start {
                spans.push(MentionSpan {
                    start: idx,
                    end,
                    token: &text[token_start..end],
                });
                preceding = Some(last);
                continue;
            }
        }
        preceding = Some(c);
    }
    spans
}

/// The handles `text` mentions, in reading order and without repeats.
///
/// A token that is not a handle under [`AgentHandle::parse`] — uppercase, too
/// long, opened by a separator — is not a candidate at all.
#[must_use]
pub fn mention_candidates(text: &str) -> Vec<AgentHandle> {
    let mut handles: Vec<AgentHandle> = Vec::new();
    for span in mention_spans(text) {
        if let Ok(handle) = AgentHandle::parse(span.token) {
            if !handles.contains(&handle) {
                handles.push(handle);
            }
        }
    }
    handles
}

/// `text` with every `@handle` token for `handle` removed.
///
/// The gap each token leaves is closed: the whitespace after it goes, or,
/// when the token runs into punctuation (`@agent, how…`), the whitespace
/// before it. A message that is nothing but the mention is returned as typed
/// — an empty user turn would be dropped from the prompt, and the athlete
/// summoning an agent by name alone still expects an opening.
#[must_use]
pub fn strip_mention(text: &str, handle: &AgentHandle) -> String {
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for span in mention_spans(text) {
        if span.token != handle.as_str() {
            continue;
        }
        out.push_str(&text[cursor..span.start]);
        cursor = span.end;
        match text[cursor..].chars().next() {
            Some(after) if after.is_whitespace() => cursor += after.len_utf8(),
            _ => {
                if out.ends_with(char::is_whitespace) {
                    out.pop();
                }
            }
        }
    }
    out.push_str(&text[cursor..]);
    let stripped = out.trim();
    if stripped.is_empty() {
        text.to_owned()
    } else {
        stripped.to_owned()
    }
}

/// Give the model this turn's message with the resolved token removed.
///
/// A resolved `@handle` is a routing token, not something the athlete said to
/// the agent. Only the in-memory row for the turn's own user message changes:
/// the persisted row keeps the raw text, so the transcript and every later
/// turn's history show what was typed. A turn with no mention is untouched.
pub fn apply_prompt_text(history: &mut [MessageRecord], user_message_id: &str, input: &TurnInput) {
    let Some(mention) = input.mentioned_agent.as_deref() else {
        return;
    };
    if let Some(row) = history.iter_mut().find(|m| m.id == user_message_id) {
        row.content.clone_from(&mention.prompt_text);
    }
}

/// Resolve the agent `text` hands this turn to, if it names one the athlete
/// has installed.
///
/// Candidates are tried in reading order and the first that resolves wins,
/// so one turn is answered by exactly one agent. `tenant_id` is the athlete's
/// own tenant: installs live there, and so does the runtime context the turn
/// is assembled from. A lookup that fails is logged and skipped rather than
/// failing the turn — an unresolved mention is a plain coaching turn, which
/// is also what a typo gets.
pub async fn resolve_agent_mention(
    agents: &dyn AgentsRepository,
    text: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Option<MentionedAgent> {
    for handle in mention_candidates(text) {
        let Some(agent_id) = installed_agent_id(agents, &handle, user_id, tenant_id).await else {
            continue;
        };
        let Some(runtime) = runtime_context(agents, &agent_id, &handle, tenant_id).await else {
            continue;
        };
        info!(agent_id = %agent_id, handle = %handle, "turn routed to the mentioned coach");
        return Some(MentionedAgent {
            prompt_text: strip_mention(text, &handle),
            agent_id,
            handle,
            runtime,
        });
    }
    None
}

/// The id of the agent `handle` names on the athlete's agent list, if any.
async fn installed_agent_id(
    agents: &dyn AgentsRepository,
    handle: &AgentHandle,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Option<String> {
    match agents
        .find_installed_by_handle(handle, user_id, tenant_id)
        .await
    {
        Ok(agent) => agent.map(|c| c.id.to_string()),
        Err(e) => {
            warn!(handle = %handle, error = %e, "coach mention lookup failed; the mention does not route");
            None
        }
    }
}

/// The runtime context of an installed agent, in the athlete's tenant.
async fn runtime_context(
    agents: &dyn AgentsRepository,
    agent_id: &str,
    handle: &AgentHandle,
    tenant_id: TenantId,
) -> Option<AgentRuntimeContext> {
    match agents.get_agent_runtime_context(agent_id, tenant_id).await {
        Ok(Some(runtime)) => Some(runtime),
        Ok(None) => {
            warn!(agent_id, handle = %handle, "mentioned coach has no runtime context; the mention does not route");
            None
        }
        Err(e) => {
            warn!(agent_id, handle = %handle, error = %e, "mentioned coach runtime context failed; the mention does not route");
            None
        }
    }
}
