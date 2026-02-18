// ABOUTME: Model pricing registry for LLM cost tracking and billing
// ABOUTME: Maps (provider, model) pairs to per-token pricing with admin override support
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # LLM Model Pricing Registry
//!
//! Provides compile-time pricing data for supported LLM providers and models,
//! with optional admin overrides via the config system. Used by the usage tracking
//! pipeline to calculate per-request costs.

use tracing::warn;

use crate::config::admin::service::AdminConfigService;
use crate::errors::AppResult;

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
];

/// Look up pricing for a (provider, model) pair using prefix matching
fn lookup_pricing(provider: &str, model: &str) -> Option<&'static ModelPricing> {
    PRICING_TABLE
        .iter()
        .find(|(p, prefix, _)| *p == provider && model.starts_with(prefix))
        .map(|(_, _, pricing)| pricing)
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

/// Calculate cost with admin overrides from the config system
///
/// Checks for tenant-specific pricing overrides in the admin config under keys:
/// - `llm_pricing.{provider}.{model_prefix}.input_per_million`
/// - `llm_pricing.{provider}.{model_prefix}.output_per_million`
///
/// Falls back to compile-time constants if no overrides exist.
///
/// # Errors
///
/// Returns an error if the config service fails to read override values.
pub async fn calculate_cost_with_overrides(
    provider: &str,
    model: &str,
    prompt_tokens: i64,
    completion_tokens: i64,
    config: Option<&AdminConfigService>,
    tenant_id: Option<&str>,
) -> AppResult<f64> {
    let divisor = 1_000_000.0;

    // Find the matching prefix to construct config keys
    let Some((p, prefix, pricing)) = PRICING_TABLE
        .iter()
        .find(|(p, pfx, _)| *p == provider && model.starts_with(pfx))
    else {
        warn!(
            provider = provider,
            model = model,
            "No pricing data for model, cost will be recorded as 0.0"
        );
        return Ok(0.0);
    };

    let (input_rate, output_rate) = if let Some(cfg) = config {
        let config_prefix = format!("llm_pricing.{p}.{prefix}");
        let input_key = format!("{config_prefix}.input_per_million");
        let output_key = format!("{config_prefix}.output_per_million");

        let input_override = cfg
            .get_value(&input_key, tenant_id)
            .await?
            .and_then(|v| v.as_f64());
        let output_override = cfg
            .get_value(&output_key, tenant_id)
            .await?
            .and_then(|v| v.as_f64());

        (
            input_override.unwrap_or(pricing.input_per_million),
            output_override.unwrap_or(pricing.output_per_million),
        )
    } else {
        (pricing.input_per_million, pricing.output_per_million)
    };

    let input_cost = prompt_tokens as f64 * input_rate / divisor;
    Ok((completion_tokens as f64).mul_add(output_rate / divisor, input_cost))
}
