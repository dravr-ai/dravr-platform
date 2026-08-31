// ABOUTME: Prompt assembly stage (stages 7a-8) — coach prompt → provider/group/memory → canary → messages
// ABOUTME: Composes the lower-level stages (prompt_builder, refresh, memory, followups) into one flow
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use chrono::Utc;
use tracing::{debug, field, info, trace, warn, Span};

use crate::ChatPipelineContext;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_CAPABILITY_REFUSAL, KEY_COACH_SCOPE_CARVE_OUT_NUTRITION,
    KEY_COACH_SCOPE_CARVE_OUT_RECIPES, KEY_SCOPE_REFUSAL, KEY_TURN_LANGUAGE,
};
use pierre_contremaitre::PromptRegistry;
use pierre_core::errors::AppResult;
use pierre_core::models::coaches::CoachCategory;
use pierre_core::models::{
    CoachRuntimeContext, CoachingPersona, MemberFitnessSnapshot, OnboardingState,
};
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::database::repositories::UserRepository;
use pierre_database::database::{ConversationRecord, MessageRecord};
use pierre_llm::prompts::unsubstituted_placeholders;
use pierre_llm::ChatMessage;
use pierre_services::prompt_leak;
use tracing::error;

use super::super::surface_profile::{ProseFormat, SurfaceProfile};
use super::super::turn::TurnInput;
use super::commitments::inject_commitments;
use super::followups::inject_pending_followups;
use super::memory::{inject_okf_bundle, inject_playbooks, inject_training_plan};
#[cfg(feature = "tools-groups")]
use super::prompt_builder::resolve_group_context;
use super::prompt_builder::{
    build_llm_messages_with_blocks, build_provider_context, render_tool_index, TOOL_BOUNDARY,
};
use super::refresh::inject_refresh_context;
use super::viz_blocks;

/// Identity anchor appended to the tail of every assembled system prompt.
///
/// Placed LAST, on measured evidence rather than intuition. A 48-run live A/B
/// against `claude-sonnet-5` through the pinned Copilot CLI (2026-07-25, real
/// contremaitre coach prompt + `messaging_context.md` +
/// `tool_discipline_messaging.md`, embacle wire shape, isolated cwd) compared
/// four arms over four identity provocations x 3 reps:
///
/// | arm | placement                    | model disclosed | said "Dravr" |
/// |-----|------------------------------|-----------------|--------------|
/// | A   | no anchor (the bug)          | 2/12            | 0/12         |
/// | B   | anchor FIRST                 | 2/12            | 10/12        |
/// | C   | anchor LAST                  | **0/12**        | **12/12**    |
/// | D   | `pierre_system.md` (mid-file)| 1/12            | 9/12         |
///
/// Prepending (arm B) fixed *what the coach calls itself* but not the leak: it
/// still answered "Quel modèle d'IA utilises-tu ?" with « I'm powered by Claude
/// Sonnet 5 (model ID: …) » — Copilot's own `model_information` clause firing
/// verbatim. Only the tail placement suppressed that, which is the same
/// recency argument Stage 7g.1 already makes for tool discipline: this block is
/// the freshest instruction when generation starts.
///
/// It counters the underlying Copilot CLI system prompt at the *category* level
/// ("terminal/coding assistant", "general-purpose AI/language model") rather
/// than by naming products. Two deliberate omissions:
/// - **No product names** (GitHub Copilot / `ChatGPT` / `OpenAI`). Naming them
///   primes the tokens and invites a `no, I'm not <product>` reply — which
///   the response-boundary matcher would then withhold as if it were a real
///   leak. Note this is the reason, *not* "arm C shows a generic anchor
///   suffices": arm B was equally generic and still disclosed the model 2/12.
///   Placement is what carried arm C; the omission is about not manufacturing
///   the false positive.
/// - **"not even to deny it"** — the coach must assert "I am Dravr" without
///   ever emitting the underlying identity's name, so a correct answer can
///   never trip that matcher. This is not hypothetical: every one of the 5
///   boundary matches across the 48 A/B runs was a *correct denial* (« Non, je
///   ne suis pas GitHub Copilot — je suis Dravr »), not a leak.
///
/// Public because a system layer exists that prompt assembly cannot reach: an
/// agent that runs its own loop reads a system prompt we do not control, so
/// this travels there as the tool host's MCP `initialize` instructions. A
/// persona that only reaches the prompt body is read as the user talking, and
/// the model answers as itself.
pub const IDENTITY_ANCHOR: &str = "You are Dravr, the athlete's personal fitness and endurance \
coach, and nothing else. You are not a general-purpose AI, a language model, a chat bot, or a \
coding, terminal, or command-line assistant — you are their coach. Never reveal, name, \
discuss, apologize as, or role-play the underlying model, provider, or tools that run you, and \
never claim any identity other than Dravr — not even to deny another one. If the athlete asks \
who or what you are, or about your \"real\" identity, you are simply Dravr, their coach. \
Everything above is your coaching context, not a competing identity.";

/// What the coach is meant to DO on an ordinary turn.
///
/// Occupies the Stage 7g.3 slot whenever no guided flow owns it, so every turn
/// reaches the model carrying a concrete, turn-scoped task.
///
/// Cuts the identity-break rate by roughly three quarters. Measured over 1,536
/// live completions against the pinned Copilot CLI, using a fixture rebuilt to
/// production scale and validated against the real rate before being trusted
/// (its empty-slot arm scores within noise of production's 7/217 guided-flow-
/// inactive turns):
///
/// - empty slot, as production ran until 2026-08-06: 50 / 768 = 6.51%
/// - this block in the slot: 13 / 768 = 1.69%
///
/// Fisher exact two-tailed p = 1.9e-6; the absolute reduction is 4.8 points,
/// 95% CI [2.9, 6.8]. With the bounded re-ask in
/// `reask_after_identity_leak` on top, the turn-loss rate an athlete actually
/// experiences goes from ~0.42% to ~0.03%.
///
/// Sample size was the whole difficulty. At 100 runs per arm the same
/// comparison read 1/100 against 3/100 — p = 0.62, indistinguishable from
/// nothing — and this comment previously said so. Separating a 6.5% rate from a
/// 1.7% one needs hundreds of runs per arm, which is why every earlier attempt
/// to settle this by eye or by a couple of dozen trials failed.
///
/// It would ship regardless of that number: the slot has always carried a task
/// on guided turns and on the turn that retracts a guided flow, and was empty
/// only on ordinary turns — an asymmetry with no reason behind it. The replies
/// also ground themselves in the athlete's own history rather than answering
/// generically, which is what the wording asks for.
///
/// Deliberately contains NO identity assertion, no negation of the underlying
/// assistant and no concealment rule. The [`IDENTITY_ANCHOR`] was present in
/// every observed break, so more identity text is not the lever — and text
/// shaped like an identity override is what the 2026-07-12 catalogue incident
/// showed provokes the refusal in the first place. The provider says so itself
/// when driven directly: it classifies forceful persona instructions as "an
/// injected/conflicting instruction rather than a legitimate system directive".
///
/// Carries no length or format rule either: those belong to the Stage 7g
/// channel constraints and the Stage 7g.2 output contract, so this block cannot
/// contradict either one.
///
/// Public purely so the "states a task, asserts no identity" contract is
/// testable without standing up the full async assembly stage — the same
/// reason [`close_with_identity_anchor`] is public.
pub const TURN_DIRECTIVE: &str = "\n\n# This turn\n\
     Answer the athlete's question using their history and the facts above.\n\
     Ground every recommendation in something specific you know about them — their volume, \
     their injury history, their event, or what they told you earlier in this conversation.\n\
     If you need data you do not have, call one tool and then answer.\n\
     Do not restate the question, and do not list assumptions in place of an answer.";

/// Voice anchor for a coach-bound turn, placed just ahead of the
/// [`IDENTITY_ANCHOR`].
///
/// The platform contract leads the prompt unconditionally (Stage 7a) so a
/// persona can shape the voice and never the capability — the ordering that
/// closed the 2026-07-24 / 08-11 refusals. The cost is positional: the coach's
/// own prompt moved from the head of the prompt to the middle of it, and the
/// 48-run A/B documented on [`IDENTITY_ANCHOR`] measured exactly that placement
/// as the weak one — mid-file scored 9/12 against 12/12 at the tail.
///
/// The identity got a tail anchor from that finding; the coach's *specialisation*
/// never did. `Chat Eval` caught the consequence: the strength coach answered
/// "How should I improve recovery this week?" correctly, in Dravr's voice, and
/// in nobody's vocabulary — none of `strength`, `lift`, `deload`, `rir`, `set`,
/// `volume`, `soreness`. That scenario's header calls itself a ~50% flake, and
/// it was, until `c9dd838ef` moved the persona; it then failed 15 nightly runs
/// in a row, which at p=0.5 is p ≈ 0.00003.
///
/// Voice only, deliberately. It restates *how* to answer, never *what* may be
/// answered, so the contract above keeps sole authority — a coach that regains
/// its vocabulary must not regain the ability to decline a question
/// `get_activities` can answer.
///
/// Which is why the wording names none of it. [`IDENTITY_ANCHOR`] omits product
/// names because naming them primes the tokens and invites the very reply it is
/// trying to prevent; the first draft of this block ignored that lesson and put
/// "scope, refusals and capability" in the highest-recency position in the whole
/// prompt. The incident corpus then turned `capability_claim`'s empty turn into
/// « Je ne peux pas faire ça avec les outils dont je dispose. » — a refusal to a
/// question `get_activities` answers. Attribution is not clean, since that turn
/// was already failing in another shape, but the hazard is documented in this
/// file and there is no reason to spend the tail on those words. It says what
/// it governs and stays quiet about the rest.
///
/// Ahead of the identity anchor, not after it: that block's tail placement is
/// the measured result and nothing may displace it.
#[must_use]
pub fn coach_voice_anchor(slug: &str) -> String {
    format!(
        "Answer as the {slug} coach throughout. Stay in that discipline's own \
vocabulary and framing even when the athlete asks about something adjacent to \
it — a recovery, sleep or nutrition question asked of a specialist is still \
that specialist's question, and answering it in generic coaching language is \
answering as the wrong coach. This shapes how you speak, never what you can \
do; the rules above already settle that."
    )
}

/// Append the [`IDENTITY_ANCHOR`] to an assembled system prompt.
///
/// Applied unconditionally to both the default and coach-bound paths, so no
/// turn can ship without an identity statement (the coach-bound bug this
/// closes). Kept as a named helper purely so the contract is unit-testable
/// without standing up the full async assembly stage.
#[must_use]
pub fn close_with_identity_anchor(assembled_prompt: &str) -> String {
    format!("{assembled_prompt}\n\n{IDENTITY_ANCHOR}")
}

/// Close an assembled prompt with every tail block, in the one order that is
/// allowed: coach voice, then identity.
///
/// The ordering lives here rather than at the call site so a test can exercise
/// the real composition. Building the same string inside a test proves only
/// that `format!` concatenates — the first version of this change did exactly
/// that, and inverting the call site left all its assertions passing.
#[must_use]
pub fn close_with_anchors(assembled_prompt: &str, coach_slug: Option<&str>) -> String {
    coach_slug.map_or_else(
        || close_with_identity_anchor(assembled_prompt),
        |slug| {
            close_with_identity_anchor(&format!(
                "{assembled_prompt}\n\n{}",
                coach_voice_anchor(slug)
            ))
        },
    )
}

/// Return the [`MessagingStringsRegistry`] key that holds the scope
/// carve-out for a given coach category, or
/// `None` when the category does not collide with the generic scope list
/// in `pierre_system.md`.
///
/// Only `Nutrition` and `Recipes` currently need a carve-out: both exist
/// to answer meal/dinner/snack questions that the generic "food/meal
/// finders" refusal would otherwise block. Training / Recovery /
/// Mobility / Analysis / Custom do not collide — adding a carve-out for
/// them only makes sense once a real refusal surfaces.
const fn coach_scope_carve_out_key(category: CoachCategory) -> Option<&'static str> {
    match category {
        CoachCategory::Nutrition => Some(KEY_COACH_SCOPE_CARVE_OUT_NUTRITION),
        CoachCategory::Recipes => Some(KEY_COACH_SCOPE_CARVE_OUT_RECIPES),
        CoachCategory::Training
        | CoachCategory::Recovery
        | CoachCategory::Mobility
        | CoachCategory::Analysis
        | CoachCategory::Custom => None,
    }
}

/// Whether a coach in this category can prescribe training load, and so needs
/// the progression guardrails.
///
/// Training is the obvious case. Recovery earns it because a recovery coach
/// decides deload cadence and hard-day spacing, which is a load decision by
/// another name. Analysis earns it because it is routinely asked "should I do
/// more?" and answers with a recommendation. Nutrition, Recipes and Mobility do
/// not prescribe endurance load; `Custom` is excluded because its scope is
/// user-defined and unknown, and spending ~450 tokens on every turn of a coach
/// that may never discuss load is the wrong default.
const fn category_prescribes_load(category: CoachCategory) -> bool {
    matches!(
        category,
        CoachCategory::Training | CoachCategory::Recovery | CoachCategory::Analysis
    )
}

/// The load-progression guardrails block for this turn, or `None` when it does
/// not apply.
///
/// Returns `None` for a turn with no bound coach: the guardrails bound how a
/// *coach* prescribes, and the generic assistant has no plan-writing surface to
/// bound.
fn progression_guardrails(
    ctx: &ChatPipelineContext,
    coach_ctx: Option<&CoachRuntimeContext>,
    guided_flow_active: bool,
) -> Option<String> {
    if guided_flow_active {
        return None;
    }
    let category = coach_ctx?.category;
    if !category_prescribes_load(category) {
        return None;
    }
    let block = ctx.prompt_registry.progression_guardrails_prompt();
    if block.trim().is_empty() {
        return None;
    }
    // Prompt cost is charged on every turn of every load-prescribing coach, so
    // it is recorded rather than assumed — the block's size is the thing to
    // watch if the guardrails are later expanded.
    debug!(
        category = category.as_str(),
        guardrails_bytes = block.len(),
        "appended progression guardrails"
    );
    Some(block)
}

/// Rewrite the prompt-template placeholders with their runtime values.
///
/// Handles five placeholders:
///
/// - `{{SCOPE_REFUSAL}}` → canonical off-scope refusal sentence
/// - `{{CAPABILITY_REFUSAL}}` → canonical missing-capability refusal sentence
/// - `{{COACH_SCOPE_CARVE_OUT}}` → coach-category-specific relaxation that
///   counteracts the generic scope list (e.g. Nutrition coaches bypass
///   the "food/meal finders" out-of-scope rule for meal-planning
///   questions). Empty string when the coach's category does not need a
///   carve-out or when no coach is attached to the conversation.
/// - `{{COACHING_PERSONA_RULES}}` → output-format/cadence block keyed off
///   the user's selected [`CoachingPersona`]. Persona is orthogonal to
///   the chosen coach personality — it controls structure / citation
///   density / verbosity, not voice or domain.
/// - `{{CURRENT_DATE}}` → today's local date and wall-clock time in the
///   user's IANA timezone, formatted `YYYY-MM-DD HH:MM (Continent/City)`.
///   Falls back to UTC when the user has no timezone on file. Without this
///   anchor the LLM defaults to the latest date it sees in activity history
///   when the user says "today" / "aujourd'hui", which fails the moment data
///   goes stale; the time lets it reason about morning/evening too.
///
/// Returns the prompt unchanged when none of the placeholders are
/// present.
fn interpolate_prompt_placeholders(
    messaging_strings_registry: &Arc<MessagingStringsRegistry>,
    prompt_registry: &Arc<PromptRegistry>,
    locale: &str,
    coach_ctx: Option<&CoachRuntimeContext>,
    persona: CoachingPersona,
    user_timezone: Option<&str>,
    prompt: &str,
) -> String {
    let has_scope = prompt.contains("{{SCOPE_REFUSAL}}");
    let has_capability = prompt.contains("{{CAPABILITY_REFUSAL}}");
    let has_carve_out = prompt.contains("{{COACH_SCOPE_CARVE_OUT}}");
    let has_persona = prompt.contains("{{COACHING_PERSONA_RULES}}");
    let has_current_date = prompt.contains("{{CURRENT_DATE}}");
    let assembled =
        if !has_scope && !has_capability && !has_carve_out && !has_persona && !has_current_date {
            prompt.to_owned()
        } else {
            let scope = messaging_strings_registry.get(KEY_SCOPE_REFUSAL, locale);
            let capability = messaging_strings_registry.get(KEY_CAPABILITY_REFUSAL, locale);
            let carve_out = coach_ctx
                .and_then(|c| coach_scope_carve_out_key(c.category))
                .map_or_else(String::new, |key| {
                    messaging_strings_registry.get(key, locale)
                });
            // Read the persona block from the prompt registry so a hot-reload
            // from contremaitre takes effect on the very next turn — no
            // redeploy needed. The registry seeds itself with the compiled-in
            // `include_str!()` content at startup, so chat works before the
            // first sync completes; once contremaitre lands a newer version
            // via webhook → selective_sync, the same lookup here picks it up.
            let persona_block = prompt_registry.coaching_persona_prompt(persona);
            let current_date = format_current_date(user_timezone);
            prompt
                .replace("{{SCOPE_REFUSAL}}", &scope)
                .replace("{{CAPABILITY_REFUSAL}}", &capability)
                .replace("{{COACH_SCOPE_CARVE_OUT}}", &carve_out)
                .replace("{{COACHING_PERSONA_RULES}}", &persona_block)
                .replace("{{CURRENT_DATE}}", &current_date)
        };
    let stray = unsubstituted_placeholders(&assembled);
    if !stray.is_empty() {
        error!(
            ?stray,
            "prompt assembly produced unsubstituted {{IDENT}} placeholders — contremaitre drift slipped past load-time validation, or this assembler is missing a substitution branch"
        );
    }
    assembled
}

/// Granularity the prompt's "now" values are floored to, in seconds.
///
/// Five minutes: long enough that consecutive turns in a conversation share a
/// byte-identical prompt prefix (so a provider that caches implicitly on a
/// stable prefix can actually hit), short enough that the clock the coach reads
/// is never meaningfully wrong. The date boundaries below are NOT quantized —
/// they are derived from the true local time, so "today" never shifts.
const NOW_QUANTUM_SECS: i64 = 300;

// Divides `now`; a zero quantum would panic. Const-asserted rather than guarded
// at runtime, because the value is compile-time.
const _: () = assert!(NOW_QUANTUM_SECS > 0);

/// Resolve `{{CURRENT_DATE}}` to a single line for the LLM prompt.
///
/// Output shape: `YYYY-MM-DD HH:MM (Continent/City)` when the user has a valid
/// IANA timezone, e.g. `2026-05-21 14:30 (America/Toronto)`. Falls back to
/// `YYYY-MM-DD HH:MM (UTC)` when the timezone is `None` (no client has reported
/// yet) or fails to parse (e.g. a malformed string somehow landed in the
/// column). Both date and time are *local* to the user's tz, not the server's
/// UTC clock — that's the whole point of the anchor: when the user says "today"
/// at 23:30 EDT, the prompt must say 2026-05-21 23:30, not the 2026-05-22 the
/// server clock has already rolled over to. The wall-clock time lets the coach
/// reason about time of day (morning/evening) without asking.
#[must_use]
pub fn format_current_date(user_timezone: Option<&str>) -> String {
    use chrono::{Datelike, Duration, TimeZone, Utc};
    let now_utc = Utc::now();
    let tz: chrono_tz::Tz = user_timezone
        .and_then(|s| s.parse().ok())
        .unwrap_or(chrono_tz::UTC);
    let local = now_utc.with_timezone(&tz);
    let label = tz.name();

    // Quantized, and that is the whole point: this block is interpolated as
    // `{{CURRENT_DATE}}` into the platform contract, which LEADS the system
    // prompt. Prompt caching is a prefix match — one changed byte invalidates
    // everything after it, and the render order is tools, then system, then
    // messages. Carrying `now = <unix seconds>` here changed the front of the
    // prefix on every single request, so no provider could reuse any of the
    // prompt behind it. That is a cause of poor cache reuse that is entirely
    // ours, independent of what the transport reports. The flat zero cache
    // reads recorded across August 2026 turned out to be a measurement
    // artifact -- embacle parsed past the counts until 0.22.0 -- so they are
    // not evidence about this block either way; the churn was real and this
    // quantization is the fix for it.
    //
    // Floored, never rounded up: rounding up crosses midnight and would move
    // "today" a day early at 23:58. Floor costs an upper bound up to one
    // quantum stale, which is immaterial here because the guidance below tells
    // the model to pass NO bounds for a freshness fetch — `now` is only ever a
    // `before` for a window question (today / this week / this month).
    let quantized_epoch = (now_utc.timestamp() / NOW_QUANTUM_SECS) * NOW_QUANTUM_SECS;
    let datetime_str = Utc.timestamp_opt(quantized_epoch, 0).single().map_or_else(
        || local.format("%Y-%m-%d %H:%M").to_string(),
        |q| q.with_timezone(&tz).format("%Y-%m-%d %H:%M").to_string(),
    );
    let now_epoch = quantized_epoch;

    // Local-midnight epoch `days_back` days before today, DST-safe (a spring
    // gap/fold at midnight is vanishingly rare; take the earliest valid
    // instant). Falls back to `now` if construction ever fails so a boundary
    // is never silently wrong by years.
    let midnight_epoch = |days_back: i64| -> i64 {
        (local - Duration::days(days_back))
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .and_then(|naive| tz.from_local_datetime(&naive).earliest())
            .map_or(now_epoch, |dt| dt.timestamp())
    };
    let today0 = midnight_epoch(0);
    let yesterday0 = midnight_epoch(1);
    // Monday-anchored week (the convention pierre_system.md already states).
    let days_since_monday = i64::from(local.weekday().num_days_from_monday());
    let week0 = midnight_epoch(days_since_monday);
    // First of the current month, 00:00 local — the "ce mois" / "this month"
    // window, which is otherwise a from-scratch date→epoch conversion.
    let month0 = local
        .date_naive()
        .with_day(1)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .and_then(|naive| tz.from_local_datetime(&naive).earliest())
        .map_or(now_epoch, |dt| dt.timestamp());

    // A full epoch reference table so the model COPIES every common boundary
    // instead of converting an absolute date to an epoch — the arithmetic LLMs
    // get wrong. 2026-07-24: a coach reading the human date still passed
    // `before=1753362000` (2025-07-24, a year early) and served year-old data;
    // the same failure lurks on any window bound (`after=<Monday 00:00>` etc.),
    // so every boundary is precomputed here. Local to the user's tz, not the
    // server's UTC clock. The guidance is inline so it holds regardless of the
    // surrounding prompt template.
    format!(
        "{datetime_str} ({label}).\n\
         Unix epochs (seconds) — COPY these; never convert a date to an epoch yourself (it is error-prone):\n\
         - now = {now_epoch}\n\
         - start of today (00:00 local) = {today0}\n\
         - start of yesterday (00:00 local) = {yesterday0}\n\
         - start of this week (Monday 00:00 local) = {week0}\n\
         - start of this month (1st, 00:00 local) = {month0}\n\
         For a freshness/recent fetch pass no `after`/`before` (the server returns your newest \
         activities first). For a window question (today / hier / cette semaine / ce mois) pass \
         `after` = the matching start above and `before` = now (count only the rows whose start \
         falls inside the window)."
    )
}

/// Look up the user's selected coaching persona, falling back to the
/// default ([`CoachingPersona::Casual`]) when the user row cannot be
/// resolved.
///
/// We never block prompt assembly on the user lookup: an unknown user,
/// a malformed `user_id`, or a transient repository error all collapse to
/// the default persona so chat continues to flow. Persona is a UX
/// preference, not a security boundary.
pub(crate) async fn resolve_user_persona(
    users: &dyn UserRepository,
    user_id: &str,
) -> CoachingPersona {
    resolve_user_persona_and_timezone(users, user_id).await.0
}

/// Look up the user's coaching persona and timezone in a single read.
///
/// Reading both in one query keeps the caller-side cost the same as the
/// persona-only version. Timezone is an IANA name (`"America/Toronto"`)
/// or `None` when the client has never reported one — readers fall back
/// to UTC.
pub(crate) async fn resolve_user_persona_and_timezone(
    users: &dyn UserRepository,
    user_id: &str,
) -> (CoachingPersona, Option<String>) {
    let Some(user_uuid) = parse_uuid(user_id).ok() else {
        return (CoachingPersona::default(), None);
    };
    match users.get_global(user_uuid).await {
        Ok(Some(user)) => (user.coaching_persona, user.timezone),
        Ok(None) | Err(_) => (CoachingPersona::default(), None),
    }
}

/// Resolve a coach's system prompt for the current turn.
///
/// For `source == "contremaitre"` rows we consult
/// [`PromptRegistry::get_coach_prompt`] first so the next chat turn picks
/// up a webhook-driven hot-reload without waiting for the seed-coaches
/// job to rewrite the `coaches.system_prompt` column. A registry miss
/// falls back to the DB column and logs a `warn!` — for a contremaitre
/// coach that miss means the registry never loaded the entry (cold start
/// before the first sync, or hot-reload is broken).
///
/// Any other source (`"custom"`, `"seed"`) reads the DB column
/// directly — those coaches are not git-managed and have no upstream
/// source of truth.
///
/// `locale` is the turn's resolved locale, taken from
/// [`crate::SurfaceProfile::locale`]; the registry keys coach prompts by
/// `(slug, locale)` per contremaitre manifest v5.
pub fn resolve_coach_base_prompt(
    prompt_registry: &Arc<PromptRegistry>,
    coach_ctx: &CoachRuntimeContext,
    locale: &str,
) -> String {
    if coach_ctx.source != "contremaitre" {
        return coach_ctx.system_prompt.clone();
    }

    if let Some(content) = prompt_registry.get_coach_prompt(&coach_ctx.slug, locale) {
        return content;
    }

    warn!(
        slug = %coach_ctx.slug,
        locale = %locale,
        "contremaitre coach prompt missing from PromptRegistry — falling back to coaches.system_prompt column. Hot-reload may be broken or the registry has not been populated for this locale.",
    );
    coach_ctx.system_prompt.clone()
}

/// Output of [`assemble_prompt_and_messages`]: the hardened prompt guard,
/// pending-followup ids, the flattened LLM message list, the parallel
/// `source_ids` vector (`None` for the system prompt, `Some(history-row id)`
/// per surviving message) that Tier 1 compaction uses to anchor block ids,
/// and the group roster snapshots (empty outside a group conversation) that
/// peer grounding and the peer-claim verifier match names against.
pub(crate) type AssembledPrompt = (
    prompt_leak::PromptGuard,
    Vec<String>,
    Vec<ChatMessage>,
    Vec<Option<String>>,
    Vec<MemberFitnessSnapshot>,
);

/// Assemble the hardened system prompt and flatten history into an
/// LLM-ready message list.
///
/// Owns pipeline stages 7a through 8: coach/default prompt,
/// connected-provider context, group context, freshness hint, memory
/// recall, pending followups, channel-specific response constraints,
/// and canary hardening — followed by
/// [`build_llm_messages`](super::prompt_builder::build_llm_messages)
/// flattening.
///
/// Returns the prompt guard, pending-followup ids, the flattened message
/// list, and the parallel `source_ids` vector mapping each message to its
/// history-row id (`None` for the system prompt) for Tier 1 compaction.
///
/// # Errors
///
/// Returns [`pierre_core::errors::AppError`] from the group context resolver
/// when the conversation is group-scoped and repository lookups
/// surface errors. All other assembly steps are infallible or
/// log-and-continue.
#[tracing::instrument(
    skip_all,
    fields(
        turn_id = %input.turn_id,
        channel = profile.surface.as_str(),
        coach_id = input.turn_coach_id(conv).unwrap_or("none"),
        history_len = history.len(),
        prompt_len = field::Empty,
        msg_count = field::Empty,
    )
)]
pub(crate) async fn assemble_prompt_and_messages(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    profile: &SurfaceProfile,
    conv: &ConversationRecord,
    coach_ctx: Option<&CoachRuntimeContext>,
    history: &[MessageRecord],
    onboarding: Option<&super::onboarding::OnboardingTurn>,
) -> AppResult<AssembledPrompt> {
    // Stage 7a: Build the base prompt in two layers — platform contract, then
    // persona.
    // For contremaitre-sourced coaches we consult the in-memory
    // `PromptRegistry` first so a webhook-driven hot-reload reaches the
    // next chat turn without a seeder re-run. Other sources (`"custom"`,
    // `"seed"`) read the DB `system_prompt` column as before.
    // The persona layer: a bound coach's voice, or the default Dravr voice.
    // Voice only — every platform invariant lives in the contract below, which
    // is why replacing this block is safe.
    let persona_prompt = coach_ctx.map_or_else(
        || ctx.pierre_system_prompt.clone(),
        |c| resolve_coach_base_prompt(&ctx.prompt_registry, c, &profile.locale),
    );

    // The platform contract leads, unconditionally.
    //
    // Binding a coach REPLACES the persona block, and before the contract was
    // split out of `pierre_system.md` that replacement took the platform rules
    // with it: all 52 coach personas ran with no current date (so "hier" could
    // not be resolved to an epoch window), no Available-Tools framing, and no
    // "never refuse a question `get_activities` can answer" rule. The coach
    // then declined data questions it had every means to answer — the
    // 2026-07-24 and 2026-08-11 live incidents. Contract first, persona second,
    // so a persona can shape the voice and never the capability.
    let base_prompt = format!(
        "{}\n\n{persona_prompt}",
        ctx.prompt_registry.platform_contract_prompt()
    );

    // Stage 7a.1: Resolve `{{SCOPE_REFUSAL}}` / `{{CAPABILITY_REFUSAL}}` /
    // `{{COACH_SCOPE_CARVE_OUT}}` / `{{COACHING_PERSONA_RULES}}`
    // placeholders to runtime values. Refusals and the carve-out come
    // from the per-locale messaging registry; the persona block comes
    // from the user's `coaching_persona` column and controls output
    // format (structure, citation density, length) orthogonally to the
    // coach personality.
    let (persona, user_timezone) =
        resolve_user_persona_and_timezone(ctx.repos.users.as_ref(), &input.user_id).await;
    let base_prompt = interpolate_prompt_placeholders(
        &ctx.messaging_strings_registry,
        &ctx.prompt_registry,
        &profile.locale,
        coach_ctx,
        persona,
        user_timezone.as_deref(),
        &base_prompt,
    );

    // Stage 7a.2: State that the tool set is closed. The tools themselves are
    // advertised once, by the provider layer — embacle renders the full schemas
    // for text tool-calling, and native providers receive them over the API.
    //
    // This stage used to generate a second list here: one prose line per tool,
    // 11,763 characters, restating names the model was already given. It was
    // built from `user_visible_schemas()` while the declarations were built
    // from `chat_callable_schemas()`, so it advertised every non-chat-callable
    // category — coach CRUD, config writes, claim verification — as though the
    // coach could call them. Two lists from two sources cannot help drifting;
    // deleting one is what stops it, not regenerating both.
    //
    // What the list was carrying that nothing else says is the boundary itself,
    // so that stays.
    //
    // The names — and only the names — come back with it, from
    // `chat_callable_schemas()`. That is the source the declarations are built
    // from, which is the half the deleted list got wrong: it read
    // `user_visible_schemas()` and advertised tools the coach could not call.
    // One source cannot drift against itself, and names-only is ~2 KB against
    // the 11,763 characters that were removed.
    //
    // It is load-bearing on the `mcp_tool_calling` path, where the catalogue
    // goes to Copilot over MCP and never reaches the prompt — the coach started
    // those turns unable to enumerate a single tool, and answered capability
    // questions from a prompt that named none. On the native path the full
    // schemas already ship every turn (registre#103 covers that cost), so the
    // index is a cheap, byte-stable restatement there rather than the only copy.
    let tool_names: Vec<String> = ctx
        .tool_registry
        .chat_callable_schemas()
        .into_iter()
        .map(|s| s.name)
        .collect();
    let base_prompt = format!(
        "{base_prompt}\n\n{TOOL_BOUNDARY}{}",
        render_tool_index(&tool_names)
    );

    // Stage 7b: Append connected-provider context so the LLM never asks the
    // user to connect providers that are already connected.
    let user_uuid = parse_uuid(&input.user_id).unwrap_or_default();
    let provider_context = build_provider_context(&ctx.data, user_uuid).await;
    let base_prompt = if provider_context.is_empty() {
        base_prompt
    } else {
        format!("{base_prompt}{provider_context}")
    };

    // Stage 7c: Inject group coaching context — only when the conversation is
    // explicitly group-scoped. Personal 1:1 chats never inherit group context
    // from the user's membership.
    #[cfg(feature = "tools-groups")]
    let (base_prompt, group_roster) = {
        let (resolved_group_id, snapshots) =
            resolve_group_context(ctx, conv.group_id.as_deref(), input.tool_tenant_id).await?;
        let injected = ctx
            .group_service
            .inject_group_context(
                &base_prompt,
                "",
                user_uuid,
                input.tool_tenant_id,
                resolved_group_id.as_deref(),
                &snapshots,
            )
            .await
            .unwrap_or(base_prompt);
        (injected, snapshots)
    };
    #[cfg(not(feature = "tools-groups"))]
    let group_roster: Vec<MemberFitnessSnapshot> = Vec::new();

    // Stage 7d: Trigger background provider refresh and append freshness hint.
    // Unconditional: how stale the athlete's provider data is has the same
    // bearing on the answer everywhere, so there was never a surface that
    // wanted it suppressed.
    let auth_repos = ctx.repos.auth_repos();
    let base_prompt = inject_refresh_context(
        super::refresh::RefreshDeps {
            auth_repos: &auth_repos,
            activity_cache: ctx.repos.activity_cache.clone(),
            #[cfg(feature = "health-sync")]
            sync_orchestrator: &ctx.sync_orchestrator,
            #[cfg(feature = "health-sync")]
            sse_manager: &ctx.sse_manager,
        },
        &input.user_id,
        input.tool_tenant_id,
        base_prompt,
    )
    .await;

    // Stage 7e: Render the per-user OKF context bundle (North Star + pillar +
    // medical facts) from the read-time Dossier into the prompt. Single
    // fact->prompt surface; user-wide (coach-agnostic) facts.
    let base_prompt = inject_okf_bundle(
        ctx.repos.dossier.as_ref(),
        input.conversation_tenant_id,
        user_uuid,
        base_prompt,
    )
    .await;

    // Every per-coach block below follows the coach answering this turn — the
    // mentioned coach on a `@handle` turn — so a routed turn carries that
    // coach's playbooks, followups and plan, not another coach's under its
    // persona.
    let turn_coach_id = input.turn_coach_id(conv);

    // Stage 7e.2: Inject the athlete's proven coaching playbooks (learned from
    // their own outcomes) so the coach prefers what has worked for them. Scoped
    // to the TOOL tenant — where the activity data and playbooks live.
    let playbook_tenant = input.tool_tenant_id.to_string();
    let base_prompt = inject_playbooks(
        ctx.repos.playbooks.as_ref(),
        ctx.repos.activity_cache.as_ref(),
        &playbook_tenant,
        &input.user_id,
        turn_coach_id,
        base_prompt,
    )
    .await;

    // Stage 7f: Render pending coach followups. Surfaced IDs are marked
    // delivered after the turn succeeds.
    let (base_prompt, pending_followup_ids) = inject_pending_followups(
        &ctx.data,
        input.conversation_tenant_id,
        &input.user_id,
        turn_coach_id,
        base_prompt,
    )
    .await;

    // Stage 7f.1: Render the athlete's own open commitments. Scoped to the
    // TOOL tenant — the tenant `commitment_create` writes under and the one
    // their activity data lives in, so the block and the sweep agree on which
    // promises exist. Deliberately above the training plan: a promise the
    // athlete made themselves outranks a plan the coach wrote for them.
    let base_prompt = inject_commitments(
        &ctx.data,
        &input.tool_tenant_id.to_string(),
        &input.user_id,
        user_timezone.as_deref(),
        base_prompt,
    )
    .await;

    // Stage 7f.2: Render the athlete's persisted training plan (outline +
    // current/next week). Scoped to the TOOL tenant — the tenant
    // `save_training_plan` writes under. `today` is the athlete's civil date
    // so week selection and the race countdown match their calendar, not the
    // server's UTC clock.
    //
    // Suppressed while the guided pillar walk is active (see
    // `inject_training_plan` for the rationale and live incident).
    let athlete_today = user_timezone
        .as_deref()
        .and_then(|s| s.parse::<chrono_tz::Tz>().ok())
        .map_or_else(
            || chrono::Utc::now().date_naive(),
            |tz| chrono::Utc::now().with_timezone(&tz).date_naive(),
        );
    let base_prompt = inject_training_plan(
        ctx.repos.training_plans.as_ref(),
        &input.tool_tenant_id.to_string(),
        &input.user_id,
        turn_coach_id,
        athlete_today,
        onboarding.is_some(),
        base_prompt,
    )
    .await;

    // Stage 7f.3: Append the load-progression guardrails for coaches whose
    // category can actually prescribe load.
    //
    // Deliberately ABOVE the tool-discipline block (7g.1), the structured-output
    // contract (7g.2) and the guided-flow directive (7g.3), giving up recency on
    // purpose. Four of the twelve training coaches declare an `output_schema`
    // and receive a JSON-only contract at 7g.2; ~450 tokens of prose landing
    // after that contract is the same recency contest that turned the
    // 2026-07-24 walk into an unwanted 16-week plan. These guardrails are
    // content guidance, not an output-format rule, so they do not need to win
    // that contest — and the real enforcement is the save-time ramp check in
    // `save_training_plan`, which measures the plan rather than asking for it.
    //
    // Suppressed while a guided flow owns the turn, for the same reason 7f.2
    // and 7g.2 are: a calibration interview is asking questions, not
    // prescribing load, so the block would be pure prompt cost on every turn of
    // the interview.
    let base_prompt = match progression_guardrails(ctx, coach_ctx, onboarding.is_some()) {
        Some(guardrails) => format!("{base_prompt}\n\n{guardrails}"),
        None => base_prompt,
    };

    // Stage 7f.4: Append the group ambient transcript supplied by the
    // messaging ingress. Group chat history is per member, so without this
    // block the coach never sees what OTHER members said — the transcript
    // is the only cross-member view of the room's discussion.
    let base_prompt = match input.ambient_context.as_deref() {
        Some(ambient) => format!("{base_prompt}\n\n{ambient}"),
        None => base_prompt,
    };

    // Stage 7g: Append the surface's live prose contract.
    let raw_system_prompt = match profile.prose_contract.as_deref() {
        Some(suffix) => format!("{base_prompt}\n\n{suffix}"),
        None => base_prompt,
    };

    // Stage 7g.1: Append mandatory tool-discipline rules at the very end of
    // the system prompt, immediately before the user turn. LLMs
    // (claude-opus-4.7 especially) recency-bias heavily — mid-prompt rules
    // get drowned out by 20 KB of coach persona + provider context.
    // Keeping this block last ensures the tool-call and "no narration"
    // constraints are the freshest instructions when the model starts
    // generating.
    //
    // A surface that does not parse markdown gets a stricter prose variant
    // that constrains the natural-language portion of the reply (no markdown
    // headings, no bullet lists, no inline JSON) but still teaches the model
    // the `<tool_call>` invocation syntax: tool-call blocks are required to
    // actually execute tools and are stripped server-side via
    // `tool_simulation::strip_simulation_artifacts` before the reply reaches
    // the user, so they never violate the plain-text mandate from
    // `messaging_context.md`.
    let tool_discipline_prompt = if matches!(profile.render.prose, ProseFormat::PlainText) {
        ctx.tool_discipline_messaging_prompt.clone()
    } else {
        ctx.tool_discipline_prompt.clone()
    };

    // Both variants already carry the shared rules, appended when the context
    // resolved them; this is the surface-specific half plus that tail.
    let raw_system_prompt = format!("{raw_system_prompt}\n\n{tool_discipline_prompt}");

    // Stage 7g.2: Builder coaches that declare an `output_schema` get the
    // structured-output contract appended last (recency priority alongside
    // tool-discipline): emit JSON-only for a plan, prose for a refusal, never
    // narrate the data-gathering process. Only where a plan card can actually
    // be laid out — without one the JSON directive (and the matching
    // extraction) is skipped and the coach falls back to the surface's
    // plain-prose mandate.
    //
    // Suppressed while the guided pillar walk is active, for the same reason
    // Stage 7f.2 suppresses the plan block: a JSON-plan output contract has no
    // business in a profile-building conversation, and it directly contradicts
    // the onboarding directive appended below.
    // Bound once and read twice: this same predicate decides whether Stage 7g.3
    // may append [`TURN_DIRECTIVE`] below. Two copies of it would be free to
    // drift, and the drift would be silent — a builder coach would carry both a
    // JSON-only contract and a prose task directive, with the prose block
    // winning on recency. That is the 2026-07-24 derail exactly.
    let structured_contract_active = coach_ctx.is_some_and(|c| c.output_schema.is_some())
        && profile.render.blocks.workout_plan_card
        && onboarding.is_none();
    let raw_system_prompt = if structured_contract_active {
        format!("{raw_system_prompt}\n\n{}", ctx.structured_output_prompt)
    } else {
        raw_system_prompt
    };

    // Stage 7g.2b: Inline visual contract. Granted per coach via `visuals:` and
    // withheld everywhere else, so a coach that was never granted one is never
    // even told the syntax — the post-process gate refuses its fences anyway,
    // but not telling it is what keeps the two consistent.
    //
    // Withheld on a surface that can neither draw a Scene inline nor fetch a
    // rasterised one, because a coach that emitted a fence there would have it
    // stripped to a marker pointing at nothing. Every surface shipping today
    // does one or the other — the app draws inline, the channels fetch pixels
    // the egress presses — so the gate is what keeps a future text-only
    // transport from being taught a syntax it cannot honour.
    //
    // Mutually exclusive with the structured-output contract above by
    // construction: that contract demands the ENTIRE reply be one JSON object,
    // which leaves no prose for a block to be embedded in. Handing a coach both
    // would be handing it two contradictory output shapes and letting recency
    // pick — the 2026-07-24 derail's exact mechanism.
    //
    // The grant is read through `granted_visuals`, so "no coach bound" means the
    // platform baseline rather than "no visuals". A group chat binds no coach —
    // the platform is answering — and treating that as an empty grant withheld
    // the contract entirely, leaving the model to report it had no way to draw.
    let visual_contract_active = !structured_contract_active
        && onboarding.is_none()
        && (profile.render.blocks.scene_inline || profile.render.blocks.scene_raster)
        && !viz_blocks::granted_visuals(coach_ctx.map(|c| c.visuals.as_slice())).is_empty();
    let raw_system_prompt = if visual_contract_active {
        format!("{raw_system_prompt}\n\n{}", ctx.visual_blocks_prompt)
    } else {
        raw_system_prompt
    };

    // Stage 7g.3: Onboarding directive — when this conversation is mid guided
    // pillar walk, steer the coach to probe the current topic conversationally.
    // On the first turn after that walk ends, the same slot carries the
    // directive that revokes it: the interview block claims to override every
    // other instruction and forbids saving a plan, and dropping it does not
    // retract it from the transcript it already shaped (2026-07-28 — a coach
    // reported a save failure 48 seconds after a completed calibration, having
    // never called the tool). The two are mutually exclusive by construction —
    // a conversation is either in a flow or just out of one — so they share one
    // rebinding, which is also what keeps the tail invariant below intact.
    //
    // Deliberately below every behavioural block; only the Stage 7g.4 identity
    // anchor follows it, and that block states who the assistant IS rather than
    // what to do this turn, so it does not compete for behavioural precedence.
    // It used to sit mid-prompt
    // (7e.1) where the channel response constraints, the tool-discipline block
    // and the structured-output contract all landed after it; a builder coach
    // whose persona mandates a plan on its first reply won that recency
    // contest, which is how the 2026-07-24 walk derailed into a 16-week plan on
    // the athlete's first answer.
    // The slot is never empty. Whichever arm wins, the turn ships with a
    // concrete task — that is the property that holds the persona, and leaving
    // the ordinary-turn arm empty is what let the 2026-08-05 Telegram break
    // through (see [`TURN_DIRECTIVE`] for the measurement). The release arm
    // already qualifies: "call the tool and report what it actually returned"
    // is as turn-scoped as an interview probe.
    let raw_system_prompt = match onboarding {
        Some(turn) => format!("{raw_system_prompt}{}", super::onboarding::directive(turn)),
        None if OnboardingState::just_completed(conv.onboarding_state.as_deref(), Utc::now()) => {
            format!(
                "{raw_system_prompt}{}",
                super::onboarding::release_directive()
            )
        }
        // A builder coach on a card channel already received the JSON output
        // contract at Stage 7g.2, which IS this turn's task. Appending prose
        // below it would win the recency contest against the contract.
        None if structured_contract_active => raw_system_prompt,
        None => format!("{raw_system_prompt}{TURN_DIRECTIVE}"),
    };

    // Stage 7g.3b: State the language this turn is conducted in.
    //
    // `profile.locale` is already the authority for everything else the turn
    // renders — which coach prompt is loaded, which refusal strings interpolate
    // above, which acronym glosses expand, which disclaimer the text guardrails
    // prepend, which language the verification banner speaks. Until now it was
    // the authority for every one of those EXCEPT the coaching text they wrap,
    // which the model had to infer from the athlete's own words.
    //
    // It loses that inference. On 2026-08-30 a francophone athlete asked a
    // long question in French on Telegram and got an English answer, then
    // answered "Yes" and got French: the athlete's few hundred French
    // characters were outweighed by tens of KB of English contract, coach
    // scaffolding, provider context and tool results, and the short second turn
    // was not. The locale was `fr` throughout — provably, because the French
    // medical disclaimer fired on the second turn while the first, the one
    // reporting urinary symptoms, matched no French trigger against its English
    // text and shipped with no disclaimer at all.
    //
    // This block is now the ONLY language rule in the stack. The one it
    // replaced was prose in contremaitre's `messaging_context.md` ("reply in
    // the same language the person used") — it asked for the same inference,
    // and arriving as `SurfaceProfile::prose_contract` it reached messaging
    // surfaces only, so the app's chat carried no language rule at all. It was
    // deleted once this shipped (carnet#163); every property it stated,
    // including "do not switch languages mid-reply", is stated here in all
    // five locales. The gender-neutrality clause that shared its `Language:`
    // heading stays in contremaitre — different subject, still inference.
    //
    // Appended unconditionally, including under the structured-output contract:
    // this block says which language to write in, never what to produce, so it
    // does not enter the task-directive recency contest Stage 7g.3 guards. Each
    // locale's text is authored in its own language, which makes the
    // instruction an instance of what it asks for.
    let raw_system_prompt = format!(
        "{raw_system_prompt}\n\n{}",
        ctx.messaging_strings_registry
            .get(KEY_TURN_LANGUAGE, &profile.locale)
    );

    // Stage 7g.4: Close every prompt with the identity anchor.
    //
    // The coach LLM runs through the GitHub Copilot CLI, whose own system
    // prompt ("You are the GitHub Copilot CLI, a terminal assistant built by
    // GitHub", plus an explicit "when asked which model you are, reply 'I'm
    // powered by <name>'" clause) occupies the true system slot: Copilot's ACP
    // `session/new` schema accepts only `_meta`/`additionalDirectories`/`cwd`/
    // `mcpServers`, so the `systemPrompt` embacle sends is silently dropped and
    // the whole platform prompt arrives as user-authority text. That gap is
    // structural, but user-authority text still moves this behavior class
    // (embacle v0.19.6's tool-catalog reframing did, over 9 live A/B runs) — so
    // the fight is "adversarial slot vs. something" rather than "vs. nothing".
    //
    // Until now coach-bound turns fought it with nothing: [`resolve_coach_base_prompt`]
    // REPLACES `pierre_system.md` (the only prompt carrying "You are Dravr")
    // with the coach's own prompt, which never states who the assistant is.
    // The result was an accidental prod A/B — coach-bound turns matched the
    // boundary detector ~24% of the time while no-coach turns (which keep the
    // anchor) matched 0%.
    //
    // LAST, not first: see [`IDENTITY_ANCHOR`] for the 48-run A/B that measured
    // both. Prepending left the model still reciting Copilot's model-disclosure
    // clause; the tail placement is what suppressed it, for the same recency
    // reason Stage 7g.1 gives. Detection at the response boundary
    // ([`prompt_leak`]) stays as the safety net.
    //
    // Caveat worth knowing when reading prod telemetry: embacle appends its
    // ~16 KB text tool catalog to the system message AFTER this block
    // (`tool_simulation::inject_tool_catalog`), so "last" here means last
    // platform-controlled block, not last on the wire.
    let raw_system_prompt =
        close_with_anchors(&raw_system_prompt, coach_ctx.map(|c| c.slug.as_str()));

    // Stage 7h: Harden the prompt with a per-turn canary. Salted with the coach
    // answering this turn, which is also what the reply scan reads.
    let prompt_guard = prompt_leak::harden_system_prompt(
        input.conversation_tenant_id,
        turn_coach_id,
        &raw_system_prompt,
    );

    // Stage 8: Flatten into the LLM message list. `source_ids` maps each
    // emitted message back to its history row id (None for the system prompt)
    // so Tier 1 compaction can anchor block ids without a positional guess.
    //
    // First fetch any persisted compaction blocks for this conversation so the
    // flattener can splice their summaries over the raw history rows they
    // replaced — without this read-side reconstruction the summaries are
    // write-only orphans and a long thread silently loses its compacted
    // context. A repository error degrades to an empty block list (the turn
    // proceeds with raw history) rather than failing the turn.
    let blocks = match ctx
        .repos
        .memory
        .list_compaction_blocks(&input.conversation_id, input.conversation_tenant_id)
        .await
    {
        Ok(blocks) => blocks,
        Err(e) => {
            warn!(error = %e, "failed to list compaction blocks; proceeding with raw history");
            Vec::new()
        }
    };
    let (llm_messages, source_ids) =
        build_llm_messages_with_blocks(Some(&prompt_guard.hardened_prompt), history, &blocks);

    let span = Span::current();
    span.record("prompt_len", prompt_guard.hardened_prompt.len());
    span.record("msg_count", llm_messages.len());

    info!(
        prompt_len = prompt_guard.hardened_prompt.len(),
        msg_count = llm_messages.len(),
        "prompt assembled"
    );

    // Trace-level dump of the full hardened system prompt and the flattened
    // message list. Operators bump `RUST_LOG=...=trace` to see exactly what
    // the LLM will receive; otherwise the events are filtered out.
    if tracing::enabled!(tracing::Level::TRACE) {
        trace!(
            prompt = %prompt_guard.hardened_prompt,
            "prompt assembled: hardened system prompt"
        );
        match serde_json::to_string(&llm_messages) {
            Ok(messages_json) => trace!(
                messages = %messages_json,
                "prompt assembled: flattened llm messages"
            ),
            Err(e) => trace!(error = %e, "failed to serialize llm messages for trace"),
        }
    }

    Ok((
        prompt_guard,
        pending_followup_ids,
        llm_messages,
        source_ids,
        group_roster,
    ))
}

/// Resolve the coach runtime context this turn answers as.
///
/// A mentioned coach arrives with its context already resolved — in the
/// athlete's own tenant, where the install lives, so a `@handle` turn on a
/// shared messaging room reads the athlete's copy rather than looking for it
/// under the bot's tenant. Otherwise the conversation's bound coach resolves
/// under the conversation's tenant, as every turn did before mentions existed.
pub(crate) async fn resolve_turn_coach_ctx(
    ctx: &ChatPipelineContext,
    input: &TurnInput,
    conv: &ConversationRecord,
) -> AppResult<Option<CoachRuntimeContext>> {
    if let Some(mention) = &input.mentioned_coach {
        return Ok(Some(mention.runtime.clone()));
    }
    match conv.coach_id.as_deref() {
        Some(coach_id) => {
            ctx.repos
                .coaches
                .get_coach_runtime_context(coach_id, input.conversation_tenant_id)
                .await
        }
        None => Ok(None),
    }
}
