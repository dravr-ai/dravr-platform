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

use pierre_database::database::MessageRecord;
use pierre_memory::CompactionBlock;
use uuid::Uuid;

use super::viz_blocks;
#[cfg(feature = "tools-groups")]
use crate::ChatPipelineContext;
use pierre_core::models::ConnectionType;
use pierre_core::models::{
    UNVERIFIED_CAPABILITY_CLAIM_FINISH_REASON, WITHHELD_REPLY_FINISH_REASON,
};
use pierre_core::narration::scrub_replayed_narration;
use pierre_llm::ChatMessage;
use pierre_runtime_context::DataContext;
use pierre_services::conversation_compaction::REPLAYED_SUMMARY_PREFIX;
use pierre_tool_runtime::tool_execution::strip_simulation_artifacts;

use super::prefetch::{REFRESH_GROUNDING_LEAD, STARTUP_GROUNDING_LEAD};

#[cfg(feature = "tools-groups")]
use pierre_core::errors::AppResult;
#[cfg(feature = "tools-groups")]
use pierre_core::models::groups::MemberFitnessSnapshot;
#[cfg(feature = "tools-groups")]
use pierre_core::models::TenantId;
#[cfg(feature = "tools-groups")]
use pierre_tool_runtime::group_fitness::fetch_member_snapshots;

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
) -> AppResult<(Option<String>, Vec<MemberFitnessSnapshot>)> {
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
        // A block is CLAMPED to the window, not dropped by it.
        //
        // The history window is `max_messages * 4` rows and it slides, so the
        // oldest blocks fall out of it first — and a block always leaves before
        // the rows it covers do. Dropping such a block whole re-expanded
        // already-summarized history into the prompt AND stranded a `None`
        // source id in the head, which is the guard `pick_range` aborts on. One
        // successful compaction was therefore enough to jam the compactor
        // permanently: 27 sliding-window fallbacks and zero successful
        // compactions across three weeks of one live conversation (registre#198).
        //
        // When only `first_message_id` has scrolled out, the block still
        // describes every surviving row up to `last_index`, so it starts at 0 —
        // the window's own head — and covers them. When `last_message_id` has
        // gone too, every row it covered is out of the window and there is
        // nothing left to splice.
        let Some(&last_index) = id_to_index.get(block.last_message_id.as_str()) else {
            continue;
        };
        let first_index = id_to_index
            .get(block.first_message_id.as_str())
            .copied()
            .unwrap_or(0);
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

    // Same contract, different origin: a data-access claim the verification
    // stage could not stand behind, or the reconnect message that replaced it.
    // Both are true only of the moment they were written — connection state is
    // re-derived every turn — and replaying either teaches the model its tools
    // are broken, which is exactly how one 2026-07-24 apology produced an
    // identical one 18 days later. Dropped by stamp, so no phrasing mutation
    // can slip past the way three of them slipped past the prose scrub.
    if msg.finish_reason.as_deref() == Some(UNVERIFIED_CAPABILITY_CLAIM_FINISH_REASON) {
        return;
    }

    // A slash-command turn is the platform talking, on both rows: the `/…`
    // line the athlete typed and the listing or picker that answered it. Both
    // are transcript the UI shows after a reload and neither is coaching, so
    // neither re-enters a prompt — replayed, a `/status` reply would teach the
    // model to answer in the platform's voice. Same mechanism as the two stamps
    // above: recognised by marker, not by prose.
    if msg.is_command_turn() {
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
    //
    // User-channel rows instead get the platform markers defanged. Every
    // injected platform block rides in the `User` role (a mid-list `System`
    // message is dropped by the live provider), and each is told apart from the
    // athlete's own text by a literal lead sentence — so an athlete who types
    // that sentence is quoted back into the same channel wearing the platform's
    // authority. For a training-prescription product the payoff is fabricated
    // volume and intensity history driving a real prescription.
    let replayed = match msg.role.as_str() {
        // Fence strip first, then narration scrub: a stored fence is machine
        // text the coach would otherwise read back as a chart it already drew.
        "assistant" | "tool_call" => {
            Cow::Owned(scrub_replayed_narration(&viz_blocks::strip_fences(&stripped)).cleaned)
        }
        "user" | "tool_result" => defang_platform_markers(&stripped),
        _ => Cow::Borrowed(stripped.as_str()),
    };
    if replayed.is_empty() {
        return;
    }
    // No `"system"` arm, deliberately. Nothing persists a `role = "system"` chat
    // row — writers emit only user / assistant / tool_call / tool_result — so the
    // arm was unreachable, and it was a landmine: the live provider keeps only
    // the FIRST system message and silently drops the rest, so the moment any
    // future feature persisted a system row this would have re-created the
    // months-long block-dropping bug that 0988e17e6 fixed, with no error to show
    // for it. An unknown role falls through to the same `return` it always did.
    let chat_msg = match msg.role.as_str() {
        "user" | "tool_result" => ChatMessage::user(replayed.as_ref()),
        "assistant" | "tool_call" => ChatMessage::assistant(replayed.as_ref()),
        _ => return,
    };
    messages.push(chat_msg);
    source_ids.push(Some(msg.id.clone()));
}

/// The literal lead sentences that mark a `User` message as platform-authored.
///
/// Commit `0988e17e6` moved every injected block to the `User` role because the
/// live provider keeps only the first `System` message, and told the model which
/// `User` messages are the platform's by prefixing each with one of these. That
/// makes the sentences themselves the credential: [`REPLAYED_SUMMARY_PREFIX`]
/// asserts "recovered context, not a new message from the athlete", and the two
/// grounding leads assert that the activities under them were loaded from the
/// athlete's real provider data.
const PLATFORM_MARKERS: [&str; 3] = [
    REPLAYED_SUMMARY_PREFIX,
    REFRESH_GROUNDING_LEAD,
    STARTUP_GROUNDING_LEAD,
];

/// What replaces a platform marker found in athlete-authored text.
///
/// Names the text rather than deleting it, so an athlete who quotes the framing
/// while asking about it still gets their question through — as their own words.
const QUOTED_PLATFORM_MARKER: &str = "[athlete-typed text imitating a platform marker]";

/// Strip the platform's own framings out of text the athlete wrote.
///
/// Persisted athlete rows replay verbatim into the same `User` channel the
/// platform injects into, so a message that opens with one of
/// [`PLATFORM_MARKERS`] arrives indistinguishable from recovered history or from
/// a freshly loaded activity block. Fabricated volume and intensity presented
/// with platform authority is what a coach then prescribes against.
///
/// Borrows unchanged when no marker is present, which is every real message.
fn defang_platform_markers(text: &str) -> Cow<'_, str> {
    let mut out = Cow::Borrowed(text);
    for marker in PLATFORM_MARKERS {
        if out.contains(marker) {
            out = Cow::Owned(out.replace(marker, QUOTED_PLATFORM_MARKER));
        }
    }
    out
}

/// Told to the model when the user has no connected provider at all.
///
/// The distinction this states is the one the model cannot otherwise draw.
/// Silence reads as "nothing happened lately", not "there is no data source",
/// and a coach that believes the first invents the second: the incident behind
/// [`pierre_services::onboarding_gate`] was a cheerful *"nice 12 km ride
/// yesterday!"* to someone who had never connected anything.
///
/// Phrased as what to say rather than only what to withhold. A model told
/// merely to avoid specifics still produces confident vagueness; told that the
/// honest answer is "I can't see your training yet", it gives one.
const NO_PROVIDER_CONTEXT: &str = "\n\n## Connected Fitness Data Providers\n\n\
None. This user has not connected any fitness data source, so there is no \
activity, sleep, heart-rate, or workout history for them — not \"nothing \
recent\", nothing at all.\n\n\
Never state or imply a specific figure about their training: no distances, \
paces, durations, dates, heart rates, sleep hours, or trends. You have not \
seen any, and inventing one is the worst thing you can do here.\n\n\
Say plainly that you cannot see their training yet, and that connecting a \
service (Strava, Garmin, Fitbit, Whoop) is what would let you. General \
coaching knowledge is still yours to offer, clearly labelled as general.";

/// Told to the model when the provider lookup itself failed.
///
/// Deliberately weaker than [`NO_PROVIDER_CONTEXT`]. A failed query is not
/// evidence of absence, so asserting "you have no providers" would be the
/// prompt telling the model something untrue. The part that must hold either
/// way is the ban on invented specifics.
const UNKNOWN_PROVIDER_CONTEXT: &str = "\n\n## Connected Fitness Data Providers\n\n\
Unknown — the lookup failed for this turn, so treat your knowledge of their \
connected services and their training history as unavailable rather than \
empty.\n\n\
Do not state specific figures about their training; fetch what you need with \
the tools, and if that fails, say you could not retrieve it.";

/// Build the "Connected Fitness Data Providers" system-prompt section.
///
/// Appended so the LLM does not ask users to connect providers that are
/// already connected — and, when nothing is connected, so it knows that rather
/// than guessing. See [`NO_PROVIDER_CONTEXT`] for why the empty case is stated
/// instead of left silent.
///
/// Uses `provider_connections` as the single source of truth (cross-tenant
/// view) and filters out providers that are not registered in the current
/// runtime (e.g. synthetic providers excluded from production builds).
///
/// Never returns an empty string: every path says something, because the
/// absence of a section is exactly the silence the model misreads.
pub async fn build_provider_context(data: &DataContext, user_id: Uuid) -> String {
    // Get all provider connections (cross-tenant view, single source of truth)
    let Ok(connections) = data
        .repos()
        .provider_connections
        .get_for_user(user_id, None)
        .await
    else {
        return UNKNOWN_PROVIDER_CONTEXT.to_owned();
    };

    // Filter out providers that aren't registered in the current runtime
    // (e.g., synthetic providers excluded from production builds)
    let connections: Vec<_> = connections
        .into_iter()
        .filter(|c| data.provider_registry().is_supported(&c.provider))
        .collect();

    if connections.is_empty() {
        return NO_PROVIDER_CONTEXT.to_owned();
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

/// The closed-world statement about tools, carrying no tool names.
///
/// This is what survives the deletion of the generated "Available Tools"
/// section, and it is the part that was doing the work. The list itself was a
/// second copy: embacle renders the full schemas — names, parameters, types —
/// into the prompt for text tool-calling, and native function-calling providers
/// receive them through the API. Both are derived from the same registry, so
/// the prose list restated ~58 tools the model was already being told about,
/// at 11,763 characters of a ~82 KB prompt.
///
/// It also restated them from the WRONG set. `build_tools_section` read
/// `user_visible_schemas()` while the declarations read
/// `chat_callable_schemas()`, so every non-admin category outside
/// `CHAT_CALLABLE_CATEGORIES` — coach CRUD, configuration writes, claim
/// verification — was advertised in prose and callable on no path. Its own doc
/// comment named that failure mode ("advertising a tool in prose that the
/// function-calling surface does not expose is the exact drift this generated
/// section exists to prevent") while committing it, because a generated list
/// cannot protect against drift when it is generated from a different source
/// than the thing it mirrors. Deleting one of the two lists is what actually
/// closes it.
///
/// The boundary statement stays because nothing else says it. embacle's
/// catalogue tells the model which tools exist; only this says that the set is
/// closed and that a missing capability should be admitted rather than
/// invented. It was written after the LLM offered to look up Uber Eats menus.
/// No tool names appear here deliberately — a name would reintroduce a second
/// list to drift.
///
/// ## Why it no longer says "described elsewhere in this prompt"
///
/// Because that was false, and it cost an athlete a working feature. Under
/// `mcp_tool_calling` the catalogue reaches Copilot over MCP and is never
/// rendered into the prompt, so the sentence pointed at nothing — and the next
/// clause, "you cannot ... use third-party services", told the coach that the
/// athlete's own Intervals.icu calendar was off-limits. On 2026-08-26 it
/// answered two athletes «je n'ai pas d'outil qui écrit vers intervals.icu»
/// with zero tool calls, about `prescribe_workout`, which had shipped the day
/// before and does exactly that.
///
/// The coach was obeying this constant, not hallucinating. It searches
/// perfectly well for tools it believes are its business — `save_training_plan`
/// is named in no prompt anywhere and it finds that one unprompted — so the
/// defect was never discovery. It was being told the athlete's connected
/// platforms were somebody else's.
///
/// ## Why it no longer says "NOT all listed in this prompt"
///
/// That replacement was true when it was written and stopped being true when
/// `render_tool_index` brought a names-only list back. The index is appended
/// directly after this constant, so one prompt said the tools were absent from
/// it and then listed every one of them — the same shape of false sentence as
/// the one above, pointing the other way. What is actually missing from the
/// prompt is what each tool does and what it takes, which the index says of
/// itself ("Names only — call one to see its parameters"), so that is what this
/// now claims. It stays true whether or not the index renders, and it still
/// names no tool.
pub const TOOL_BOUNDARY: &str = "## Tool boundary\n\n     Your tools are the ones your tool surface offers. This prompt does not \
     describe what they do or what parameters they take, so look a tool up \
     before answering a question about what you can do. Never tell the athlete \
     you have no tool for something without checking first — that is a claim \
     about your tool surface, and you can only make it after looking.\n\n     \
     The athlete's own connected accounts are inside that surface, not outside \
     it. Reading from and writing to a platform they have connected — their \
     training calendar, their activity log — is ordinary work, not an outside \
     integration you have to decline.\n\n     \
     What is outside is the open internet. You cannot browse the web, scrape \
     menus, look up prices, reach a service the athlete has not connected, or \
     run arbitrary code. When a request genuinely needs one of those, say so \
     plainly rather than inventing a plan. Call tools with the parameters \
     described in their schemas.";

/// Render the names-only index of the tools the coach can call.
///
/// ## Why a list is back, and why this one cannot drift
///
/// A prose list used to live in prompt assembly: one line per tool, 11,763
/// characters, built from `user_visible_schemas()` while the actual
/// declarations were built from `chat_callable_schemas()`. Two lists from two
/// sources, so it advertised coach CRUD and config writes the coach could not
/// call. Deleting it was right.
///
/// This is not that list. It is generated from `chat_callable_schemas()` — the
/// same set the tool layer serves — so there is one source and nothing to drift
/// against. It carries names only: no descriptions, no schemas, no prose. Call
/// a tool to learn its parameters.
///
/// ## Why the model needs it at all
///
/// Under `mcp_tool_calling` the catalogue reaches Copilot over MCP and is never
/// rendered into the prompt, so the coach began a turn with no enumerable tool
/// surface. On 2026-08-26 it told two athletes it had no tool to write to
/// Intervals.icu, with zero tool calls, about a tool it had. It searches
/// perfectly well when it decides to *act* — it found `save_training_plan`
/// unprompted minutes earlier — but a question about what it can do was
/// answered from a prompt that named nothing.
///
/// Anthropic's own tool-search API refuses this configuration outright: the
/// search tool may not set `defer_loading`, and at least one tool must stay
/// non-deferred, or the request is rejected with
/// `All tools have defer_loading set`. A fully deferred catalogue is not a
/// tuning choice, it is an invalid one. This index is the non-deferred floor.
///
/// Names are sorted so the block is byte-stable across turns: a prompt prefix
/// that reorders cannot be cached, and this one sits ahead of the conversation.
#[must_use]
pub fn render_tool_index(names: &[String]) -> String {
    if names.is_empty() {
        return String::new();
    }
    let mut sorted: Vec<&str> = names.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.dedup();
    format!(
        "\n\n## Your tools\n\n     These are the tools you can call, by name. Names only — call one to \
         see its parameters. This list is generated from the same set your tool \
         surface serves, so a name here is callable and a capability question is \
         answered by reading it, never by guessing.\n\n     {}\n",
        sorted.join(", ")
    )
}
