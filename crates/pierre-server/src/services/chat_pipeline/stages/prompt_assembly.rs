// ABOUTME: Prompt assembly stage (stages 7a-8) — coach prompt → provider/group/memory → canary → messages
// ABOUTME: Composes the lower-level stages (prompt_builder, refresh, memory, followups) into one flow
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use tracing::{field, info, trace, Span};

use crate::contremaitre::messaging_strings::{
    DEFAULT_LOCALE, KEY_CAPABILITY_REFUSAL, KEY_COACH_SCOPE_CARVE_OUT_NUTRITION,
    KEY_COACH_SCOPE_CARVE_OUT_RECIPES, KEY_SCOPE_REFUSAL,
};
use crate::errors::AppResult;
use crate::llm::ChatMessage;
use crate::mcp::resources::ServerContext;
use crate::services::prompt_leak;
use pierre_core::models::coaches::CoachCategory;
use pierre_core::models::{CoachRuntimeContext, CoachingPersona};
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::database::repositories::UserRepository;
use pierre_database::database::{ConversationRecord, MessageRecord};
use pierre_llm::prompts::unsubstituted_placeholders;
use tracing::error;

use super::super::channel_profile::ChannelProfile;
use super::super::turn::TurnInput;
use super::followups::inject_pending_followups;
use super::memory::inject_memory_recall;
#[cfg(feature = "tools-groups")]
use super::prompt_builder::resolve_group_context;
use super::prompt_builder::{build_llm_messages, build_provider_context, build_tools_section};
use super::refresh::inject_refresh_context;

/// Return the [`messaging_strings_registry`](ServerContext::messaging_strings_registry)
/// key that holds the scope carve-out for a given coach category, or
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

/// Rewrite the prompt-template placeholders with their runtime values.
///
/// Handles four placeholders:
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
///
/// Returns the prompt unchanged when none of the placeholders are
/// present.
fn interpolate_prompt_placeholders(
    resources: &Arc<ServerContext>,
    input: &TurnInput,
    coach_ctx: Option<&CoachRuntimeContext>,
    persona: CoachingPersona,
    prompt: &str,
) -> String {
    let has_scope = prompt.contains("{{SCOPE_REFUSAL}}");
    let has_capability = prompt.contains("{{CAPABILITY_REFUSAL}}");
    let has_carve_out = prompt.contains("{{COACH_SCOPE_CARVE_OUT}}");
    let has_persona = prompt.contains("{{COACHING_PERSONA_RULES}}");
    let assembled = if !has_scope && !has_capability && !has_carve_out && !has_persona {
        prompt.to_owned()
    } else {
        let locale = input.locale.as_deref().unwrap_or(DEFAULT_LOCALE);
        let registry = &resources.messaging_strings_registry;
        let scope = registry.get(KEY_SCOPE_REFUSAL, locale);
        let capability = registry.get(KEY_CAPABILITY_REFUSAL, locale);
        let carve_out = coach_ctx
            .and_then(|c| coach_scope_carve_out_key(c.category))
            .map_or_else(String::new, |key| registry.get(key, locale));
        // Read the persona block from the prompt registry so a hot-reload
        // from contremaitre takes effect on the very next turn — no
        // redeploy needed. The registry seeds itself with the compiled-in
        // `include_str!()` content at startup, so chat works before the
        // first sync completes; once contremaitre lands a newer version
        // via webhook → selective_sync, the same lookup here picks it up.
        let persona_block = resources.prompt_registry.coaching_persona_prompt(persona);
        prompt
            .replace("{{SCOPE_REFUSAL}}", &scope)
            .replace("{{CAPABILITY_REFUSAL}}", &capability)
            .replace("{{COACH_SCOPE_CARVE_OUT}}", &carve_out)
            .replace("{{COACHING_PERSONA_RULES}}", &persona_block)
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

/// Look up the user's selected coaching persona, falling back to the
/// default ([`CoachingPersona::Casual`]) when the user row cannot be
/// resolved.
///
/// We never block prompt assembly on the user lookup: an unknown user,
/// a malformed `user_id`, or a transient repository error all collapse to
/// the default persona so chat continues to flow. Persona is a UX
/// preference, not a security boundary.
pub(in crate::services::chat_pipeline) async fn resolve_user_persona(
    resources: &Arc<ServerContext>,
    user_id: &str,
) -> CoachingPersona {
    let Some(user_uuid) = parse_uuid(user_id).ok() else {
        return CoachingPersona::default();
    };
    let users: &dyn UserRepository = resources.repos.users.as_ref();
    match users.get_global(user_uuid).await {
        Ok(Some(user)) => user.coaching_persona,
        Ok(None) | Err(_) => CoachingPersona::default(),
    }
}

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
/// # Errors
///
/// Returns [`crate::errors::AppError`] from the group context resolver
/// when the conversation is group-scoped and repository lookups
/// surface errors. All other assembly steps are infallible or
/// log-and-continue.
#[tracing::instrument(
    skip_all,
    fields(
        turn_id = %input.turn_id,
        channel = profile.channel.as_str(),
        coach_id = conv.coach_id.as_deref().unwrap_or("none"),
        history_len = history.len(),
        prompt_len = field::Empty,
        msg_count = field::Empty,
    )
)]
pub(in crate::services::chat_pipeline) async fn assemble_prompt_and_messages(
    resources: &Arc<ServerContext>,
    input: &TurnInput,
    profile: &ChannelProfile,
    conv: &ConversationRecord,
    coach_ctx: Option<&CoachRuntimeContext>,
    history: &[MessageRecord],
) -> AppResult<(prompt_leak::PromptGuard, Vec<String>, Vec<ChatMessage>)> {
    // Stage 7a: Start from coach-defined or default Pierre system prompt.
    let base_prompt = coach_ctx.map_or_else(
        || resources.pierre_system_prompt(),
        |c| c.system_prompt.clone(),
    );

    // Stage 7a.1: Resolve `{{SCOPE_REFUSAL}}` / `{{CAPABILITY_REFUSAL}}` /
    // `{{COACH_SCOPE_CARVE_OUT}}` / `{{COACHING_PERSONA_RULES}}`
    // placeholders to runtime values. Refusals and the carve-out come
    // from the per-locale messaging registry; the persona block comes
    // from the user's `coaching_persona` column and controls output
    // format (structure, citation density, length) orthogonally to the
    // coach personality.
    let persona = resolve_user_persona(resources, &input.user_id).await;
    let base_prompt =
        interpolate_prompt_placeholders(resources, input, coach_ctx, persona, &base_prompt);

    // Stage 7a.2: Append the runtime-generated "Available Tools" section.
    // Both the default Pierre system prompt and every coach's custom
    // system_prompt flow through this stage so neither can drift from
    // the actual tool registry. The registry is the single source of
    // truth; if a tool is added, renamed, or removed, the prompt
    // immediately reflects the change without a prompt edit.
    let tools_section = build_tools_section(resources);
    let base_prompt = format!("{base_prompt}\n\n{tools_section}");

    // Stage 7b: Append connected-provider context so the LLM never asks the
    // user to connect providers that are already connected.
    let user_uuid = parse_uuid(&input.user_id).unwrap_or_default();
    let provider_context = build_provider_context(resources, user_uuid).await;
    let base_prompt = if provider_context.is_empty() {
        base_prompt
    } else {
        format!("{base_prompt}{provider_context}")
    };

    // Stage 7c: Inject group coaching context — only when the conversation is
    // explicitly group-scoped. Personal 1:1 chats never inherit group context
    // from the user's membership.
    #[cfg(feature = "tools-groups")]
    let base_prompt = {
        let group_service = resources.group_service();
        let (resolved_group_id, snapshots) =
            resolve_group_context(resources, conv.group_id.as_deref(), input.tool_tenant_id)
                .await?;
        group_service
            .inject_group_context(
                &base_prompt,
                "",
                user_uuid,
                input.tool_tenant_id,
                resolved_group_id.as_deref(),
                &snapshots,
            )
            .await
            .unwrap_or(base_prompt)
    };

    // Stage 7d: Trigger background provider refresh and append freshness hint.
    let base_prompt = if profile.emit_data_freshness_hint {
        inject_refresh_context(resources, &input.user_id, input.tool_tenant_id, base_prompt).await
    } else {
        base_prompt
    };

    // Stage 7e: Inject recalled user memory facts into the prompt.
    let base_prompt = inject_memory_recall(
        resources,
        input.conversation_tenant_id,
        &input.user_id,
        conv.coach_id.as_deref(),
        base_prompt,
    )
    .await;

    // Stage 7f: Render pending coach followups. Surfaced IDs are marked
    // delivered after the turn succeeds.
    let (base_prompt, pending_followup_ids) = inject_pending_followups(
        resources,
        input.conversation_tenant_id,
        &input.user_id,
        conv.coach_id.as_deref(),
        base_prompt,
    )
    .await;

    // Stage 7g: Append the channel-profile response-constraints prompt.
    let raw_system_prompt = match profile.response_constraints_prompt.as_deref() {
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
    // Messaging channels use a stricter prose variant that constrains the
    // natural-language portion of the reply (no markdown headings, no
    // bullet lists, no inline JSON) but still teaches the model the
    // `<tool_call>` invocation syntax: tool-call blocks are required to
    // actually execute tools and are stripped server-side via
    // `tool_simulation::strip_tool_call_blocks` before the reply reaches
    // the user, so they never violate the plain-text mandate from
    // `messaging_context.md`.
    let tool_discipline_prompt = if profile.channel.is_messaging() {
        resources.tool_discipline_messaging_prompt()
    } else {
        resources.tool_discipline_prompt()
    };
    let raw_system_prompt = format!("{raw_system_prompt}\n\n{tool_discipline_prompt}");

    // Stage 7h: Harden the prompt with a per-turn canary.
    let prompt_guard = prompt_leak::harden_system_prompt(
        input.conversation_tenant_id,
        conv.coach_id.as_deref(),
        &raw_system_prompt,
    );

    // Stage 8: Flatten into the LLM message list.
    let llm_messages = build_llm_messages(Some(&prompt_guard.hardened_prompt), history);

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

    Ok((prompt_guard, pending_followup_ids, llm_messages))
}
