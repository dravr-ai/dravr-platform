// ABOUTME: Model pricing registry for LLM cost tracking and billing
// ABOUTME: Maps (provider, model) pairs to per-token pricing with prefix-based lookup
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # LLM Model Pricing Registry
//!
//! Provides compile-time pricing data for supported LLM providers and models.
//! Used by the usage tracking pipeline to calculate per-request costs.
//!
//! ## Cached-token discount
//!
//! Prompt tokens that hit the provider's context cache bill at 25% of
//! the full input rate. Gemini and OpenAI both report cached-token
//! counts in their usage payloads; the value is carried end-to-end
//! through [`pierre_core::llm::ExtendedTokenUsage`] and applied here
//! via [`calculate_cost_with_cache`].
//!
//! ## Admin-editable overrides
//!
//! [`PricingRegistry`] layers per-tenant and global operator overrides
//! on top of the compile-time table. Overrides are fetched from
//! `admin_config_overrides` under the `cat_llm_pricing` category and
//! cached in-process; the compile-time table remains the fallback for
//! models the operator has not repriced.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::RwLock;
use tracing::warn;

/// Fraction of the full input rate at which context-cache hits are billed.
///
/// Matches `Gemini`'s advertised 25% cache-read discount and `OpenAI`'s
/// 50% discount on `cached_tokens` (taking the more conservative 25%
/// for billing parity across providers).
pub const CACHED_TOKEN_RATE: f64 = 0.25;

/// Per-model pricing rates in USD per million tokens
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ModelPricing {
    /// USD per 1 million input (prompt) tokens
    pub input_per_million: f64,
    /// USD per 1 million output (completion) tokens
    pub output_per_million: f64,
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
fn lookup_pricing(provider: &str, model: &str) -> Option<ModelPricing> {
    PRICING_TABLE
        .iter()
        .find(|(p, prefix, _)| *p == provider && model.starts_with(prefix))
        .map(|(_, _, pricing)| *pricing)
}

/// Calculate the cost of an LLM request using compile-time pricing.
///
/// Returns the cost in USD. Returns 0.0 for unknown provider/model combinations
/// (with a warning log). Does not account for cached tokens — callers that
/// have a cache hit count must use [`calculate_cost_with_cache`].
#[must_use]
pub fn calculate_cost(
    provider: &str,
    model: &str,
    prompt_tokens: i64,
    completion_tokens: i64,
) -> f64 {
    calculate_cost_with_cache(provider, model, prompt_tokens, 0, completion_tokens)
}

/// Calculate the cost of an LLM request, breaking the prompt into cached + fresh tokens.
///
/// Cached tokens bill at [`CACHED_TOKEN_RATE`] of the model's input rate;
/// the remaining `(prompt_tokens - cached_tokens)` bill at the full input
/// rate. Completion tokens always bill at the full output rate. Returns
/// 0.0 for unknown (provider, model) pairs and logs a warning —
/// billing-silent fallback behaviour is deliberate so a missing pricing
/// entry never blocks a chat turn.
#[must_use]
pub fn calculate_cost_with_cache(
    provider: &str,
    model: &str,
    prompt_tokens: i64,
    cached_tokens: i64,
    completion_tokens: i64,
) -> f64 {
    let Some(pricing) = lookup_pricing(provider, model) else {
        warn!(
            provider = provider,
            model = model,
            "No pricing data for model, cost will be recorded as 0.0"
        );
        return 0.0;
    };

    cost_from_pricing(&pricing, prompt_tokens, cached_tokens, completion_tokens)
}

/// Compute the USD cost from a resolved [`ModelPricing`] + token counts.
/// Shared by the compile-time lookup above and the [`PricingRegistry`]
/// override path so both sources apply the cached-token discount
/// identically.
fn cost_from_pricing(
    pricing: &ModelPricing,
    prompt_tokens: i64,
    cached_tokens: i64,
    completion_tokens: i64,
) -> f64 {
    let divisor = 1_000_000.0;
    let cached = cached_tokens.min(prompt_tokens).max(0);
    let fresh_prompt = (prompt_tokens - cached).max(0);

    let fresh_prompt_cost = fresh_prompt as f64 * pricing.input_per_million / divisor;
    let cached_prompt_cost =
        cached as f64 * pricing.input_per_million * CACHED_TOKEN_RATE / divisor;
    let completion_cost = completion_tokens as f64 * pricing.output_per_million / divisor;

    fresh_prompt_cost + cached_prompt_cost + completion_cost
}

/// Map keyed by `(provider, model_prefix)` to a single pricing entry.
pub type PricingOverrideMap = HashMap<(String, String), ModelPricing>;

/// Admin-editable layer over the compile-time [`PRICING_TABLE`].
///
/// Overrides are stored in `admin_config_overrides` under the
/// `cat_llm_pricing` category and loaded into this registry at server
/// startup (and on config-change broadcast). Lookup order:
///
/// 1. Tenant-scoped override for `(provider, model_prefix)`
/// 2. Global override for `(provider, model_prefix)`
/// 3. Compile-time [`PRICING_TABLE`] entry
///
/// All override keys use the same longest-prefix matching rule as the
/// compile-time table so operators can reprice a family without listing
/// every model variant.
#[derive(Debug, Default)]
pub struct PricingRegistry {
    /// `(provider, model_prefix) -> pricing`, populated from admin config.
    global: RwLock<PricingOverrideMap>,
    /// `tenant_id -> (provider, model_prefix) -> pricing`.
    per_tenant: RwLock<HashMap<String, PricingOverrideMap>>,
}

impl PricingRegistry {
    /// Build an empty registry that falls through to the compile-time table.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the in-process global override map. Called after the admin
    /// config loader fetches `cat_llm_pricing` rows from the database.
    pub fn replace_global(&self, overrides: PricingOverrideMap) {
        if let Ok(mut guard) = self.global.write() {
            *guard = overrides;
        }
    }

    /// Replace overrides for a single tenant. A tenant with no overrides
    /// is simply absent from the map and inherits from the global layer.
    pub fn replace_tenant(&self, tenant_id: String, overrides: PricingOverrideMap) {
        if let Ok(mut guard) = self.per_tenant.write() {
            guard.insert(tenant_id, overrides);
        }
    }

    /// Resolve pricing for `(provider, model)` under an optional tenant
    /// scope, applying longest-prefix matching at each layer.
    fn resolve(
        &self,
        tenant_id: Option<&str>,
        provider: &str,
        model: &str,
    ) -> Option<ModelPricing> {
        if let Some(tenant) = tenant_id {
            if let Ok(guard) = self.per_tenant.read() {
                if let Some(tenant_map) = guard.get(tenant) {
                    if let Some(p) = longest_prefix_match(tenant_map, provider, model) {
                        return Some(p);
                    }
                }
            }
        }
        if let Ok(guard) = self.global.read() {
            if let Some(p) = longest_prefix_match(&guard, provider, model) {
                return Some(p);
            }
        }
        lookup_pricing(provider, model)
    }

    /// Compute cost for a (provider, model) pair under an optional tenant
    /// override scope. Callers that do not have admin overrides loaded
    /// should use [`calculate_cost_with_cache`] directly.
    #[must_use]
    pub fn calculate_cost(
        &self,
        tenant_id: Option<&str>,
        provider: &str,
        model: &str,
        prompt_tokens: i64,
        cached_tokens: i64,
        completion_tokens: i64,
    ) -> f64 {
        let Some(pricing) = self.resolve(tenant_id, provider, model) else {
            warn!(
                provider = provider,
                model = model,
                tenant_id = tenant_id,
                "No pricing data for model, cost will be recorded as 0.0"
            );
            return 0.0;
        };
        cost_from_pricing(&pricing, prompt_tokens, cached_tokens, completion_tokens)
    }
}

fn longest_prefix_match(
    map: &PricingOverrideMap,
    provider: &str,
    model: &str,
) -> Option<ModelPricing> {
    map.iter()
        .filter(|((p, prefix), _)| p == provider && model.starts_with(prefix.as_str()))
        .max_by_key(|((_, prefix), _)| prefix.len())
        .map(|(_, pricing)| *pricing)
}
