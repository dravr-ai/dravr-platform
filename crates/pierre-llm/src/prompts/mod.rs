// ABOUTME: System prompts for LLM interactions loaded at compile time
// ABOUTME: Provides the Pierre fitness assistant system prompt for Gemini function calling
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # System Prompts
//!
//! This module provides system prompts for LLM interactions.
//! Prompts are loaded at compile time from markdown files for easy maintenance.

/// Pierre Fitness Intelligence Assistant system prompt
///
/// Contains instructions for the AI assistant including:
/// - Role and communication style
/// - Available MCP tools with parameters
/// - Guidelines for data handling
/// - Example interaction patterns
pub const PIERRE_SYSTEM_PROMPT: &str = include_str!("pierre_system.md");

/// Coach generation system prompt
///
/// Contains instructions for the LLM to analyze a conversation and generate
/// a specialized coach profile with title, description, system prompt, and tags.
pub const COACH_GENERATION_PROMPT: &str = include_str!("coach_generation.md");

/// Insight validation system prompt
///
/// Contains instructions for the LLM to evaluate fitness content quality
/// before sharing to social feed. Returns valid, improved, or rejected verdict.
pub const INSIGHT_VALIDATION_PROMPT: &str = include_str!("insight_validation.md");

/// Insight generation system prompt
///
/// Contains instructions for the LLM to transform a fitness analysis into
/// a shareable social post with hashtags, ready for direct copying.
pub const INSIGHT_GENERATION_PROMPT: &str = include_str!("insight_generation.md");

/// Messaging channel context prompt
///
/// Appended to the system prompt when the LLM is replying via a chat channel
/// (`WhatsApp`, Telegram, Slack, Discord, Messenger). Instructs the model to
/// keep responses concise and mobile-friendly.
pub const MESSAGING_CONTEXT_PROMPT: &str = include_str!("messaging_context.md");

/// Mandatory tool-discipline prompt — tool-capable channel variant.
///
/// Appended to every system prompt on channels that render markdown and
/// can surface structured output to the user (web chat, MCP clients, A2A).
/// Contains the full rule set including the `<tool_call>` format example.
pub const TOOL_DISCIPLINE_PROMPT: &str = include_str!("tool_discipline.md");

/// Mandatory tool-discipline prompt — messaging channel variant.
///
/// Appended to every system prompt on mobile messaging channels
/// (`WhatsApp`, Telegram, Slack, Discord, Messenger). Keeps the
/// data-honesty and tool-usage rules but drops the markdown-heavy XML
/// format block, which conflicts with [`MESSAGING_CONTEXT_PROMPT`]'s
/// plain-text mandate and biases models toward structured output on
/// channels where the user only sees plain text.
pub const TOOL_DISCIPLINE_MESSAGING_PROMPT: &str = include_str!("tool_discipline_messaging.md");

/// Recommendation analysis user prompt template
///
/// Contains the user-facing prompt for generating training recommendations.
/// Template placeholders: `{activity_summary}`, `{recommendation_type}`.
pub const RECOMMENDATION_ANALYSIS_PROMPT: &str = include_str!("recommendation_analysis.md");

/// Recommendation analysis system prompt
///
/// System prompt for the LLM when generating training recommendations.
/// Instructs the model to respond as an expert fitness coach with valid JSON.
pub const RECOMMENDATION_SYSTEM_PROMPT: &str = include_str!("recommendation_system.md");

/// Activity analysis user prompt template
///
/// Contains the user-facing prompt for AI-powered activity analysis.
/// Template placeholder: `{activity_summary}`.
pub const ACTIVITY_ANALYSIS_PROMPT: &str = include_str!("activity_analysis.md");

/// Activity analysis system prompt
///
/// System prompt for the LLM when analyzing individual activities.
/// Instructs the model to respond as an expert fitness coach with valid JSON.
pub const ACTIVITY_ANALYSIS_SYSTEM_PROMPT: &str = include_str!("activity_analysis_system.md");

/// Memory extraction system prompt (Tier 2 semantic user memory)
///
/// Instructs the LLM to read one coaching exchange and emit a JSON array
/// of `Fact` objects describing durable claims the user made about
/// themselves (preferences, physiology, injuries, goals, schedule, equipment).
pub const MEMORY_EXTRACTION_PROMPT: &str = include_str!("memory_extraction.md");

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
