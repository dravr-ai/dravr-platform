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
//! through [`pierre_core::llm::TokenUsage`] and applied here
//! via [`calculate_cost_with_cache`].
//!
//! ## Admin-editable overrides
//!
//! [`PricingRegistry`] layers per-tenant and global operator overrides
//! on top of the compile-time table. Overrides are fetched from
//! `admin_config_overrides` under the `cat_llm_pricing` category and
//! cached in-process; the compile-time table remains the fallback for
//! models the operator has not repriced.

use pierre_core::models::usage::{LlmUsageAggregateRow, LlmUsageRecord};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};
use tracing::{debug, info, warn};

/// Process-wide [`PricingRegistry`] singleton.
///
/// The pierre-server startup hook reads `cat_llm_pricing` overrides from
/// `admin_config_overrides` once the database is ready and calls
/// [`PricingRegistry::replace_global`] on this instance. Per-call cost
/// computation in the chat pipeline reads back from the same singleton
/// so overrides take effect without restart.
pub static GLOBAL_PRICING_REGISTRY: LazyLock<PricingRegistry> = LazyLock::new(PricingRegistry::new);

/// Fraction of the full input rate at which context-cache hits are billed.
///
/// Matches `Gemini`'s advertised 25% cache-read discount and `OpenAI`'s
/// 50% discount on `cached_tokens` (taking the more conservative 25%
/// for billing parity across providers).
///
/// LIMITATION(registre#102): `CACHED_TOKEN_RATE` is one flat read discount for every provider — Anthropic bills cache reads at 0.10× and cache writes at 1.25×, and the ACP path reports no cache reads at all, so a native turn's 40–55K-token prefix is always imputed at the full input rate.
pub const CACHED_TOKEN_RATE: f64 = 0.25;

/// Per-model pricing rates in USD per million tokens
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ModelPricing {
    /// USD per 1 million input (prompt) tokens
    pub input_per_million: f64,
    /// USD per 1 million output (completion) tokens
    pub output_per_million: f64,
}

/// Providers whose per-token cost is genuinely \$0 by design, so a \$0 billed
/// cost is *correct* rather than a missing-price undercount.
///
/// Two classes share this property:
///
/// - **Self-hosted** runtimes (Ollama / vLLM / `LocalAI` / generic local
///   `OpenAI`-compatible endpoints): the operator pays for the hardware, not
///   per token, so the metered per-token cost is zero.
/// - **Flat-rate subscription** CLI/agent runners (Claude Code, GitHub
///   Copilot, Cursor, `OpenCode`, Codex, Goose, Cline, Continue, Warp, Kiro,
///   Kilo): billed by the upstream subscription, not per token through this
///   platform.
///
/// Entries match the machine `name()` string each provider reports onto the
/// usage record (see `LlmProvider::name`). A model under one of these prefixes
/// resolves to \$0 *without* the missing-price warning, so cost dashboards do
/// not flag it as an undercount. `copilot_headless` and `claude_code` are
/// deliberately absent here: they carry real per-token `PRICING_TABLE` entries
/// because their Anthropic pass-through usage is metered.
const NOT_PER_TOKEN_METERED_PROVIDERS: &[&str] = &[
    // Self-hosted OpenAI-compatible runtimes.
    "local",
    "ollama",
    "vllm",
    "localai",
    // Flat-rate subscription CLI / agent runners (machine names as reported
    // by each runner's name()).
    "cursor-agent",
    "claude-code",
    "opencode",
    "codex",
    "goose",
    "cline",
    "continue",
    "warp_cli",
    "kiro",
    "kilo",
    "copilot",
];

/// `OpenRouter` exposes 200+ models with per-model pricing, so a static table
/// can never be exhaustive. An unknown `OpenRouter` model is therefore logged
/// as *model-specific price unset* (a deliberate, non-alarming message) rather
/// than counted as a silent undercount alongside genuinely-unexpected misses.
const OPENROUTER_PROVIDER: &str = "openrouter";

/// Compile-time pricing table: `(provider, model_prefix, pricing)`
///
/// Model matching uses prefix comparison — a model name like "gemini-2.0-flash-exp"
/// matches the prefix "gemini-2.0-flash". Entries are ordered longest-prefix-first
/// within each provider to ensure the most specific match wins.
const PRICING_TABLE: &[(&str, &str, ModelPricing)] = &[
    // Gemini models (provider name matches GeminiProvider::name() = "gemini")
    // gemini-flash-lite-latest is a Google-maintained rolling alias to the
    // current GA flash-lite tier; prices track whatever that tier costs today.
    (
        "gemini",
        "gemini-flash-lite-latest",
        ModelPricing {
            input_per_million: 0.10,
            output_per_million: 0.40,
        },
    ),
    (
        "gemini",
        "gemini-2.5-flash-lite",
        ModelPricing {
            input_per_million: 0.10,
            output_per_million: 0.40,
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
        // Prefix spans every Sonnet generation (claude-sonnet-4, -4.5, -4.6, -5);
        // all bill at the same $3/$15 Sonnet rate, so a version-agnostic prefix
        // keeps shadow-COGS attributed instead of falling through to $0 on a bump.
        "claude-sonnet",
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
        // Version-agnostic Sonnet prefix (claude-sonnet-4, -4.5, -4.6, -5),
        // mirroring the copilot_headless entry so a model bump keeps shadow-COGS
        // attributed instead of falling through to $0.
        "claude-sonnet",
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
    // Cohere — Command A and Command R family.
    // Entries are ordered longest-prefix-first so `command-a-reasoning` and
    // `command-a-vision` match before the bare `command-a` prefix and the
    // R-family entries don't accidentally swallow R+ / R7B.
    (
        "cohere",
        "command-a-reasoning",
        ModelPricing {
            input_per_million: 2.50,
            output_per_million: 10.0,
        },
    ),
    (
        "cohere",
        "command-a-vision",
        ModelPricing {
            input_per_million: 2.50,
            output_per_million: 10.0,
        },
    ),
    (
        "cohere",
        "command-a",
        ModelPricing {
            input_per_million: 2.50,
            output_per_million: 10.0,
        },
    ),
    (
        "cohere",
        "command-r-plus",
        ModelPricing {
            input_per_million: 2.50,
            output_per_million: 10.0,
        },
    ),
    (
        "cohere",
        "command-r7b",
        ModelPricing {
            input_per_million: 0.0375,
            output_per_million: 0.15,
        },
    ),
    (
        "cohere",
        "command-r",
        ModelPricing {
            input_per_million: 0.15,
            output_per_million: 0.60,
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
    // OpenRouter — the gateway passes through the underlying model's published
    // price. These cover the curated default slugs the platform ships
    // (HARDCODED_DEFAULT_MODEL + openrouter::AVAILABLE_MODELS); any other slug
    // is honestly logged as model-specific-unset rather than silently $0.
    // Prefixes carry the provider/model slug so prefix matching still resolves
    // dated or suffixed variants (e.g. `:free`, `-001`).
    (
        "openrouter",
        "meta-llama/llama-3.3-70b-instruct",
        ModelPricing {
            input_per_million: 0.12,
            output_per_million: 0.30,
        },
    ),
    (
        "openrouter",
        "meta-llama/llama-3.1-8b-instruct",
        ModelPricing {
            input_per_million: 0.02,
            output_per_million: 0.03,
        },
    ),
    (
        "openrouter",
        "anthropic/claude-3.5-sonnet",
        ModelPricing {
            input_per_million: 3.0,
            output_per_million: 15.0,
        },
    ),
    (
        "openrouter",
        "anthropic/claude-3.5-haiku",
        ModelPricing {
            input_per_million: 0.80,
            output_per_million: 4.0,
        },
    ),
    (
        "openrouter",
        "openai/gpt-4o-mini",
        ModelPricing {
            input_per_million: 0.15,
            output_per_million: 0.60,
        },
    ),
    (
        "openrouter",
        "openai/gpt-4o",
        ModelPricing {
            input_per_million: 2.50,
            output_per_million: 10.0,
        },
    ),
    (
        "openrouter",
        "google/gemini-2.0-flash-001",
        ModelPricing {
            input_per_million: 0.10,
            output_per_million: 0.40,
        },
    ),
    (
        "openrouter",
        "google/gemini-pro-1.5",
        ModelPricing {
            input_per_million: 1.25,
            output_per_million: 5.0,
        },
    ),
    (
        "openrouter",
        "mistralai/mistral-large",
        ModelPricing {
            input_per_million: 2.0,
            output_per_million: 6.0,
        },
    ),
    (
        "openrouter",
        "mistralai/mistral-nemo",
        ModelPricing {
            input_per_million: 0.03,
            output_per_million: 0.07,
        },
    ),
    (
        "openrouter",
        "qwen/qwen-2.5-72b-instruct",
        ModelPricing {
            input_per_million: 0.13,
            output_per_million: 0.40,
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

/// True when a provider bills via flat-rate subscription or self-hosting, so a
/// \$0 per-token cost is correct rather than a missing-price undercount.
#[must_use]
pub fn is_not_per_token_metered(provider: &str) -> bool {
    NOT_PER_TOKEN_METERED_PROVIDERS.contains(&provider)
}

/// Log the resolution of a (provider, model) pair that has no `PRICING_TABLE`
/// or override entry, at a severity matching *why* it is unpriced, and return
/// the \$0 cost all three paths fall back to.
///
/// - Subscription / self-hosted providers: \$0 is correct — `debug!` only, no
///   undercount warning.
/// - `OpenRouter`: model-specific price genuinely unset for this slug —
///   `info!` with a distinct, non-alarming message.
/// - Anything else: a genuinely-unexpected miss that *does* undercount —
///   keep the `warn!`.
fn zero_cost_for_unpriced(provider: &str, model: &str, tenant_id: Option<&str>) -> f64 {
    if is_not_per_token_metered(provider) {
        debug!(
            provider = provider,
            model = model,
            "Provider is subscription/self-hosted; per-token cost is $0 by design (not an undercount)"
        );
    } else if provider == OPENROUTER_PROVIDER {
        info!(
            provider = provider,
            model = model,
            "OpenRouter model price is model-specific and unset for this slug; recording $0 (add a PRICING_TABLE or admin override to meter it)"
        );
    } else {
        warn!(
            provider = provider,
            model = model,
            tenant_id = tenant_id,
            "No pricing data for model, cost will be recorded as 0.0"
        );
    }
    0.0
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
/// 0.0 for unpriced (provider, model) pairs — the $0 fallback is deliberate
/// so a missing pricing entry never blocks a chat turn. The log severity of
/// that fallback depends on *why* the pair is unpriced (see
/// [`zero_cost_for_unpriced`]): subscription/self-hosted providers log at
/// debug (their $0 is correct), `OpenRouter` logs an honest model-specific
/// notice, and only a genuinely-unexpected miss emits the undercount warning.
#[must_use]
pub fn calculate_cost_with_cache(
    provider: &str,
    model: &str,
    prompt_tokens: i64,
    cached_tokens: i64,
    completion_tokens: i64,
) -> f64 {
    let Some(pricing) = lookup_pricing(provider, model) else {
        return zero_cost_for_unpriced(provider, model, None);
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
            return zero_cost_for_unpriced(provider, model, tenant_id);
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

/// Cost of one recorded LLM call, crediting the prompt tokens it served from
/// cache.
///
/// Every read path recomputes cost from a stored row, and each one used to
/// spell the five arguments out at the call site — so each was free to forget
/// the cache one, and all of them had: they called the four-argument
/// `calculate_cost`, which passes 0. Taking the row itself removes the
/// opportunity.
#[must_use]
pub fn cost_for_record(record: &LlmUsageRecord) -> f64 {
    calculate_cost_with_cache(
        &record.provider,
        &record.model,
        record.prompt_tokens,
        record.cached_tokens,
        record.completion_tokens,
    )
}

/// Cost of a grouped usage row, crediting its summed cache reads.
///
/// Counterpart to [`cost_for_record`] for the aggregate and daily series.
#[must_use]
pub fn cost_for_aggregate(row: &LlmUsageAggregateRow) -> f64 {
    calculate_cost_with_cache(
        &row.provider,
        &row.model,
        row.prompt_tokens,
        row.cached_tokens,
        row.completion_tokens,
    )
}
