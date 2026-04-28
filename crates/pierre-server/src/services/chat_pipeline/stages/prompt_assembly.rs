// ABOUTME: Prompt assembly stage (stages 7a-8) — coach prompt → provider/group/memory → canary → messages
// ABOUTME: Composes the lower-level stages (prompt_builder, refresh, memory, followups) into one flow
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use crate::contremaitre::messaging_strings::{
    DEFAULT_LOCALE, KEY_CAPABILITY_REFUSAL, KEY_COACH_SCOPE_CARVE_OUT_NUTRITION,
    KEY_COACH_SCOPE_CARVE_OUT_RECIPES, KEY_SCOPE_REFUSAL,
};
use crate::errors::AppResult;
use crate::llm::ChatMessage;
use crate::mcp::resources::ServerResources;
use crate::services::prompt_leak;
use pierre_core::models::coaches::CoachCategory;
use pierre_core::models::CoachRuntimeContext;
use pierre_core::uuid_utils::parse_uuid;
use pierre_database::database::{ConversationRecord, MessageRecord};

use super::super::channel_profile::ChannelProfile;
use super::super::turn::TurnInput;
use super::followups::inject_pending_followups;
use super::memory::inject_memory_recall;
#[cfg(feature = "tools-groups")]
use super::prompt_builder::resolve_group_context;
use super::prompt_builder::{build_llm_messages, build_provider_context, build_tools_section};
use super::refresh::inject_refresh_context;

/// Return the [`messaging_strings_registry`](ServerResources::messaging_strings_registry)
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
/// Handles three placeholders:
///
/// - `{{SCOPE_REFUSAL}}` → canonical off-scope refusal sentence
/// - `{{CAPABILITY_REFUSAL}}` → canonical missing-capability refusal sentence
/// - `{{COACH_SCOPE_CARVE_OUT}}` → coach-category-specific relaxation that
///   counteracts the generic scope list (e.g. Nutrition coaches bypass
///   the "food/meal finders" out-of-scope rule for meal-planning
///   questions). Empty string when the coach's category does not need a
///   carve-out or when no coach is attached to the conversation.
///
/// Returns the prompt unchanged when none of the placeholders are
/// present.
fn interpolate_prompt_placeholders(
    resources: &Arc<ServerResources>,
    input: &TurnInput,
    coach_ctx: Option<&CoachRuntimeContext>,
    prompt: &str,
) -> String {
    let has_scope = prompt.contains("{{SCOPE_REFUSAL}}");
    let has_capability = prompt.contains("{{CAPABILITY_REFUSAL}}");
    let has_carve_out = prompt.contains("{{COACH_SCOPE_CARVE_OUT}}");
    if !has_scope && !has_capability && !has_carve_out {
        return prompt.to_owned();
    }
    let locale = input.locale.as_deref().unwrap_or(DEFAULT_LOCALE);
    let registry = &resources.messaging_strings_registry;
    let scope = registry.get(KEY_SCOPE_REFUSAL, locale);
    let capability = registry.get(KEY_CAPABILITY_REFUSAL, locale);
    let carve_out = coach_ctx
        .and_then(|c| coach_scope_carve_out_key(c.category))
        .map_or_else(String::new, |key| registry.get(key, locale));
    prompt
        .replace("{{SCOPE_REFUSAL}}", &scope)
        .replace("{{CAPABILITY_REFUSAL}}", &capability)
        .replace("{{COACH_SCOPE_CARVE_OUT}}", &carve_out)
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
pub(in crate::services::chat_pipeline) async fn assemble_prompt_and_messages(
    resources: &Arc<ServerResources>,
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
    // `{{COACH_SCOPE_CARVE_OUT}}` placeholders to runtime values from the
    // messaging registry. Refusals are per-locale canonical strings;
    // the carve-out is per-(coach-category, locale) and relaxes the
    // generic scope list for categories whose core purpose would
    // otherwise be blocked (e.g. Nutrition coaches answering dinner
    // questions).
    let base_prompt = interpolate_prompt_placeholders(resources, input, coach_ctx, &base_prompt);

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
    // Messaging channels use a prose-only variant that omits the
    // `<tool_call>` markdown code-fence example, which conflicts with
    // the plain-text mandate in `messaging_context.md` and biases the
    // model toward structured output on channels where the user sees
    // only plain text.
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

    Ok((prompt_guard, pending_followup_ids, llm_messages))
}
