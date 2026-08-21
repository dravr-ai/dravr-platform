// ABOUTME: DataRequirements-driven activity prefetch stage — deterministically loads activities before LLM dispatch
// ABOUTME: Extracted from routes/chat.rs::ChatRoutes prefetch_activity_context / inject_startup_context / get_startup_context_if_applicable (2026-04-16)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `DataRequirements` activity prefetch.
//!
//! When the conversation's coach has a `data_requirements.activities` block
//! in its YAML frontmatter, this stage deterministically invokes
//! `get_activities` with exact parameters from the coach definition before
//! the LLM runs. The fetched activities are formatted and injected as a
//! system-prompt section ("The following activity data has been pre-loaded
//! for your analysis"), and the coach's startup query (if any) is appended
//! as the analysis instruction.
//!
//! Before this stage was extended to messaging, only web chat's first-turn
//! dispatch ran prefetch. On Telegram / `WhatsApp` / Discord / Slack the coach
//! had to decide to call `get_activities` itself — and when the LLM skipped
//! the call, it would hallucinate activity details (observed 2026-04-16 on
//! a Telegram conversation about route planning between Prévost and
//! Saint-Alexis-des-Monts, where the coach admitted "je n'ai pas encore
//! chargé les détails exacts de cette sortie").
//!
//! First-turn dispatch runs the full startup prefetch
//! ([`inject_startup_context`]: activity data + the coach's startup query).
//! Later turns are grounded by [`maybe_refresh_activity_context`] whenever the
//! coach declares an activity `data_requirements` window: it is re-fetched and
//! injected as fresh context. Whether the athlete's wording "sounds like" a data
//! question is deliberately not consulted — a keyword test there sent a real
//! Telegram turn to the model with no data at all, and the coach answered from
//! memory. That later-turn refresh runs AFTER Tier-1 compaction,
//! so the freshly injected block is never summarized away and cannot desync
//! the `source_ids`/`llm_messages` vectors compaction consumes.

use std::borrow::Cow;
use std::sync::Arc;

use pierre_core::models::coaches::DataRequirements;
use pierre_core::models::CoachRuntimeContext;
use pierre_database::database::MessageRecord;
use tracing::{info, warn};

use pierre_core::models::TenantId;
use pierre_llm::ChatMessage;
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalResponse};

/// Extract the formatted content string from a `get_activities` response.
///
/// Handles both string results and JSON object results by serializing
/// the object to a compact string representation suitable for LLM context.
fn extract_prefetch_content(response: &UniversalResponse) -> String {
    match &response.result {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(value) => serde_json::to_string(value).unwrap_or_default(),
        None => String::new(),
    }
}

/// Parse a coach's `data_requirements` JSON frontmatter into the structured
/// [`DataRequirements`]. A malformed block logs at WARN and degrades to `None`
/// (the turn proceeds without deterministic prefetch) rather than failing.
fn parse_data_requirements(coach_ctx: &CoachRuntimeContext) -> Option<DataRequirements> {
    let json = coach_ctx.data_requirements.as_ref()?;
    match serde_json::from_str::<DataRequirements>(json) {
        Ok(dr) => Some(dr),
        Err(e) => {
            warn!("Failed to parse data_requirements JSON: {e}");
            None
        }
    }
}

/// Decide whether the turn should run the first-turn startup prefetch.
///
/// Returns `Some((query, data_requirements))` when:
/// - This is the first message in the conversation (`history_len == 1`)
/// - The conversation has a resolved coach context
/// - The coach has a `startup_query` or `data_requirements` configured
/// - No guided flow (e.g. the `/pillars` walk) owns the turn
///
/// Returns `None` otherwise. Later turns are handled by
/// [`maybe_refresh_activity_context`], not this gate.
///
/// The guided-flow gate is not cosmetic. Startup grounding injects the coach's
/// `startup_query` as a synthetic **user** message right before the athlete's
/// real one, so a builder coach's "Build an ultra-endurance block for my
/// athlete" arrived as if the athlete had asked for it — on the very turn they
/// were answering a profile question (2026-07-24). A profile interview is
/// grounded in the athlete's words, not in 60 prefetched rides.
#[must_use]
pub fn get_startup_context_if_applicable(
    history_len: usize,
    coach_ctx: Option<&CoachRuntimeContext>,
    guided_flow_active: bool,
) -> Option<(Option<String>, Option<DataRequirements>)> {
    if history_len != 1 || guided_flow_active {
        return None;
    }

    let ctx = coach_ctx?;

    let query = ctx.startup_query.clone();
    let data_reqs = parse_data_requirements(ctx);
    if data_reqs.is_some() {
        info!("Found data_requirements for coach context assembly");
    }

    if query.is_none() && data_reqs.is_none() {
        return None;
    }

    if let Some(q) = &query {
        info!(
            "Found startup query for coach conversation: {}",
            startup_query_preview(q)
        );
    }

    Some((query, data_reqs))
}

/// How much of a coach's `startup_query` the log line shows.
const STARTUP_QUERY_PREVIEW_CHARS: usize = 50;

/// The opening of a coach's `startup_query`, for the log line only.
///
/// Counted in characters rather than bytes. `startup_query` is accepted
/// verbatim by the custom-coach create/update API on a fr-first platform, so a
/// byte slice at a fixed offset panics as soon as that offset lands inside an
/// accented character — and this runs on the `history_len == 1` path, before
/// dispatch, with no panic boundary between it and the turn boundary. The same
/// defect class in the deterministic-bounds scanner destroyed a production turn
/// on 2026-07-28.
#[must_use]
pub fn startup_query_preview(query: &str) -> String {
    query.chars().take(STARTUP_QUERY_PREVIEW_CHARS).collect()
}

/// The tool this stage invokes on the athlete's behalf.
///
/// Exported because a prefetch is a real tool run, just one the platform
/// decided on rather than the model: downstream provenance checks — the
/// `dravr-viz` anti-fabrication gate above all — have to see it, or a chart
/// built from pre-loaded data reads as invented and gets refused.
pub const PREFETCH_TOOL: &str = "get_activities";

/// Pre-fetch activity data based on structured `DataRequirements`.
///
/// Calls `get_activities` deterministically with exact parameters from the
/// coach definition, bypassing LLM interpretation. Returns the activity
/// data as a formatted string for injection into the conversation context,
/// or `None` when the coach has no activity requirements or the tool call
/// fails.
pub async fn prefetch_activity_context(
    executor: &Arc<UniversalExecutor>,
    user_id: &str,
    tenant_id: TenantId,
    data_reqs: &DataRequirements,
) -> Option<String> {
    use pierre_tool_runtime::protocol::UniversalRequest;

    let activities_req = data_reqs.activities.as_ref()?;

    // Build parameters JSON matching what `get_activities` expects.
    let mut params = serde_json::json!({
        "limit": activities_req.count,
        "mode": activities_req.mode,
        "format": activities_req.format,
        "analysis_type": activities_req.analysis_type,
    });

    // Filter by sport_type only when the coach requests exactly one. The
    // downstream `get_activities` filter accepts a single sport type, so a
    // multi-sport coach (e.g. sport_types: [Run, Ride]) omits the filter and
    // fetches across all sports — narrowing to `.first()` would silently drop
    // every requested sport but the first. Over-fetching is corrected by later
    // stages; dropped data is not.
    if let [sport_type] = activities_req.sport_types.as_slice() {
        params["sport_type"] = serde_json::Value::String(String::clone(sport_type));
    }

    // Add time_frame as paired (after, before) timestamps.
    //
    // Strava silently flips activity ordering to oldest-first when only
    // `after` is set — see memory feedback
    // `strava-after-only-returns-ascending`. With `count=10` and a 7-day
    // window, an active athlete who logged 10 sessions on a single day
    // (e.g. ChefFamille's 2026-05-17 block) ends up with the 10 oldest
    // activities in the window, and everything from the next three days
    // gets dropped. Pairing `before = now` with `after` keeps the
    // default newest-first ordering, so the prefetched batch shows the
    // most recent activities up to `count`.
    if let Some(seconds) = activities_req.time_frame_seconds() {
        let now = chrono::Utc::now().timestamp();
        let after = now - seconds;
        params["after"] = serde_json::Value::Number(serde_json::Number::from(after));
        params["before"] = serde_json::Value::Number(serde_json::Number::from(now));
    }

    // Dispatch through the canonical McpTool registry — same path the chat
    // tool loop uses. `execute_tool` lifts the response back into the
    // `UniversalResponse` shape this prefetch already consumes via
    // `extract_prefetch_content`, so the rest of the stage is unchanged.
    let request = UniversalRequest {
        tool_name: PREFETCH_TOOL.to_owned(),
        parameters: params,
        user_id: user_id.to_owned(),
        protocol: "chat".to_owned(),
        tenant_id: Some(tenant_id.to_string()),
        progress_token: None,
        cancellation_token: None,
        progress_reporter: None,
    };

    match executor.execute_tool(request).await {
        Ok(response) => {
            let content = extract_prefetch_content(&response);
            info!(
                content_len = content.len(),
                "Pre-fetched activity context for coach"
            );
            Some(content)
        }
        Err(e) => {
            warn!("Failed to pre-fetch activity context: {e}");
            None
        }
    }
}

/// Inject startup context (pre-fetched data + analysis query) into LLM messages.
///
/// When a coach has `data_requirements`, activity data is fetched
/// deterministically via [`prefetch_activity_context`] and injected as
/// system context immediately after the system prompt. The coach's
/// startup query becomes the analysis instruction appended before the
/// user turn.
///
/// Without `data_requirements`, the startup query is injected as-is for
/// the LLM to interpret (tool-calling fallback).
///
/// Returns `true` when a non-empty activity window reached the prompt — the
/// caller records that as a [`PREFETCH_TOOL`] run so provenance checks can see
/// the platform's own fetch. A startup query alone grounds nothing and returns
/// `false`, as does an empty window.
pub async fn inject_startup_context(
    executor: &Arc<UniversalExecutor>,
    llm_messages: &mut Vec<ChatMessage>,
    history: &[MessageRecord],
    coach_ctx: Option<&CoachRuntimeContext>,
    user_id: &str,
    tenant_id: TenantId,
    guided_flow_active: bool,
) -> bool {
    let Some((startup_query, data_reqs)) =
        get_startup_context_if_applicable(history.len(), coach_ctx, guided_flow_active)
    else {
        return false;
    };

    // Set only when a non-empty activity window actually reached the prompt.
    // An empty window injects nothing, so it grounds nothing.
    let mut prefetched = false;

    if let Some(data_reqs) = &data_reqs {
        // Full context assembly: pre-fetch activity data deterministically.
        // An empty window is not injected — the "pre-loaded for your analysis"
        // banner over zero activities would tell the model it has data it does
        // not (the startup query below still lets it fetch on its own).
        if let Some(activity_context) =
            prefetch_activity_context(executor, user_id, tenant_id, data_reqs).await
        {
            if prefetch_window_is_empty(&activity_context) {
                info!("startup grounding: activity window empty, skipping pre-load injection");
            } else {
                let context_msg = format!(
                    "{STARTUP_GROUNDING_LEAD}\n\n{}",
                    injectable_activity_text(&activity_context)
                );
                log_inject("activity context", &context_msg);
                // `User`, not `System`. The live provider (copilot_headless)
                // keeps only the first system message and filters every other
                // one out of history, so this pre-load never reached the model
                // — the coach looked grounded only when it chose to call
                // `get_activities` itself, which is the non-determinism this
                // injection exists to remove. Index 1 keeps it directly after
                // the system prompt and before the athlete's turn.
                llm_messages.insert(1, ChatMessage::user(&context_msg));
                prefetched = true;
            }
        }

        // Inject the startup query as the analysis instruction (after data context)
        if let Some(query) = &startup_query {
            log_inject("startup query", query);
            let insert_pos = llm_messages.len().saturating_sub(1);
            llm_messages.insert(insert_pos, ChatMessage::user(query));
        }
    } else if let Some(query) = &startup_query {
        // No data_requirements: inject startup query for LLM tool-calling
        log_inject("startup query (no data_requirements)", query);
        llm_messages.insert(1, ChatMessage::user(query));
    }

    prefetched
}

/// Whether a later turn should re-ground the model in fresh activity data.
///
/// True when all hold: it is past the first turn (turn 1 is already grounded by
/// [`inject_startup_context`]); no guided flow owns the turn; a coach is bound;
/// and that coach declares an activity `data_requirements` window. A
/// nutrition/mobility coach with no activity window is left untouched. Pure, so
/// the full gate is unit-testable without an executor.
///
/// Deliberately no test of what the athlete asked. The declared window is the
/// decision — a coach that asks for 12 weeks of activities is a coach whose
/// answers are supposed to stand on them, whatever the wording of the question.
///
/// The guided-flow gate carries real weight on its own: a pillar interview turn
/// must not pull in an activity dump instructing the coach to "base your
/// analysis and any plan on these specific activities", which is the opposite of
/// what a profile interview should do.
#[must_use]
pub fn should_refresh_activity_context(
    history_len: usize,
    coach_ctx: Option<&CoachRuntimeContext>,
    guided_flow_active: bool,
) -> bool {
    if history_len <= 1 || guided_flow_active {
        return false;
    }
    let Some(coach) = coach_ctx else {
        return false;
    };
    let Some(data_reqs) = parse_data_requirements(coach) else {
        return false;
    };
    if data_reqs.activities.is_none() {
        return false;
    }
    // No intent test. There used to be one — a 61-term bilingual keyword list
    // that had to match the athlete's phrasing before their own data was
    // fetched — and it failed exactly the way substring routing always fails:
    // "Montre-moi l'évolution de mon volume hebdomadaire sur les 3 derniers
    // mois" matched none of the 61 terms, so the coach answered a question
    // about training data with no training data, from conversation history
    // alone (live on Telegram, 2026-08-21). Adding "évolution" would only have
    // moved the blind spot.
    //
    // A coach that declares an activity window is a coach whose every answer is
    // supposed to be grounded in it. The window is bounded and cached, so the
    // honest rule is the simple one: if it declared the requirement, satisfy it.
    true
}

/// Positively confirm that a `get_activities` prefetch returned an empty
/// window, so the later-turn refresh never injects a "freshly loaded" banner
/// over zero activities (which would tell the model it has data it does not).
///
/// The `get_activities` envelope always carries a `count` field; we treat the
/// window as empty only when we can parse that field and it is `0`. When the
/// content cannot be parsed we assume non-empty rather than risk dropping real
/// grounding data on a serialization hiccup.
/// Reduce a `get_activities` payload to the part the coach actually reads.
///
/// The tool answers with the same activities two or three times over: a
/// pre-rendered `activity_list` prose block, the structured `activities` (or
/// `activities_toon`) array, and a `retrieval_context` sidecar. Injecting the
/// whole serialized response put every copy in the prompt — 12,448 bytes for a
/// 30-activity window, roughly 3k tokens, on every grounded turn.
///
/// `activity_list` alone carries what the coach cites: date, sport, name,
/// distance, duration, elevation, temperature — one line each — and the
/// formatter folds the fragment-dedup note into it, so the "count sessions, not
/// rows" signal survives without the sidecar. The structured copy exists for
/// tool consumers, and the prefetch is not one; the coach reads prose.
///
/// Falls back to the raw payload whenever the list is absent or empty, so a
/// response shape this does not recognise still reaches the model intact.
fn injectable_activity_text(raw: &str) -> Cow<'_, str> {
    // Owned, not a slice of `raw`: the decoded prose carries real newlines
    // where the JSON holds `\n` escapes, so it is not a substring of the
    // payload and cannot be borrowed out of it.
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .and_then(|value| {
            let list = value.get("activity_list")?.as_str()?;
            (!list.trim().is_empty()).then(|| Cow::Owned(list.to_owned()))
        })
        .unwrap_or(Cow::Borrowed(raw))
}

fn prefetch_window_is_empty(content: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|value| value.get("count").and_then(serde_json::Value::as_u64))
        .is_some_and(|count| count == 0)
}

/// Opening sentence of the later-turn activity-refresh block.
///
/// Named because it is a platform framing that rides in the User channel: it
/// tells the coach that what follows is data the platform loaded, which is
/// authority the athlete's own text must never borrow. `push_history_row`
/// neutralizes it in replayed user rows, and it can only do that against one
/// stated source — see [`super::prompt_builder`].
pub const REFRESH_GROUNDING_LEAD: &str =
    "The athlete's question needs to be grounded in their real training.";

/// Opening sentence of the first-turn activity pre-load block.
///
/// Carries the same platform authority in the User channel as
/// [`REFRESH_GROUNDING_LEAD`], and is neutralized in replayed user rows for the
/// same reason.
pub const STARTUP_GROUNDING_LEAD: &str =
    "The following activity data has been pre-loaded for your analysis:";

/// Insert the fresh-activity system message just before the latest user message.
///
/// That slot is the most salient for a smaller model. Returns whether a message
/// was injected: an empty window skips injection so the model is never told it
/// has data it does not. Pure (no I/O), so the injection contract is
/// unit-testable without an executor.
pub fn inject_activity_refresh(
    llm_messages: &mut Vec<ChatMessage>,
    activity_context: &str,
) -> bool {
    if prefetch_window_is_empty(activity_context) {
        info!("later-turn grounding: activity window empty, skipping injection");
        return false;
    }
    let context_msg = format!(
        "{REFRESH_GROUNDING_LEAD} \
         The following activity data was freshly loaded for this turn — base \
         your analysis and any plan on these specific activities, cite them by \
         name and date, infer the sport mix from them rather than asking, and \
         do not answer from memory:\n\n\
         {}",
        injectable_activity_text(activity_context)
    );
    log_inject("activity refresh", &context_msg);
    // `User`, not `System` — same reason as the startup pre-load above: a
    // mid-list system message is dropped wholesale by the live provider, so
    // this refresh has never reached the model.
    //
    // Clamped to >= 1 so a degenerate single-message vector cannot displace the
    // system prompt from index 0: `non_system_count` and `sliding_window_to_fit`
    // both infer the system prompt from `messages.first()`, and a `User` there
    // would make the prompt itself eligible for the sliding window.
    let insert_pos = llm_messages.len().saturating_sub(1).max(1);
    llm_messages.insert(insert_pos, ChatMessage::user(&context_msg));
    true
}

/// Later-turn activity refresh — the belt-and-suspenders for a smaller model.
///
/// [`inject_startup_context`] deterministically grounds a coach conversation in
/// real activities, but only on its first message. On every later turn the
/// model had to *choose* to call `get_activities` itself; when it skipped that
/// call, a "fais-moi un plan" / "analyse ma charge" request was answered from
/// the persona prompt alone, producing the generic, ungrounded plans this stage
/// fixes. When [`should_refresh_activity_context`] holds, this stage re-fetches
/// the coach's activity window and injects it via [`inject_activity_refresh`].
///
/// Must run AFTER [`super::compaction::apply_tier1_compaction`]: the injected
/// block is then safe from summarization, and the insertion cannot desync the
/// `source_ids`/`llm_messages` parallel vectors that compaction consumes.
///
/// Returns `true` when a refreshed window reached the prompt, which the caller
/// records as a [`PREFETCH_TOOL`] run — same provenance contract as
/// [`inject_startup_context`].
pub async fn maybe_refresh_activity_context(
    inputs: ActivityRefreshInputs<'_>,
    llm_messages: &mut Vec<ChatMessage>,
) -> bool {
    let ActivityRefreshInputs {
        executor,
        history,
        coach_ctx,
        user_id,
        tenant_id,
        guided_flow_active,
    } = inputs;
    if !should_refresh_activity_context(history.len(), coach_ctx, guided_flow_active) {
        return false;
    }

    // The gate above guaranteed a coach with a parseable activity window; this
    // re-extracts it for the fetch parameters (a deterministic re-parse, not a
    // second decision — the gate remains the single decision authority).
    let Some(data_reqs) = coach_ctx.and_then(parse_data_requirements) else {
        return false;
    };

    let Some(activity_context) =
        prefetch_activity_context(executor, user_id, tenant_id, &data_reqs).await
    else {
        return false;
    };

    // Propagate rather than assume: an empty window injects nothing, so it
    // grounds nothing and must not be reported as a fetch that happened.
    inject_activity_refresh(llm_messages, &activity_context)
}

/// Read-only inputs for [`maybe_refresh_activity_context`], bundled so the
/// mutable `llm_messages` borrow stays the only positional argument.
pub struct ActivityRefreshInputs<'a> {
    /// Tool executor used to re-fetch the coach's activity window.
    pub executor: &'a Arc<UniversalExecutor>,
    /// Persisted conversation history — only its length is consulted (turn 1
    /// belongs to [`inject_startup_context`]).
    pub history: &'a [MessageRecord],
    /// Active coach runtime context, when one is bound.
    pub coach_ctx: Option<&'a CoachRuntimeContext>,
    /// Athlete whose activities are fetched.
    pub user_id: &'a str,
    /// Tenant the activity data lives under.
    pub tenant_id: TenantId,
    /// Whether a guided conversational flow owns this turn.
    pub guided_flow_active: bool,
}

/// Emit `info!` (length) + `trace!` (full body) for a prefetch injection.
///
/// Centralized so each call site in [`inject_startup_context`] stays a
/// single line and the function fits inside the workspace cognitive
/// complexity budget.
fn log_inject(kind: &'static str, body: &str) {
    info!(
        kind,
        body_len = body.len(),
        "prefetch: injecting into prompt"
    );
    tracing::trace!(kind, body = %body, "prefetch: injection body");
}
