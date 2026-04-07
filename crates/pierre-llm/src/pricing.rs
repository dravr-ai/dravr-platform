// ABOUTME: Model pricing registry for LLM cost tracking and billing
// ABOUTME: Maps (provider, model) pairs to per-token pricing with prefix-based lookup
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # LLM Model Pricing Registry
//!
//! Provides compile-time pricing data for supported LLM providers and models.
//! Used by the usage tracking pipeline to calculate per-request costs.

use tracing::warn;

/// Per-model pricing rates in USD per million tokens
#[derive(Debug, Clone, Copy)]
struct ModelPricing {
    /// USD per 1 million input (prompt) tokens
    input_per_million: f64,
    /// USD per 1 million output (completion) tokens
    output_per_million: f64,
}

/// Compile-time pricing table: `(provider, model_prefix, pricing)`
///
/// Model matching uses prefix comparison — a model name like "gemini-2.0-flash-exp"
/// matches the prefix "gemini-2.0-flash". Entries are ordered longest-prefix-first
/// within each provider to ensure the most specific match wins.
const PRICING_TABLE: &[(&str, &str, ModelPricing)] = &[
    // Gemini models (provider name matches GeminiProvider::name() = "gemini")
    (
        "gemini",
        "gemini-3-flash",
        ModelPricing {
            input_per_million: 0.50,
            output_per_million: 3.0,
        },
    ),
    (
        "gemini",
        "gemini-2.5-pro",
        ModelPricing {
            input_per_million: 1.25,
            output_per_million: 10.0,
        },
    ),
    (
        "gemini",
        "gemini-2.5-flash",
        ModelPricing {
            input_per_million: 0.15,
            output_per_million: 0.60,
        },
    ),
    (
        "gemini",
        "gemini-2.0-flash",
        ModelPricing {
            input_per_million: 0.075,
            output_per_million: 0.30,
        },
    ),
    // Groq models
    (
        "groq",
        "llama-3.3-70b",
        ModelPricing {
            input_per_million: 0.59,
            output_per_million: 0.79,
        },
    ),
    (
        "groq",
        "mixtral",
        ModelPricing {
            input_per_million: 0.24,
            output_per_million: 0.24,
        },
    ),
    (
        "groq",
        "llama-3.1-8b",
        ModelPricing {
            input_per_million: 0.05,
            output_per_million: 0.08,
        },
    ),
    // Copilot headless (embacle) — proxies to Anthropic Claude models
    (
        "copilot_headless",
        "claude-opus-4",
        ModelPricing {
            input_per_million: 15.0,
            output_per_million: 75.0,
        },
    ),
    (
        "copilot_headless",
        "claude-sonnet-4",
        ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
        },
    ),
    (
        "copilot_headless",
        "claude-haiku-4",
        ModelPricing {
            input_per_million: 0.80,
            output_per_million: 4.0,
        },
    ),
    // Claude Code CLI — same models as copilot_headless
    (
        "claude_code",
        "claude-opus-4",
        ModelPricing {
            input_per_million: 15.0,
            output_per_million: 75.0,
        },
    ),
    (
        "claude_code",
        "claude-sonnet-4",
        ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
        },
    ),
    (
        "claude_code",
        "claude-haiku-4",
        ModelPricing {
            input_per_million: 0.80,
            output_per_million: 4.0,
        },
    ),
    // OpenAI API models
    (
        "openai_api",
        "gpt-4o",
        ModelPricing {
            input_per_million: 2.50,
            output_per_million: 10.0,
        },
    ),
    (
        "openai_api",
        "gpt-4o-mini",
        ModelPricing {
            input_per_million: 0.15,
            output_per_million: 0.60,
        },
    ),
];

/// Look up pricing for a (provider, model) pair using prefix matching
fn lookup_pricing(provider: &str, model: &str) -> Option<&'static ModelPricing> {
    PRICING_TABLE
        .iter()
        .find(|(p, prefix, _)| *p == provider && model.starts_with(prefix))
        .map(|(_, _, pricing)| pricing)
}

/// Average characters per token for LLM tokenizers.
///
/// Most English text averages ~4 characters per token across major tokenizers
/// (GPT, Gemini, `LLaMA`). Used as a fallback when providers don't return real
/// token counts (e.g., CLI-based and headless providers).
const CHARS_PER_TOKEN: usize = 4;

/// Estimate token count from text content using character-based heuristic.
///
/// Returns `(prompt_tokens, completion_tokens)` estimates. Used as a fallback
/// when LLM providers don't return real token counts in their API response
/// (e.g., `copilot_headless`, Claude Code CLI).
#[must_use]
pub fn estimate_tokens(prompt_text: &str, completion_text: &str) -> (u32, u32) {
    let prompt = (prompt_text.len() / CHARS_PER_TOKEN).max(1) as u32;
    let completion = (completion_text.len() / CHARS_PER_TOKEN).max(1) as u32;
    (prompt, completion)
}

/// Calculate the cost of an LLM request using compile-time pricing
///
/// Returns the cost in USD. Returns 0.0 for unknown provider/model combinations
/// (with a warning log).
#[must_use]
pub fn calculate_cost(
    provider: &str,
    model: &str,
    prompt_tokens: i64,
    completion_tokens: i64,
) -> f64 {
    let divisor = 1_000_000.0;

    let Some(pricing) = lookup_pricing(provider, model) else {
        warn!(
            provider = provider,
            model = model,
            "No pricing data for model, cost will be recorded as 0.0"
        );
        return 0.0;
    };

    let input_cost = prompt_tokens as f64 * pricing.input_per_million / divisor;
    (completion_tokens as f64).mul_add(pricing.output_per_million / divisor, input_cost)
}
