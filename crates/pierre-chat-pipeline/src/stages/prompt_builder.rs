// ABOUTME: System prompt assembly stage — LLM message construction from history
// ABOUTME: Provides build_llm_messages — turns persisted chat history + system prompt into wire-format messages
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! System prompt assembly helpers.
//!
//! The full prompt-building pipeline for a turn is driven by
//! [`super::super::run`], which composes coach/default text,
//! [`super::super::stages::refresh`] freshness hints, Tier 2 memory recall
//! ([`super::memory`]), Tier 4 followups ([`super::followups`]), and the
//! channel-profile response-constraints suffix. This module owns the
//! final mechanical step: turning the assembled system prompt and the
//! prior message history into a flat `Vec<ChatMessage>` for the LLM.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Arc;

use pierre_database::database::MessageRecord;
use pierre_memory::CompactionBlock;
use uuid::Uuid;

#[cfg(feature = "tools-groups")]
use crate::ChatPipelineContext;
use pierre_core::models::ConnectionType;
use pierre_core::models::WITHHELD_REPLY_FINISH_REASON;
use pierre_core::narration::scrub_replayed_narration;
use pierre_llm::ChatMessage;
use pierre_runtime_context::DataContext;
use pierre_services::conversation_compaction::REPLAYED_SUMMARY_PREFIX;
use pierre_tool_runtime::registry::ToolRegistry;
use pierre_tool_runtime::tool_execution::{
    is_withheld_during_guided_flow, strip_simulation_artifacts,
};

#[cfg(feature = "tools-groups")]
use pierre_core::errors::AppResult;
#[cfg(feature = "tools-groups")]
use pierre_core::models::TenantId;
#[cfg(feature = "tools-groups")]
use pierre_tool_runtime::group_fitness::{fetch_member_snapshots, MemberSnapshot};

/// Resolve group context strictly from the conversation record's
/// `group_id`.
///
/// Group context (member snapshots, group-scoped prompt injection) is
/// opt-in: a conversation must be explicitly created with
/// `group_id = Some(...)` to receive it. 1:1 personal conversations —
/// where `group_id` is `None` — never auto-attach to a group the user
/// happens to belong to. Doing so would leak another group member's
/// fitness data into a private chat and confuse the LLM about whose
/// activities the user is asking about.
///
/// Returns `(Some(group_id), snapshots)` when the conversation is
/// group-scoped and members are found, or `(None, empty_vec)` otherwise.
///
/// # Errors
///
/// Database errors from the group-member lookup are swallowed (the
/// function degrades to an empty member list); this function itself
/// currently only propagates `AppError` for signature symmetry with
/// the rest of the pipeline stages — no variants are produced today.
#[cfg(feature = "tools-groups")]
pub async fn resolve_group_context(
    ctx: &ChatPipelineContext,
    conversation_group_id: Option<&str>,
    tool_tenant_id: TenantId,
) -> AppResult<(Option<String>, Vec<MemberSnapshot>)> {
    let Some(gid) = conversation_group_id else {
        return Ok((None, Vec::new()));
    };

    let member_ids: Vec<uuid::Uuid> = ctx
        .repos
        .groups
        .list_members(gid)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|m| m.user_id)
        .collect();

    let snapshots = if member_ids.is_empty() {
        Vec::new()
    } else {
        fetch_member_snapshots(&ctx.tool_runtime, &member_ids, tool_tenant_id).await
    };

    Ok((Some(gid.to_owned()), snapshots))
}

/// Build LLM messages from conversation history and optional system prompt.
///
/// The system prompt (when provided) leads the message list; history is
/// appended in order, dropping messages with unknown roles defensively.
///
/// `tool_call` and `tool_result` rows are emitted as assistant/user text
/// messages respectively — they replay the exact `Vec<ChatMessage>` shape
/// that `run_api_tool_loop` and `run_cli_tool_loop` push into `llm_messages`
/// mid-loop, so a follow-up turn sees the same grounded evidence the model
/// just consumed last turn.
///
/// Returns the messages alongside a parallel `source_ids` vector that maps
/// each emitted [`ChatMessage`] back to its origin: `None` for the system
/// prompt, `Some(MessageRecord.id)` for each surviving history row. The two
/// vectors stay index-aligned because a dropped history row (empty after the
/// strip, or an unknown role) skips both. Compaction reads this mapping to
/// anchor a block's first/last message id to real persisted rows rather than
/// guessing positions — `strip_simulation_artifacts` drops some rows, so the
/// emitted list is shorter than `history`, and a positional mapping would be
/// off by the number of dropped rows.
#[must_use]
pub fn build_llm_messages(
    system_prompt: Option<&str>,
    history: &[MessageRecord],
) -> (Vec<ChatMessage>, Vec<Option<String>>) {
    build_llm_messages_with_blocks(system_prompt, history, &[])
}

/// Build LLM messages, splicing persisted compaction summaries back over the
/// raw history rows they replaced.
///
/// Identical to [`build_llm_messages`] except that each accepted
/// [`CompactionBlock`] collapses its covered `[first_message_id,
/// last_message_id]` history range into a single `User` message carrying the
/// block's `summary` text under [`REPLAYED_SUMMARY_PREFIX`] — exactly the
/// read-side that lets a long thread keep its summarized context on later
/// turns.
///
/// `User`, not `System`: the live provider keeps only the FIRST system message
/// and filters every other one out of history, so a mid-list system summary
/// never reached the model at all. The prefix is what tells it the text is
/// recovered history rather than something the athlete just said. The UI-only
/// [`COMPACTION_MARKER`] stays off the wire.
///
/// [`COMPACTION_MARKER`]: pierre_services::conversation_compaction::COMPACTION_MARKER
///
/// A block is *accepted* only when both of its boundary ids resolve to indices
/// in `history`, the range is well-formed (`first_index <= last_index`), and it
/// does not overlap an already-accepted block. Because `blocks` arrives
/// `created_at` ASC, the earliest-created block wins any overlap and the later
/// one is skipped — there is never a double summary for a row. A block whose
/// boundary id is absent from `history` (it straddles or sits outside the
/// loaded window, or belongs to another conversation) is skipped entirely and
/// its rows render raw.
///
/// The injected summary occupies one emitted slot with `source_id` `None`
/// (like the system prompt): it is not a persisted history row, so Tier 1
/// compaction's `source_ids`-aware guard treats it as off-limits and never
/// re-summarizes an already-injected summary.
#[must_use]
pub fn build_llm_messages_with_blocks(
    system_prompt: Option<&str>,
    history: &[MessageRecord],
    blocks: &[CompactionBlock],
) -> (Vec<ChatMessage>, Vec<Option<String>>) {
    let mut messages = Vec::with_capacity(history.len() + 1);
    let mut source_ids: Vec<Option<String>> = Vec::with_capacity(history.len() + 1);

    if let Some(prompt) = system_prompt {
        messages.push(ChatMessage::system(prompt));
        source_ids.push(None);
    }

    // Resolve each block's boundary ids to indices, then accept it only if the
    // range is well-formed and disjoint from every block already accepted. We
    // iterate `blocks` in their given order (the repository returns them
    // `created_at` ASC), so an overlap resolves in favour of the earlier block.
    let id_to_index: HashMap<&str, usize> = history
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id.as_str(), i))
        .collect();
    let mut accepted: Vec<AcceptedBlock<'_>> = Vec::new();
    for block in blocks {
        let (Some(&first_index), Some(&last_index)) = (
            id_to_index.get(block.first_message_id.as_str()),
            id_to_index.get(block.last_message_id.as_str()),
        ) else {
            continue;
        };
        if first_index > last_index {
            continue;
        }
        let overlaps = accepted
            .iter()
            .any(|a| first_index <= a.last_index && a.first_index <= last_index);
        if overlaps {
            continue;
        }
        accepted.push(AcceptedBlock {
            first_index,
            last_index,
            summary: &block.summary,
        });
    }
    // Index the accepted blocks by their start row so the walk can detect a
    // block boundary in O(1) and fast-forward over the rows it covers.
    let block_start: HashMap<usize, &AcceptedBlock<'_>> =
        accepted.iter().map(|a| (a.first_index, a)).collect();

    let mut i = 0usize;
    while i < history.len() {
        if let Some(block) = block_start.get(&i) {
            // Splice the summary in place of the raw rows `[first_index,
            // last_index]` (inclusive), then skip every covered row. It rides
            // in a `User` message under REPLAYED_SUMMARY_PREFIX: the live
            // provider keeps only the first system message and drops the rest,
            // so a mid-list `System` summary never reached the model, and the
            // prefix tells it the text is recovered history rather than a new
            // athlete turn. The UI-only COMPACTION_MARKER stays off the wire.
            // It is
            // LLM-written from assistant turns, so a leak that predates the
            // narration scrub can be baked into it (observed 2026-07-10:
            // compaction ran mid-incident; 2026-07-23: a summary carrying
            // "can't fetch your data" narrative taught the model to stop
            // calling get_activities) — scrub it with the replay variant so
            // neither leak class re-enters every subsequent prompt.
            let summary = scrub_replayed_narration(block.summary).cleaned;
            if !summary.is_empty() {
                messages.push(ChatMessage::user(format!(
                    "{REPLAYED_SUMMARY_PREFIX}{summary}"
                )));
                source_ids.push(None);
            }
            i = block.last_index + 1;
            continue;
        }
        push_history_row(&history[i], &mut messages, &mut source_ids);
        i += 1;
    }

    (messages, source_ids)
}

/// An accepted compaction block, resolved to history indices.
struct AcceptedBlock<'a> {
    first_index: usize,
    last_index: usize,
    summary: &'a str,
}

/// Map one persisted history row to its wire-format [`ChatMessage`] and append
/// it (with its `source_id`) to the in-flight vectors, or drop it.
///
/// Shared by [`build_llm_messages`] and [`build_llm_messages_with_blocks`] so
/// the per-row mapping lives in exactly one place: the no-blocks path is then
/// byte-identical to the legacy single-function output.
fn push_history_row(
    msg: &MessageRecord,
    messages: &mut Vec<ChatMessage>,
    source_ids: &mut Vec<Option<String>>,
) {
    // Strip tool-result scaffolding from replayed history before it re-enters
    // the prompt. Persisted `tool_result` turns hold raw `<tool_result>` XML,
    // and a prior parroted assistant echo holds the same blocks; replaying
    // either verbatim teaches the model to imitate the format and emit a
    // tool-result echo instead of an answer (observed: a long thread degrades
    // to empty/parroted replies). `strip_simulation_artifacts` leaves a real
    // synthesized answer untouched and reduces pure scaffolding to empty, so
    // an empty result is dropped rather than re-seeding the parrot. Mirrors
    // the per-turn strip in `run_cli_tool_loop` / `finalize_headless_turn`.
    // A withheld turn's persisted row is the platform's apology, not the coach's
    // words. It stays in the database and the UI (the athlete saw it) but must
    // never re-enter a prompt: replaying "my reply didn't go through" as an
    // assistant turn is the self-referential-failure narration the replay scrub
    // exists to remove, and it survived that scrub because the string is
    // authored in dravr-contremaitre in five locales. The stamp is the
    // pattern-free way to recognize it. Only assistant rows ever carry it —
    // user rows and tool_call/tool_result rows persist `None`.
    if msg.finish_reason.as_deref() == Some(WITHHELD_REPLY_FINISH_REASON) {
        return;
    }

    let stripped = strip_simulation_artifacts(&msg.content);
    if stripped.is_empty() {
        return;
    }
    // Assistant rows additionally get the replay-narration scrub: rows
    // persisted before the response-boundary scrub existed (or by an older
    // binary) can hold hidden-block meta-commentary, and replaying it
    // verbatim teaches the model to keep narrating — the « Je continue
    // d'ignorer le bloc caché » loop observed live on 2026-07-10. A row
    // that is pure narration is dropped like pure scaffolding.
    let replayed = match msg.role.as_str() {
        "assistant" | "tool_call" => Cow::Owned(scrub_replayed_narration(&stripped).cleaned),
        _ => Cow::Borrowed(stripped.as_str()),
    };
    if replayed.is_empty() {
        return;
    }
    let chat_msg = match msg.role.as_str() {
        "user" | "tool_result" => ChatMessage::user(replayed.as_ref()),
        "assistant" | "tool_call" => ChatMessage::assistant(replayed.as_ref()),
        "system" => ChatMessage::system(replayed.as_ref()),
        _ => return,
    };
    messages.push(chat_msg);
    source_ids.push(Some(msg.id.clone()));
}

/// Build the "Connected Fitness Data Providers" system-prompt section.
///
/// Appended so the LLM does not ask users to connect providers that are
/// already connected.
///
/// Uses `provider_connections` as the single source of truth (cross-tenant
/// view) and filters out providers that are not registered in the current
/// runtime (e.g. synthetic providers excluded from production builds).
/// Returns an empty string when the user has no registered connections —
/// callers append this unconditionally.
pub async fn build_provider_context(data: &DataContext, user_id: Uuid) -> String {
    // Get all provider connections (cross-tenant view, single source of truth)
    let Ok(connections) = data
        .repos()
        .provider_connections
        .get_for_user(user_id, None)
        .await
    else {
        return String::new();
    };

    // Filter out providers that aren't registered in the current runtime
    // (e.g., synthetic providers excluded from production builds)
    let connections: Vec<_> = connections
        .into_iter()
        .filter(|c| data.provider_registry().is_supported(&c.provider))
        .collect();

    if connections.is_empty() {
        return String::new();
    }

    let mut context = String::from("\n\n## Connected Fitness Data Providers\n\n");
    context.push_str("The user has the following data sources available:\n");
    for conn in &connections {
        let label = if conn.connection_type == ConnectionType::Synthetic {
            Cow::Owned(format!("{} (test data)", conn.provider))
        } else {
            Cow::Borrowed(conn.provider.as_str())
        };
        // Write trait used to avoid format_push_string lint
        let _ = writeln!(context, "- ✓ {label}");
    }
    context.push_str("\nUse the connected providers to fetch activity data. ");
    context.push_str("Do NOT ask the user to connect providers that are already connected above.");

    context
}

/// Build the "Available Tools" section from the runtime tool registry.
///
/// Replaces the previously-static tool list that lived in
/// `pierre_system.md`. Generating it from the registry at assembly time
/// prevents the two from drifting as tools are added, renamed, or
/// removed — a drift that historically led the LLM to invent
/// capabilities (e.g. "look up Uber Eats menus") when the static list
/// stopped reflecting reality. Each user-visible tool gets one line:
/// `` - `name`: description ``. Admin-only tools are excluded.
///
/// When `guided_flow_active`, the tools withheld for the duration of a guided
/// flow ([`GUIDED_FLOW_WITHHELD_TOOLS`]) are dropped from this list too. The
/// prose list and the native declarations must move together: advertising a tool
/// in prose that the function-calling surface does not expose is the exact
/// advertised-vs-callable drift this generated section exists to prevent.
#[must_use]
pub fn build_tools_section(tool_registry: &Arc<ToolRegistry>, guided_flow_active: bool) -> String {
    let schemas = tool_registry.user_visible_schemas();
    let schemas = schemas
        .into_iter()
        .filter(|schema| !guided_flow_active || !is_withheld_during_guided_flow(&schema.name));

    let mut out = String::with_capacity(2_048);
    out.push_str("## Available Tools\n\n");
    out.push_str("You have exactly the tools listed below. You do **not** have any tool that is not in this list: ");
    out.push_str(
        "you cannot browse the web, scrape menus, look up prices, use third-party services, ",
    );
    out.push_str(
        "or run arbitrary code. If a request requires a capability not covered here, say so ",
    );
    out.push_str("honestly rather than inventing a plan. Call tools with the parameters described in their schemas.\n\n");

    for schema in schemas {
        // One line per tool: `- `name`: description (first line only)`.
        // Multi-line descriptions exist for a handful of tools; the LLM
        // sees the full schema via native function-calling, so trimming
        // the prompt-side description to the lead sentence keeps the
        // system prompt compact.
        let description_lead = schema.description.lines().next().unwrap_or("").trim();
        let _ = writeln!(out, "- `{}`: {}", schema.name, description_lead);
    }

    out
}
