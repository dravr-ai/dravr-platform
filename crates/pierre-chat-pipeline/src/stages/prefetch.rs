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
//! Later turns are grounded on demand by [`maybe_refresh_activity_context`]:
//! when the coach declares activity `data_requirements` and the latest user
//! message asks for something that must be grounded in real training
//! ([`needs_activity_grounding`] — a plan, analysis, comparison, or
//! recommendation), the coach's activity window is re-fetched and injected as
//! fresh system context. That later-turn refresh runs AFTER Tier-1 compaction,
//! so the freshly injected block is never summarized away and cannot desync
//! the `source_ids`/`llm_messages` vectors compaction consumes.

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
///
/// Returns `None` otherwise. Later turns are handled by
/// [`maybe_refresh_activity_context`], not this gate.
#[must_use]
pub fn get_startup_context_if_applicable(
    history_len: usize,
    coach_ctx: Option<&CoachRuntimeContext>,
) -> Option<(Option<String>, Option<DataRequirements>)> {
    if history_len != 1 {
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
            &q[..q.len().min(50)]
        );
    }

    Some((query, data_reqs))
}

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
        tool_name: "get_activities".to_owned(),
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
pub async fn inject_startup_context(
    executor: &Arc<UniversalExecutor>,
    llm_messages: &mut Vec<ChatMessage>,
    history: &[MessageRecord],
    coach_ctx: Option<&CoachRuntimeContext>,
    user_id: &str,
    tenant_id: TenantId,
) {
    let Some((startup_query, data_reqs)) =
        get_startup_context_if_applicable(history.len(), coach_ctx)
    else {
        return;
    };

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
                    "The following activity data has been pre-loaded for your analysis:\n\n\
                     {activity_context}"
                );
                log_inject("activity context", &context_msg);
                llm_messages.insert(1, ChatMessage::system(&context_msg));
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
}

/// Lowercase substrings that mark the latest user message as a turn whose
/// answer must be grounded in the athlete's real recent activities — a plan,
/// a review/analysis, a comparison, or a data-driven recommendation. These
/// are exactly the asks that read as generic when the model answers from the
/// coach persona alone instead of the activity list. Bilingual (fr/en) because
/// the coaching surface is fr-first; matching is case-insensitive containment,
/// so stems (`analys`, `recommand`, `entraîne`) cover their inflections.
///
/// Kept deliberately separate from the tool-prefilter's `KEYWORD_RULES`: that
/// map decides which *tools* to expose, a different concern that must not
/// silently change activity-grounding behavior when its keywords are tuned.
const GROUNDING_INTENT_TERMS: &[&str] = &[
    // Planning / prescription — needs the real training history to be specific.
    "plan",
    "programme",
    "program",
    "séance",
    "seance",
    "workout",
    "entraîne",
    "entraine",
    "semaine",
    "week",
    "taper",
    "périodis",
    "periodis",
    // Analysis / review / comparison — must cite real activities.
    "analys",
    "compare",
    "comparer",
    "comparaison",
    "progress",
    "progrès",
    "progres",
    "tendance",
    "trend",
    "charge",
    "training load",
    "forme",
    "performance",
    // Recommendation / guidance grounded in the athlete's data.
    "recommand",
    "recommend",
    "conseil",
    "suggèr",
    "sugger",
    "suggest",
    "que dois-je",
    "que faire",
    "what should i",
    "how am i",
    "comment je m",
    "dois-je",
    // Temporal + meal + outing words: a "qu'est-ce que je mange/fais
    // aujourd'hui" or "ma course de ce soir" question must ground in real
    // recent training (the coach's meal/session advice depends on today's
    // load). Their absence let the 2026-07-24 coach decline to fetch on a
    // recommendation turn. Errs toward a wasted fetch, per the contract above.
    "aujourd'hui",
    "aujourd hui",
    "today",
    "hier",
    "yesterday",
    "ce soir",
    "tonight",
    "demain",
    "tomorrow",
    "cette semaine",
    "dîner",
    "diner",
    "souper",
    "déjeuner",
    "dejeuner",
    "manger",
    "repas",
    "course",
    "sortie",
    "ravito",
    "ravitaillement",
];

/// True when the latest user message needs grounding in real activities.
///
/// A plan, analysis, comparison, or recommendation must be answered from the
/// athlete's real recent activities, not the coach persona alone. Errs toward
/// `true`: a false positive costs one wasted activity fetch, while a false
/// negative reproduces the ungrounded-generic-plan failure this stage prevents.
#[must_use]
pub fn needs_activity_grounding(message: &str) -> bool {
    let lower = message.to_lowercase();
    GROUNDING_INTENT_TERMS
        .iter()
        .any(|&term| lower.contains(term))
}

/// Whether a later turn should re-ground the model in fresh activity data.
///
/// True only when all hold: it is past the first turn (turn 1 is already
/// grounded by [`inject_startup_context`]); a coach is bound; the coach
/// declares an activity `data_requirements` window; and the latest user
/// message [`needs_activity_grounding`]. A nutrition/mobility coach with no
/// activity window is left untouched. Pure, so the full gate is unit-testable
/// without an executor.
#[must_use]
pub fn should_refresh_activity_context(
    history_len: usize,
    coach_ctx: Option<&CoachRuntimeContext>,
    latest_user_message: &str,
) -> bool {
    if history_len <= 1 {
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
    needs_activity_grounding(latest_user_message)
}

/// Positively confirm that a `get_activities` prefetch returned an empty
/// window, so the later-turn refresh never injects a "freshly loaded" banner
/// over zero activities (which would tell the model it has data it does not).
///
/// The `get_activities` envelope always carries a `count` field; we treat the
/// window as empty only when we can parse that field and it is `0`. When the
/// content cannot be parsed we assume non-empty rather than risk dropping real
/// grounding data on a serialization hiccup.
fn prefetch_window_is_empty(content: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|value| value.get("count").and_then(serde_json::Value::as_u64))
        .is_some_and(|count| count == 0)
}

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
        "The athlete's question needs to be grounded in their real training. \
         The following activity data was freshly loaded for this turn — base \
         your analysis and any plan on these specific activities, cite them by \
         name and date, infer the sport mix from them rather than asking, and \
         do not answer from memory:\n\n\
         {activity_context}"
    );
    log_inject("activity refresh", &context_msg);
    let insert_pos = llm_messages.len().saturating_sub(1);
    llm_messages.insert(insert_pos, ChatMessage::system(&context_msg));
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
pub async fn maybe_refresh_activity_context(
    executor: &Arc<UniversalExecutor>,
    llm_messages: &mut Vec<ChatMessage>,
    history: &[MessageRecord],
    coach_ctx: Option<&CoachRuntimeContext>,
    user_id: &str,
    tenant_id: TenantId,
    latest_user_message: &str,
) {
    if !should_refresh_activity_context(history.len(), coach_ctx, latest_user_message) {
        return;
    }

    // The gate above guaranteed a coach with a parseable activity window; this
    // re-extracts it for the fetch parameters (a deterministic re-parse, not a
    // second decision — the gate remains the single decision authority).
    let Some(data_reqs) = coach_ctx.and_then(parse_data_requirements) else {
        return;
    };

    let Some(activity_context) =
        prefetch_activity_context(executor, user_id, tenant_id, &data_reqs).await
    else {
        return;
    };

    inject_activity_refresh(llm_messages, &activity_context);
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
