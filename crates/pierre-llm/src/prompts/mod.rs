// ABOUTME: System prompts for LLM interactions loaded at compile time
// ABOUTME: Provides the Pierre fitness assistant system prompt for Gemini function calling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # System Prompts
//!
//! This module provides system prompts for LLM interactions.
//! Prompts are loaded at compile time from markdown files for easy maintenance.

use pierre_core::models::CoachingPersona;

/// Pierre Fitness Intelligence Assistant system prompt
///
/// Contains instructions for the AI assistant including:
/// - Role and communication style
/// - Available MCP tools with parameters
/// - Guidelines for data handling
/// - Example interaction patterns
pub const PIERRE_SYSTEM_PROMPT: &str =
    include_str!("../../../../vendor/contremaitre/prompts/system/pierre_system.md");

/// Coach generation system prompt
///
/// Contains instructions for the LLM to analyze a conversation and generate
/// a specialized coach profile with title, description, system prompt, and tags.
pub const COACH_GENERATION_PROMPT: &str =
    include_str!("../../../../vendor/contremaitre/prompts/system/coach_generation.md");

/// Insight validation system prompt
///
/// Contains instructions for the LLM to evaluate fitness content quality
/// before sharing to social feed. Returns valid, improved, or rejected verdict.
pub const INSIGHT_VALIDATION_PROMPT: &str =
    include_str!("../../../../vendor/contremaitre/prompts/system/insight_validation.md");

/// Insight generation system prompt
///
/// Contains instructions for the LLM to transform a fitness analysis into
/// a shareable social post with hashtags, ready for direct copying.
pub const INSIGHT_GENERATION_PROMPT: &str =
    include_str!("../../../../vendor/contremaitre/prompts/system/insight_generation.md");

/// Messaging channel context prompt
///
/// Appended to the system prompt when the LLM is replying via a chat channel
/// (`WhatsApp`, Telegram, Slack, Discord, Messenger). Instructs the model to
/// keep responses concise and mobile-friendly.
pub const MESSAGING_CONTEXT_PROMPT: &str =
    include_str!("../../../../vendor/contremaitre/prompts/system/messaging_context.md");

/// Mandatory tool-discipline prompt — tool-capable channel variant.
///
/// Appended to every system prompt on channels that render markdown and
/// can surface structured output to the user (web chat, MCP clients, A2A).
/// Contains the full rule set including the `<tool_call>` format example.
pub const TOOL_DISCIPLINE_PROMPT: &str =
    include_str!("../../../../vendor/contremaitre/prompts/system/tool_discipline.md");

/// Mandatory tool-discipline prompt — messaging channel variant.
///
/// Appended to every system prompt on mobile messaging channels
/// (`WhatsApp`, Telegram, Slack, Discord, Messenger). Keeps the
/// data-honesty and tool-usage rules but drops the markdown-heavy XML
/// format block, which conflicts with [`MESSAGING_CONTEXT_PROMPT`]'s
/// plain-text mandate and biases models toward structured output on
/// channels where the user only sees plain text.
pub const TOOL_DISCIPLINE_MESSAGING_PROMPT: &str =
    include_str!("../../../../vendor/contremaitre/prompts/system/tool_discipline_messaging.md");

/// Recommendation analysis user prompt template
///
/// Contains the user-facing prompt for generating training recommendations.
/// Template placeholders: `{activity_summary}`, `{recommendation_type}`.
pub const RECOMMENDATION_ANALYSIS_PROMPT: &str =
    include_str!("../../../../vendor/contremaitre/prompts/system/recommendation_analysis.md");

/// Recommendation analysis system prompt
///
/// System prompt for the LLM when generating training recommendations.
/// Instructs the model to respond as an expert fitness coach with valid JSON.
pub const RECOMMENDATION_SYSTEM_PROMPT: &str =
    include_str!("../../../../vendor/contremaitre/prompts/system/recommendation_system.md");

/// Activity analysis user prompt template
///
/// Contains the user-facing prompt for AI-powered activity analysis.
/// Template placeholder: `{activity_summary}`.
pub const ACTIVITY_ANALYSIS_PROMPT: &str =
    include_str!("../../../../vendor/contremaitre/prompts/system/activity_analysis.md");

/// Activity analysis system prompt
///
/// System prompt for the LLM when analyzing individual activities.
/// Instructs the model to respond as an expert fitness coach with valid JSON.
pub const ACTIVITY_ANALYSIS_SYSTEM_PROMPT: &str =
    include_str!("../../../../vendor/contremaitre/prompts/system/activity_analysis_system.md");

/// Memory extraction system prompt (Tier 2 semantic user memory)
///
/// Instructs the LLM to read one coaching exchange and emit a JSON array
/// of `Fact` objects describing durable claims the user made about
/// themselves (preferences, physiology, injuries, goals, schedule, equipment).
pub const MEMORY_EXTRACTION_PROMPT: &str =
    include_str!("../../../../vendor/contremaitre/prompts/system/memory_extraction.md");

/// Casual coaching persona block — friend-texting tone, prose, sub-150 words.
pub const CASUAL_PERSONA_PROMPT: &str =
    include_str!("../../../../vendor/contremaitre/prompts/personas/casual.md");

/// Enthusiast coaching persona block — prose with optional data citations.
pub const ENTHUSIAST_PERSONA_PROMPT: &str =
    include_str!("../../../../vendor/contremaitre/prompts/personas/enthusiast.md");

/// Power-athlete coaching persona block — Endurance discipline, line-by-line
/// per-activity reports, framework citations on every numeric claim, P0–P3 ladder.
pub const POWER_ATHLETE_PERSONA_PROMPT: &str =
    include_str!("../../../../vendor/contremaitre/prompts/personas/power_athlete.md");

/// Coach coaching persona block — inherits Power-athlete plus roster framing
/// and tenant-scoped athlete reports.
pub const COACH_PERSONA_PROMPT: &str =
    include_str!("../../../../vendor/contremaitre/prompts/personas/coach.md");

/// Get the system prompt for the Pierre fitness assistant
///
/// This is the default system prompt used when starting a new conversation.
/// It includes tool definitions that match the MCP server's capabilities.
#[must_use]
pub const fn get_pierre_system_prompt() -> &'static str {
    PIERRE_SYSTEM_PROMPT
}

/// Get the system prompt for coach generation from conversations
///
/// This prompt instructs the LLM to analyze a conversation and generate
/// a structured coach profile in JSON format.
#[must_use]
pub const fn get_coach_generation_prompt() -> &'static str {
    COACH_GENERATION_PROMPT
}

/// Get the system prompt for insight quality validation
///
/// This prompt instructs the LLM to evaluate fitness content quality
/// before sharing to social feed, returning a verdict with optional improvements.
#[must_use]
pub const fn get_insight_validation_prompt() -> &'static str {
    INSIGHT_VALIDATION_PROMPT
}

/// Get the system prompt for insight generation
///
/// This prompt instructs the LLM to transform a fitness analysis into
/// a shareable social post with hashtags, ready for direct copying.
#[must_use]
pub const fn get_insight_generation_prompt() -> &'static str {
    INSIGHT_GENERATION_PROMPT
}

/// Get the messaging channel context prompt
///
/// Appended to the system prompt when replying via chat channels.
/// Constrains response length and formatting for mobile chat UX.
#[must_use]
pub const fn get_messaging_context_prompt() -> &'static str {
    MESSAGING_CONTEXT_PROMPT
}

/// Get the tool-discipline prompt for tool-capable channels (web / MCP).
#[must_use]
pub const fn get_tool_discipline_prompt() -> &'static str {
    TOOL_DISCIPLINE_PROMPT
}

/// Get the tool-discipline prompt for messaging channels.
#[must_use]
pub const fn get_tool_discipline_messaging_prompt() -> &'static str {
    TOOL_DISCIPLINE_MESSAGING_PROMPT
}

/// Get the recommendation analysis user prompt template
///
/// Template with `{activity_summary}` and `{recommendation_type}` placeholders
/// for generating training recommendations via MCP sampling.
#[must_use]
pub const fn get_recommendation_analysis_prompt() -> &'static str {
    RECOMMENDATION_ANALYSIS_PROMPT
}

/// Get the recommendation analysis system prompt
#[must_use]
pub const fn get_recommendation_system_prompt() -> &'static str {
    RECOMMENDATION_SYSTEM_PROMPT
}

/// Get the activity analysis user prompt template
///
/// Template with `{activity_summary}` placeholder for AI-powered
/// activity analysis via MCP sampling.
#[must_use]
pub const fn get_activity_analysis_prompt() -> &'static str {
    ACTIVITY_ANALYSIS_PROMPT
}

/// Get the activity analysis system prompt
#[must_use]
pub const fn get_activity_analysis_system_prompt() -> &'static str {
    ACTIVITY_ANALYSIS_SYSTEM_PROMPT
}

/// Get the Tier 2 memory extraction system prompt.
#[must_use]
pub const fn get_memory_extraction_prompt() -> &'static str {
    MEMORY_EXTRACTION_PROMPT
}

/// Get the persona-specific block to substitute for `{{COACHING_PERSONA_RULES}}`
/// in [`PIERRE_SYSTEM_PROMPT`].
///
/// Persona is orthogonal to the chosen coach personality — it controls output
/// format (structure, citation density, length) and notification cadence, not
/// voice or domain.
#[must_use]
pub const fn get_coaching_persona_prompt(persona: CoachingPersona) -> &'static str {
    match persona {
        CoachingPersona::Casual => CASUAL_PERSONA_PROMPT,
        CoachingPersona::Enthusiast => ENTHUSIAST_PERSONA_PROMPT,
        CoachingPersona::PowerAthlete => POWER_ATHLETE_PERSONA_PROMPT,
        CoachingPersona::Coach => COACH_PERSONA_PROMPT,
    }
}

/// Required `{{PLACEHOLDER}}` tokens that every system prompt key MUST
/// contain to be acceptable as a hot-reloaded replacement.
///
/// Manifest keys mirror those used by `manifest.json::prompts.system` in
/// the contremaitre repo. Keys absent from this table have no required
/// placeholders; their content is accepted as-is by the hot-reload layer.
///
/// This is consumed by the contremaitre sync engine to refuse drift —
/// e.g. when a new placeholder ships in [`PIERRE_SYSTEM_PROMPT`] but the
/// mirrored copy in contremaitre lags behind. Without this gate the
/// runtime substitution silently turns into a no-op and the feature
/// behind the placeholder (persona, scope refusal, …) disappears in
/// production. See `crates/pierre-server/src/contremaitre/sync.rs`.
pub const REQUIRED_SYSTEM_PROMPT_PLACEHOLDERS: &[(&str, &[&str])] = &[(
    "pierre_system",
    &[
        "{{SCOPE_REFUSAL}}",
        "{{CAPABILITY_REFUSAL}}",
        "{{COACH_SCOPE_CARVE_OUT}}",
        "{{COACHING_PERSONA_RULES}}",
    ],
)];

/// Look up the placeholders that the given system-prompt manifest key
/// must contain. Returns `None` for keys with no declared requirements.
#[must_use]
pub fn required_placeholders_for_system_prompt(key: &str) -> Option<&'static [&'static str]> {
    REQUIRED_SYSTEM_PROMPT_PLACEHOLDERS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, p)| *p)
}

/// Return the subset of `required` placeholders that are NOT present in
/// `content`. Empty result means the content satisfies all requirements.
#[must_use]
pub fn missing_placeholders<'a>(content: &str, required: &'a [&'a str]) -> Vec<&'a str> {
    required
        .iter()
        .copied()
        .filter(|p| !content.contains(p))
        .collect()
}

/// Find any `{{IDENT}}` patterns remaining in `prompt` where `IDENT` is a
/// non-empty sequence of uppercase ASCII letters and underscores.
///
/// Used after placeholder interpolation to detect substitution gaps —
/// either drift in contremaitre content (the load-time check should
/// already have rejected this) or a missing branch in the assembler.
/// Strict character class avoids false positives from JSON examples
/// like `{{ "key": ... }}` that legitimately appear in some prompts.
#[must_use]
pub fn unsubstituted_placeholders(prompt: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = prompt;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let ident_len = after
            .bytes()
            .take_while(|b| b.is_ascii_uppercase() || *b == b'_')
            .count();
        if ident_len > 0 && after.as_bytes().get(ident_len..ident_len + 2) == Some(b"}}") {
            let end = start + 2 + ident_len + 2;
            out.push(rest[start..end].to_owned());
            rest = &rest[end..];
        } else {
            rest = &rest[start + 2..];
        }
    }
    out
}
