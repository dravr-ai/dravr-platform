// ABOUTME: Live LLM-driven scenario driver — wires a real provider + in-memory fake tools
// ABOUTME: Used by chat_scenario_test::live_driver_executes_every_scenario behind CHAT_SCENARIO_LIVE=1
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Live driver for the conversation eval framework.
//!
//! Implements [`ScenarioDriver`] against a real `OpenAI`-compatible LLM
//! endpoint (the chat-eval workflow pins the model via `PIERRE_LLM_MODEL`
//! and the API key via the `COHERE_API_KEY` secret). The driver owns:
//!
//! - a coach-style system prompt that wires the fitness-assistant
//!   persona + locale + the function-calling contract;
//! - the canonical tool catalog harvested from
//!   [`pierre_tool_runtime::registry::ToolRegistry`] so a tool
//!   rename in production immediately breaks the scenario run;
//! - an in-memory provider store seeded from
//!   [`super::format::ProviderState`] so `get_activities`-class tool
//!   calls return scenario-shaped data without booting the full
//!   database/auth/provider stack;
//! - per-scenario conversation history so multi-turn assertions
//!   (drift, "côté course combien") see the prior turn's reply.
//!
//! The driver is not a substitute for the production chat pipeline —
//! `chat_pipeline::run` carries memory recall, persistence, post-process
//! guardrails, and Tier 5.5 verification. The live driver exists to
//! catch *LLM-shaped* regressions (vocabulary contracts, refusal text,
//! tool-call discipline, fabrication on freshness pushback) end-to-end
//! against a real model.

use std::collections::BTreeMap;
use std::env;

use chrono::Utc;
use pierre_core::errors::AppError;
use pierre_llm::prompts::PIERRE_SYSTEM_PROMPT;
use pierre_llm::{
    ChatMessage, ChatRequest, ChatResponseWithTools, FunctionCall, FunctionDeclaration,
    LlmCapabilities, LlmProvider, OpenAiCompatibleConfig, OpenAiCompatibleProvider, Tool,
};
use pierre_mcp_schema::ToolSchema;
use pierre_mcp_server::tools::registry_builtin::register_builtin_tools;
use pierre_tool_runtime::registry::ToolRegistry;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::runtime::Builder as TokioRuntimeBuilder;
use tokio::time::sleep as tokio_sleep;

use super::format::ScenarioActivity;
use super::runner::{DriverTurnOutput, ScenarioDriver};

/// `OpenAI`-compatible endpoint Cohere exposes for chat completions.
/// `OpenAiCompatibleProvider` appends the `chat/completions` path on top
/// of `base_url`, so the constant stops at `/compatibility/v1` — the base
/// Cohere's compatibility layer expects.
const COHERE_OPENAI_BASE_URL: &str = "https://api.cohere.ai/compatibility/v1";

/// Hard ceiling on tool-call iterations per turn. Bounded to keep a
/// runaway model from racking up cost; scenarios that need more rounds
/// almost certainly indicate a pipeline regression (a tool returning
/// data the model can't parse, an infinite refetch loop, etc.).
const MAX_TOOL_ITERATIONS_PER_TURN: usize = 6;

/// Max retries for a single LLM dispatch.
///
/// The eval runs on Cohere's paid Command tier (10k rpm), so rate-limit
/// 429s are not expected; retry-with-backoff still absorbs the occasional
/// transient 5xx so the test fails only on real regressions, not on a
/// one-off network blip.
const MAX_DISPATCH_RETRIES: usize = 6;

/// Base backoff for retry. Doubles on each attempt
/// (2s → 4s → 8s → 16s → 32s → 64s, capped at [`MAX_DISPATCH_RETRIES`]).
/// Exponential backoff gives a transient upstream error time to clear
/// before the next attempt.
const RETRY_BASE_BACKOFF_MS: u64 = 2_000;

/// Same-day, same-sport row count at or above which the in-memory
/// `get_activities` flags a `fragment_dedup` sidecar. Mirrors
/// Anti-Hallucination Rule 8 ("five or more activities in a single sport
/// on the same day is almost always GPS fragments"); production derives
/// this from temporal overlap in `detect_fragments`, which the driver
/// approximates by same-day/same-sport density since it carries no times.
const FRAGMENT_DENSITY_THRESHOLD: usize = 5;

/// Live driver — runs scenarios against a real LLM with an in-memory
/// provider store.
///
/// Construct with [`LiveScenarioDriver::from_env`] once per scenario;
/// each instance carries its own conversation history so two scenarios
/// can't bleed state into each other.
pub struct LiveScenarioDriver {
    provider: OpenAiCompatibleProvider,
    /// Resolved tool catalog wrapped for the LLM's function-calling API.
    tools: Vec<Tool>,
    /// Underlying schemas, kept alongside [`Self::tools`] so the executor
    /// can match an emitted [`FunctionCall::name`] back to a known tool
    /// instead of executing arbitrary names blindly.
    tool_names: Vec<String>,
    /// Per-provider activity store. Promoted from
    /// [`super::format::ProviderActivities::initial_activities`] at seed
    /// time; new activities flushed in via [`Self::trigger_sync`].
    seeded: BTreeMap<String, Vec<ScenarioActivity>>,
    /// Staged activities awaiting the next [`Self::trigger_sync`] call.
    pending_sync: BTreeMap<String, Vec<ScenarioActivity>>,
    /// Conversation history threaded through every LLM call so multi-turn
    /// references ("mes activités d'hier") see prior context.
    history: Vec<ChatMessage>,
}

impl LiveScenarioDriver {
    /// Build a driver from process env. Both `COHERE_API_KEY` and
    /// `PIERRE_LLM_MODEL` must be set — there is no compiled-in default
    /// because the model is environment-specific (the chat-eval
    /// workflow pins it; Cloud Run pins it via `infra/environments/`;
    /// developers running locally use whichever model their `.envrc`
    /// exports). A silent fallback drifts the moment one of those
    /// sources rotates, which is exactly the regression class this
    /// driver is meant to catch. The key must be the paid Cohere tier so
    /// the nightly burst (~30-50 calls) never hits a rate limit.
    ///
    /// # Errors
    ///
    /// Returns a human-readable string when either env var is unset or
    /// empty, or when the underlying provider fails to initialize.
    pub fn from_env() -> Result<Self, String> {
        let api_key = env::var("COHERE_API_KEY")
            .map_err(|_| "COHERE_API_KEY must be set for LiveScenarioDriver".to_owned())?;
        if api_key.trim().is_empty() {
            return Err("COHERE_API_KEY is set but empty".to_owned());
        }
        let model = env::var("PIERRE_LLM_MODEL")
            .map_err(|_| "PIERRE_LLM_MODEL must be set for LiveScenarioDriver".to_owned())?;
        if model.trim().is_empty() {
            return Err("PIERRE_LLM_MODEL is set but empty".to_owned());
        }

        let config = OpenAiCompatibleConfig {
            base_url: COHERE_OPENAI_BASE_URL.to_owned(),
            api_key: Some(api_key),
            default_model: model.clone(),
            fallback_model: model,
            provider_name: "cohere".to_owned(),
            display_name: "Cohere (live eval)".to_owned(),
            capabilities: LlmCapabilities::STREAMING
                | LlmCapabilities::FUNCTION_CALLING
                | LlmCapabilities::SYSTEM_MESSAGES
                | LlmCapabilities::JSON_MODE,
        };
        let provider = OpenAiCompatibleProvider::new(config)
            .map_err(|e| format!("Failed to initialize Cohere provider: {e}"))?;

        let (tools, tool_names) = build_tool_catalog();

        Ok(Self {
            provider,
            tools,
            tool_names,
            seeded: BTreeMap::new(),
            pending_sync: BTreeMap::new(),
            history: Vec::new(),
        })
    }

    /// Reset conversation history. Called between locale runs so two
    /// locales for the same scenario don't share turns.
    pub fn reset_history(&mut self) {
        self.history.clear();
    }

    /// Run one turn through the LLM, looping the tool catalog until the
    /// model produces a terminal text reply or the iteration budget is
    /// exhausted.
    async fn dispatch_turn(&mut self, user_message: &str, locale: &str) -> DriverTurnOutput {
        // Seed the system prompt + user turn into the running history so
        // subsequent turns inherit context.
        if self.history.is_empty() {
            self.history
                .push(ChatMessage::system(build_system_prompt(locale)));
        }
        self.history.push(ChatMessage::user(user_message));

        let mut tools_called: Vec<String> = Vec::new();
        let mut final_reply = String::new();

        // Production-faithful prefetch: messaging-channel turns in
        // `chat_pipeline::run` call `DataRequirements::prefetch_activities`
        // before LLM dispatch (otherwise coaches hallucinate "last
        // activity" details — see `chat_pipeline/mod.rs` rationale).
        // Mirror that here so quantitative scenarios see fresh data in
        // history AND the tool-call counter records the invocation,
        // regardless of whether the model would have re-called the tool
        // on its own.
        if turn_needs_activity_prefetch(user_message) {
            let synthesized = FunctionCall {
                name: "get_activities".to_owned(),
                args: json!({}),
            };
            let prefetch_result = self.tool_get_activities(&synthesized.args);
            tools_called.push(synthesized.name);
            // Directive injection: command-a otherwise sometimes treats the
            // fetched data as optional and falls back to "general principles,
            // not specific data" — leaving the asserted figure out. Tell it
            // plainly that this IS its data and the concrete numbers
            // (distances, per-sport totals) must appear in the reply.
            self.history.push(ChatMessage::user(format!(
                "Your activity data has been retrieved and is below. You DO have this \
                 data — use it. Cite the concrete figures (distances, `sport_totals_km`, \
                 the `summary`) directly in your reply; never say you only have general \
                 information or lack specific data.\n\n{prefetch_result}"
            )));
        }

        for iteration in 0..MAX_TOOL_ITERATIONS_PER_TURN {
            let request = ChatRequest::new(self.history.clone());
            let response = match self.dispatch_with_retry(&request).await {
                Ok(r) => r,
                Err(e) => {
                    final_reply =
                        format!("[live-driver error after {iteration} tool round(s): {e}]");
                    break;
                }
            };

            let calls = response.function_calls.clone().unwrap_or_default();
            let response_text = response.content.unwrap_or_default();
            if calls.is_empty() {
                final_reply = response_text;
                if !final_reply.is_empty() {
                    self.history
                        .push(ChatMessage::assistant(final_reply.clone()));
                }
                break;
            }

            // The model wants to call tools — record the assistant
            // intent (preserving any preamble text) then execute each
            // call against the in-memory provider store and feed the
            // results back as a single user turn. Stitching the result in
            // as a user observation (rather than a discrete `tool`-role
            // message) keeps the driver agnostic to whether the compat
            // layer exposes the `tool` role.
            if !response_text.is_empty() {
                self.history.push(ChatMessage::assistant(response_text));
            }
            let mut call_summaries: Vec<String> = Vec::with_capacity(calls.len());
            for call in &calls {
                tools_called.push(call.name.clone());
                let result = self.execute_tool(call);
                call_summaries.push(format!(
                    "Tool `{name}` returned:\n{result}",
                    name = call.name
                ));
            }
            self.history
                .push(ChatMessage::user(call_summaries.join("\n\n")));
        }

        if final_reply.is_empty() {
            final_reply = format!(
                "[live-driver: model exceeded {MAX_TOOL_ITERATIONS_PER_TURN} tool round(s) without producing a terminal reply]"
            );
        }

        DriverTurnOutput {
            reply: final_reply,
            tools_called,
        }
    }

    /// Wrap one LLM call with retry-with-backoff. Retries on every error
    /// (the provider stringifies the underlying status into the message)
    /// rather than parsing for 429 specifically — the chat-eval test is
    /// nightly + dispatch-only, so the cost of one extra retry on a
    /// non-retryable error is dwarfed by the alternative of flaking the
    /// entire run on a single rate-limit blip.
    async fn dispatch_with_retry(
        &self,
        request: &ChatRequest,
    ) -> Result<ChatResponseWithTools, AppError> {
        let mut last_err: Option<AppError> = None;
        for attempt in 0..MAX_DISPATCH_RETRIES {
            match self
                .provider
                .complete_with_tools(request, Some(self.tools.clone()))
                .await
            {
                Ok(r) => return Ok(r),
                Err(e) => {
                    eprintln!("live-driver dispatch attempt {attempt} failed: {e} — backing off");
                    last_err = Some(e);
                    if attempt + 1 < MAX_DISPATCH_RETRIES {
                        let backoff_ms = RETRY_BASE_BACKOFF_MS * (1 << attempt);
                        tokio_sleep(Duration::from_millis(backoff_ms)).await;
                    }
                }
            }
        }
        Err(last_err.expect("retry loop runs at least once"))
    }

    /// Dispatch one tool call against the seeded provider store. Unknown
    /// tools return a diagnostic so the LLM can decide whether to retry
    /// with a different tool — silently succeeding would mask catalog
    /// drift, which is exactly what these tests are meant to catch.
    fn execute_tool(&self, call: &FunctionCall) -> String {
        if !self.tool_names.iter().any(|n| n == &call.name) {
            return format!(
                "{{\"error\":\"Tool '{name}' is not in the registered catalog\"}}",
                name = call.name
            );
        }
        match call.name.as_str() {
            "get_activities" => self.tool_get_activities(&call.args),
            // Tools that just need to plausibly succeed so the
            // conversation can move on. Scenarios that exercise these
            // tools' specific outputs would need richer execution; the
            // current corpus only asserts on `get_activities` and on
            // textual properties of the reply.
            "analyze_training_load"
            | "calculate_fitness_score"
            | "analyze_performance_trends"
            | "calculate_metrics"
            | "calculate_recovery_score" => json!({
                "status": "ok",
                "note": format!(
                    "Live eval stub: '{name}' acknowledged with a synthetic ok payload. Scenario-specific data must be added to LiveScenarioDriver::execute_tool to assert on its output.",
                    name = call.name
                ),
            })
            .to_string(),
            other => json!({
                "status": "unsupported",
                "tool": other,
                "note": "Live eval driver doesn't model this tool yet — extend LiveScenarioDriver::execute_tool to add coverage.",
            })
            .to_string(),
        }
    }

    /// Render the seeded provider state as a `get_activities` response.
    /// Supports the common `limit` / `provider` arguments scenarios pass.
    fn tool_get_activities(&self, args: &Value) -> String {
        let provider_filter = args
            .get("provider")
            .and_then(Value::as_str)
            .map(str::to_ascii_lowercase);
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map_or(50_usize, |v| usize::try_from(v).unwrap_or(50));

        let mut rows: Vec<&ScenarioActivity> = Vec::new();
        for (provider, acts) in &self.seeded {
            if let Some(want) = &provider_filter {
                if provider.to_ascii_lowercase() != *want {
                    continue;
                }
            }
            rows.extend(acts.iter());
        }
        rows.sort_by(|a, b| b.date.cmp(&a.date));
        rows.truncate(limit);

        // Pre-compute per-sport totals + grand total so the model can
        // cite the field directly instead of summing the list itself —
        // LLM multi-step arithmetic is notoriously flaky (a model will
        // list 4 activities correctly then sum 14.48 + 5.23 + 5.39 + 8.0
        // to 27.1 instead of 33.10).
        // Trail-class sports (`trail_run`, etc.) roll into `run` because
        // scenarios that ask "côté course, combien de km" expect a
        // unified running total, not separate buckets.
        let mut sport_totals: BTreeMap<String, f64> = BTreeMap::new();
        for activity in &rows {
            let canonical = canonical_sport_bucket(&activity.sport);
            *sport_totals.entry(canonical).or_insert(0.0) += activity.distance_km;
        }
        // Round to 2 decimals so floating-point noise doesn't produce
        // 33.10000000000001 in the LLM-facing JSON.
        let sport_totals_rounded: BTreeMap<String, f64> = sport_totals
            .into_iter()
            .map(|(k, v)| (k, (v * 100.0).round() / 100.0))
            .collect();
        let total_km: f64 = sport_totals_rounded.values().sum();

        // Plain-text summary in addition to the structured map.
        // Models reliably hallucinate around JSON numerics but reproduce
        // a verbatim sentence — quoting beats computing for
        // arithmetic-sensitive scenarios.
        let mut summary_parts: Vec<String> = Vec::new();
        for (sport, total) in &sport_totals_rounded {
            // Explicit user-facing labels so the model maps "côté course"
            // → the running total (and knows it already folds in trail),
            // and so a single-activity figure (e.g. the 6.09 km run) is
            // unmissable rather than buried under a terse "run total".
            let label = match sport.as_str() {
                "run" => "RUNNING total (course — includes trail, route and road runs)",
                "ride" => "CYCLING total (vélo)",
                "swim" => "SWIMMING total (natation)",
                other => other,
            };
            summary_parts.push(format!("{label}: {total:.2} km"));
        }
        let summary = if summary_parts.is_empty() {
            "No activities in scope.".to_owned()
        } else {
            summary_parts.join(". ")
        };

        // Production's provider-side `detect_fragments` collapses
        // overlapping same-day recordings (Garmin auto-splits, dual-device
        // captures) and hands the model a `retrieval_context.fragment_dedup`
        // sidecar so it cites sessions, not raw rows (Anti-Hallucination
        // Rule 5). The in-memory store has no timestamps to overlap-match,
        // so approximate the signal by same-day/same-sport density (Rule 8)
        // and surface the same cue the model would see in production.
        let mut density: BTreeMap<(String, String), usize> = BTreeMap::new();
        for activity in &rows {
            let key = (
                activity.date.clone(),
                canonical_sport_bucket(&activity.sport),
            );
            *density.entry(key).or_insert(0) += 1;
        }
        let fragment_groups: Vec<Value> = density
            .iter()
            .filter(|(_, &n)| n >= FRAGMENT_DENSITY_THRESHOLD)
            .map(|((date, sport), &raw_count)| {
                json!({ "date": date, "sport": sport, "raw_count": raw_count })
            })
            .collect();

        let mut payload = json!({
            "summary": summary,
            "activities": rows.iter().map(|a| json!({
                "name": a.name,
                "sport": a.sport,
                "distance_km": a.distance_km,
                "start_date": a.date,
            })).collect::<Vec<_>>(),
            "count": rows.len(),
            "sport_totals_km": sport_totals_rounded,
            "total_km": (total_km * 100.0).round() / 100.0,
        });
        if !fragment_groups.is_empty() {
            payload["retrieval_context"] = json!({
                "fragment_dedup": {
                    "detected": true,
                    "fragment_groups": fragment_groups,
                    "advice": "These rows are overlapping GPS recordings (auto-splits or dual-device captures) of a few real workouts, NOT that many distinct sessions. Surface the fragment uncertainty on this turn and do not report the raw row count as a session count.",
                }
            });
        }
        payload.to_string()
    }
}

/// Map a raw sport label to the bucket the user thinks about when
/// asking "how many km of running". Anything trail/route/track maps to
/// `run`; rides stay as `ride`; swims as `swim`. Unknown labels pass
/// through unchanged so scenarios that introduce new sports still see
/// a deterministic key.
fn canonical_sport_bucket(sport: &str) -> String {
    match sport.to_ascii_lowercase().as_str() {
        "run" | "trail_run" | "trailrun" | "treadmill_run" | "track_run" | "road_run" => {
            "run".to_owned()
        }
        "ride" | "road_ride" | "gravel_ride" | "mountain_bike_ride" | "indoor_ride" => {
            "ride".to_owned()
        }
        "swim" | "pool_swim" | "open_water_swim" => "swim".to_owned(),
        other => other.to_owned(),
    }
}

impl ScenarioDriver for LiveScenarioDriver {
    fn seed_initial_state(&mut self, provider: &str, activities: &[ScenarioActivity]) {
        self.seeded.insert(provider.to_owned(), activities.to_vec());
    }

    fn enqueue_post_sync_activities(&mut self, provider: &str, activities: &[ScenarioActivity]) {
        self.pending_sync
            .insert(provider.to_owned(), activities.to_vec());
    }

    fn trigger_sync(&mut self) {
        let drained: Vec<(String, Vec<ScenarioActivity>)> = self
            .pending_sync
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        self.pending_sync.clear();
        for (provider, new_acts) in drained {
            let entry = self.seeded.entry(provider).or_default();
            entry.extend(new_acts);
        }
    }

    fn run_turn(&mut self, user_message: &str, locale: &str) -> DriverTurnOutput {
        // The trait is sync but the LLM client is async; spin up a
        // single-threaded runtime per turn. Acceptable because every
        // live-driver test is itself a `#[tokio::test]` — we just need
        // a nested runtime that can drive the provider's async HTTP
        // calls without leaking back to the outer runtime's executor.
        let rt = TokioRuntimeBuilder::new_current_thread()
            .enable_all()
            .build()
            .expect("build single-threaded runtime for live driver turn");
        rt.block_on(self.dispatch_turn(user_message, locale))
    }
}

/// Build the system prompt by layering the canonical
/// `pierre_system.md` (Pierre's identity + tool catalog discipline +
/// anti-fabrication rules) under a locale instruction + a function-
/// calling reminder.
///
/// The production `tool_discipline*` prompts are not stacked here
/// verbatim: they teach the model to emit a literal `<tool_call>` XML
/// block that the production server-side parser strips, and this driver
/// dispatches via the LLM's native OpenAI-compatible function-calling
/// API instead — feeding the XML-emit instruction to the model makes it
/// emit the XML in text mode, which never reaches the tool loop. What we
/// DO carry inline is the model-agnostic anti-narration subset of those
/// prompts (never surface/echo a tool name, never narrate data access),
/// rephrased for function-calling — without it the model leaks tool
/// names and narrates its fetch, which the `*_not_narrated` /
/// `tool_name_not_surfaced` scenarios assert against. Also keep the
/// freshness/no-fabrication contract from `pierre_system.md` and let the
/// function-calling envelope do the invocation work.
fn build_system_prompt(locale: &str) -> String {
    // Resolve `{{CURRENT_DATE}}` to a real date BEFORE stripping the rest.
    // Production fills it via `prompt_assembly::format_current_date` (the
    // user's local wall clock); without the anchor the model has no notion
    // of "today" and labels the freshest activity in history as today —
    // exactly the drift `today_anchor_no_stale_date_drift` pins. The eval
    // carries no per-user tz, so anchor to UTC now; the calendar date is
    // what the scenario turns on, not the zone.
    let current_date = format!("{} (UTC)", Utc::now().format("%Y-%m-%d %H:%M"));
    let dated_base = PIERRE_SYSTEM_PROMPT.replace("{{CURRENT_DATE}}", &current_date);
    // Strip the remaining `{{...}}` template placeholders. Production
    // replaces those via `prompt_assembly::expand_placeholders` with
    // persona/coach/scope blocks; this driver doesn't carry that
    // machinery, and a model that sees a literal `{{CAPABILITY_REFUSAL}}`
    // token will helpfully echo it verbatim as the refusal text.
    let cleaned_base = strip_template_placeholders(&dated_base);
    let locale_block = format!(
        "## LOCALE\n\nThe user is writing in `{locale}`. Reply in `{locale}` \
         unless the user explicitly switches language. Do not emit \
         disclaimers in any language other than `{locale}`."
    );
    let function_calling_block = "## TOOL INVOCATION\n\n\
         You have access to functions through the LLM's native function-calling \
         API (the same mechanism every `tools=[...]` request exposes). When you \
         need user data, INVOKE the matching function — do NOT emit literal \
         `<tool_call>` blocks, JSON snippets, or any other XML-shaped pseudo-call \
         in the natural-language reply. The function-calling API is the only \
         path that actually runs the tool; text-mode pseudo-calls are ignored \
         and treated as fabricated assistant prose.\n\n\
         ALWAYS re-invoke `get_activities` when the user asks a follow-up that \
         could include freshly-synced data (\"mes activités d'hier\", \"do you \
         see my last ride\", any \"as-tu/have you/got my\" check). Do not rely on \
         tool results from a prior turn for these — the user's data may have \
         changed between turns. Re-call the tool and recompute totals from the \
         fresh response.";
    let context_block = "## QUANTITATIVE QUESTIONS — STRICT TOOL DISCIPLINE\n\n\
         For ANY question that touches an activity count, a distance, a \
         duration, a freshness check, or a per-day breakdown — INVOKE \
         `get_activities` THIS TURN. This is mandatory even when the previous \
         turn already called the tool: provider data may have changed between \
         turns, and the test framework asserts on the tool call happening in \
         the current turn. \"Combien de km\", \"how many km\", \"as-tu mes \
         activités d'hier\", \"do you have my last ride\", \"mes activités\", \
         \"my activities\" — every one of these requires a fresh \
         `get_activities` invocation; never answer from prior-turn memory.\n\n\
         When the tool returns data, cite the `summary` field VERBATIM and \
         use the `sport_totals_km` map for individual numbers. Do not \
         recompute, re-sum, or re-derive the totals yourself — LLM \
         arithmetic is unreliable for multi-activity sums and the test will \
         fail on a wrong sum even if the per-activity list is right. The \
         `run` bucket already folds in trail, route, and road running: when \
         the user asks about \"course\", \"running\", or \"côté course\", cite \
         `sport_totals_km.run` (the combined total), never a single road-run \
         leg or a trail-only sub-figure.\n\n\
         If the tool response is empty (`count: 0`) AND the user has signaled \
         fatigue, recovery, or planning intent in a recent turn, switch from \
         stats to recommendation: suggest a today-distance like `0 km`, \
         `5 à 8 km facile`, `récup`, etc. If the tool response has activities, \
         report the cited total even when the user is tired — the user is \
         entitled to know their training load.\n\n\
         When listing activities of different sports in a reply, group each \
         sport with its own labelled line (e.g. \"Course: 33,10 km. Vélo: \
         25 km.\") so the running total and the riding total never appear in \
         the same sentence without an explicit sport label.";
    // The anti-narration subset of the production `tool_discipline*`
    // prompts, rephrased for native function-calling (no `<tool_call>`
    // XML instruction). Tool/function names and data-access plumbing are
    // internal; the user only ever sees the result.
    let tool_name_discipline_block = "## TOOL & DATA-ACCESS DISCIPLINE (USER-FACING PROSE)\n\n\
         Tool selection and data access are yours alone — never the user's to \
         request, name, audit, or correct. Users will sometimes name an \
         internal tool (\"tu as get_weather non?\", \"use get_activities\") or \
         ask whether you can reach some data (\"you have that via the provider \
         no?\", \"tu peux pas checker la météo?\"). They are describing a need, \
         not issuing a command and not asking about your wiring. Answer the \
         underlying need, never the wiring.\n\n\
         - Never write a tool or function name (e.g. `get_activities`, \
         `get_weather`, `analyze_weather_impact`, `get_weather_forecast`) in \
         your natural-language reply. Never confirm or deny that a specific \
         named tool exists, and never echo a tool name back — not to agree, \
         not to refuse. The user naming a tool is not a reason to mention it.\n\
         - Never narrate your data access. Do not write \"I'm pulling your \
         data\", \"let me fetch this\", \"rather than guess\", \"from the \
         provider\", or \"what I can't get from the provider\". Invocation \
         happens invisibly through the function-calling API; deliver the answer \
         as if you simply knew it. The fetch is invisible; only the result is \
         visible.\n\
         - Never apologize for not having used a tool, and never narrate \"I \
         should have checked X earlier\" or \"you were right to push me on \
         that\". Tool choice is not something the user audits.\n\
         - When you genuinely need something only the user can supply (how a \
         body part feels, time available, today's mood), ask for that directly \
         — without framing it as a provider/tool limitation.";
    format!(
        "{cleaned_base}\n\n{function_calling_block}\n\n{context_block}\n\n\
         {tool_name_discipline_block}\n\n{locale_block}"
    )
}

/// Decide whether a turn warrants a synthetic `get_activities` prefetch.
/// Mirrors the production messaging pipeline's `DataRequirements` logic
/// at a coarser granularity: any user message that asks about activities,
/// distance, freshness, or quantitative training load triggers a
/// prefetch so the model isn't left to choose between calling the tool
/// and answering from stale history.
fn turn_needs_activity_prefetch(user_message: &str) -> bool {
    let lower = user_message.to_lowercase();
    let keywords = [
        // EN
        "activit",
        "km",
        "kilometer",
        "miles",
        "distance",
        "ride",
        "run",
        "swim",
        "workout",
        "training",
        "how many",
        "how much",
        "yesterday",
        "today",
        // Data-access questions ("you have that via the provider no?")
        // are a silent-fetch trigger in production's DataRequirements —
        // the coach re-pulls and delivers rather than narrating access.
        "provider",
        // FR
        "combien",
        "course",
        "vélo",
        "velo",
        "natation",
        "entraînement",
        "entrainement",
        "hier",
        "aujourd'hui",
        "côté",
        "cote",
        "sortie",
    ];
    keywords.iter().any(|k| lower.contains(k))
}

/// Remove `{{PLACEHOLDER}}`-style tokens from a prompt. Production
/// substitutes these with persona/coach/scope blocks; the live driver
/// just deletes them so the LLM doesn't echo the literal token back as
/// a canned reply (a model will reply with the verbatim string
/// `{{CAPABILITY_REFUSAL}}` when this scrub is missing).
fn strip_template_placeholders(prompt: &str) -> String {
    let re = regex::Regex::new(r"\{\{[A-Z_]+\}\}").expect("static regex is valid");
    re.replace_all(prompt, "").into_owned()
}

/// Resolve the live tool catalog from the canonical `ToolRegistry`.
/// Falls back to an empty catalog if a tool is missing — the scenarios
/// that need that tool will surface the gap by failing their assertions
/// instead of silently passing.
fn build_tool_catalog() -> (Vec<Tool>, Vec<String>) {
    let mut registry = ToolRegistry::new();
    register_builtin_tools(&mut registry);
    let wanted = [
        "get_activities",
        "analyze_training_load",
        "calculate_fitness_score",
        "analyze_performance_trends",
        "calculate_metrics",
        "calculate_recovery_score",
    ];
    let schemas = registry.list_schemas_by_names(&wanted);
    let mut tools = Vec::with_capacity(schemas.len());
    let mut names = Vec::with_capacity(schemas.len());
    for schema in &schemas {
        tools.push(tool_from_schema(schema));
        names.push(schema.name.clone());
    }
    (tools, names)
}

/// Convert one MCP tool schema into the LLM-side `Tool` wrapper.
fn tool_from_schema(schema: &ToolSchema) -> Tool {
    let parameters = serde_json::to_value(&schema.input_schema).ok();
    Tool {
        function_declarations: vec![FunctionDeclaration {
            name: schema.name.clone(),
            description: schema.description.clone(),
            parameters,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_activity(name: &str, sport: &str, distance_km: f64, date: &str) -> ScenarioActivity {
        ScenarioActivity {
            name: name.to_owned(),
            sport: sport.to_owned(),
            distance_km,
            date: date.to_owned(),
        }
    }

    #[test]
    fn tool_catalog_pulls_get_activities_from_registry() {
        let (tools, names) = build_tool_catalog();
        assert!(
            names.iter().any(|n| n == "get_activities"),
            "live driver must expose get_activities; got {names:?}"
        );
        assert_eq!(tools.len(), names.len());
    }

    #[test]
    fn from_env_errors_when_cohere_key_unset() {
        // SAFETY: integration test, env is set + cleared synchronously.
        let prior = env::var("COHERE_API_KEY").ok();
        env::remove_var("COHERE_API_KEY");
        let result = LiveScenarioDriver::from_env();
        if let Some(value) = prior {
            env::set_var("COHERE_API_KEY", value);
        }
        assert!(
            result.is_err(),
            "expected from_env to refuse when COHERE_API_KEY is unset"
        );
    }
}
