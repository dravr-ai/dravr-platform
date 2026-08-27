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
//! ## Cache accounting
//!
//! A prompt token can bill at three rates, so the pricing entry carries the
//! multipliers rather than a single global discount: fresh input at 1.0×, a
//! cache *read* at [`ModelPricing::cache_read_multiplier`] (Anthropic 0.10×,
//! `OpenAI` 0.50×, `Gemini` 0.25×), and a cache *write* at
//! [`ModelPricing::cache_write_multiplier`] — which is a **premium** on
//! Anthropic (1.25×), not a discount. Reasoning tokens bill at the output
//! rate and are additive to the completion count, because every provider that
//! reports them separately excludes them from it.
//!
//! Counts arrive on [`embacle::TokenUsage`], are carried through
//! [`TokenCounts`], and are priced by [`calculate_cost_for`].
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

/// Default fraction of the input rate at which a context-cache *read* bills.
///
/// Matches `Gemini`'s advertised 25% cache-read discount, and applies to any
/// model that does not override it via [`ModelPricing::with_cache_rates`].
/// A provider with no cache at all never reports a read, so the multiplier
/// is inert rather than wrong for those entries.
pub const DEFAULT_CACHE_READ_RATE: f64 = 0.25;

/// Default fraction of the input rate at which a context-cache *write* bills.
///
/// `1.0` — a provider that does not price cache creation separately bills
/// those tokens as ordinary input. Anthropic is the exception and overrides
/// it; see [`ModelPricing::with_cache_rates`].
pub const DEFAULT_CACHE_WRITE_RATE: f64 = 1.0;

/// Per-model pricing rates in USD per million tokens, plus the two
/// cache multipliers that turn a raw token count into a billed one.
///
/// Cache economics are per-provider, not universal: Anthropic reads at
/// 0.10× and *charges a premium* to write at 1.25×, `OpenAI` reads at 0.50×
/// and does not bill writes separately, `Gemini` reads at 0.25×. A single
/// flat discount mispriced all three, so the rates travel with the model.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct ModelPricing {
    /// USD per 1 million input (prompt) tokens
    pub input_per_million: f64,
    /// USD per 1 million output (completion) tokens
    pub output_per_million: f64,
    /// Fraction of `input_per_million` at which cache-read tokens bill.
    #[serde(default = "default_cache_read_rate")]
    pub cache_read_multiplier: f64,
    /// Fraction of `input_per_million` at which cache-write tokens bill.
    #[serde(default = "default_cache_write_rate")]
    pub cache_write_multiplier: f64,
}

/// Serde default for [`ModelPricing::cache_read_multiplier`], so an operator
/// override stored before the multipliers existed still deserializes.
fn default_cache_read_rate() -> f64 {
    DEFAULT_CACHE_READ_RATE
}

/// Serde default for [`ModelPricing::cache_write_multiplier`], so an operator
/// override stored before the multipliers existed still deserializes.
fn default_cache_write_rate() -> f64 {
    DEFAULT_CACHE_WRITE_RATE
}

impl ModelPricing {
    /// Rates for a model that uses the default cache multipliers.
    #[must_use]
    pub const fn new(input_per_million: f64, output_per_million: f64) -> Self {
        Self {
            input_per_million,
            output_per_million,
            cache_read_multiplier: DEFAULT_CACHE_READ_RATE,
            cache_write_multiplier: DEFAULT_CACHE_WRITE_RATE,
        }
    }

    /// Override the cache read/write multipliers for a provider that prices
    /// them differently — Anthropic at `(0.10, 1.25)`, `OpenAI` at `(0.50, 1.0)`.
    #[must_use]
    pub const fn with_cache_rates(mut self, read: f64, write: f64) -> Self {
        self.cache_read_multiplier = read;
        self.cache_write_multiplier = write;
        self
    }
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
        ModelPricing::new(0.10, 0.40),
    ),
    (
        "gemini",
        "gemini-2.5-flash-lite",
        ModelPricing::new(0.10, 0.40),
    ),
    ("gemini", "gemini-2.5-pro", ModelPricing::new(1.25, 10.0)),
    ("gemini", "gemini-2.5-flash", ModelPricing::new(0.15, 0.60)),
    ("gemini", "gemini-2.0-flash", ModelPricing::new(0.075, 0.30)),
    // Groq models
    ("groq", "llama-3.3-70b", ModelPricing::new(0.59, 0.79)),
    ("groq", "mixtral", ModelPricing::new(0.24, 0.24)),
    ("groq", "llama-3.1-8b", ModelPricing::new(0.05, 0.08)),
    // Copilot headless (embacle) — proxies to Anthropic Claude models
    (
        "copilot_headless",
        "claude-opus-4",
        ModelPricing::new(15.0, 75.0).with_cache_rates(0.10, 1.25),
    ),
    (
        "copilot_headless",
        // Prefix spans every Sonnet generation (claude-sonnet-4, -4.5, -4.6, -5);
        // all bill at the same $3/$15 Sonnet rate, so a version-agnostic prefix
        // keeps shadow-COGS attributed instead of falling through to $0 on a bump.
        "claude-sonnet",
        ModelPricing::new(3.0, 15.0).with_cache_rates(0.10, 1.25),
    ),
    (
        "copilot_headless",
        "claude-haiku-4",
        ModelPricing::new(0.80, 4.0).with_cache_rates(0.10, 1.25),
    ),
    // Claude Code CLI — same models as copilot_headless
    (
        "claude_code",
        "claude-opus-4",
        ModelPricing::new(15.0, 75.0).with_cache_rates(0.10, 1.25),
    ),
    (
        "claude_code",
        // Version-agnostic Sonnet prefix (claude-sonnet-4, -4.5, -4.6, -5),
        // mirroring the copilot_headless entry so a model bump keeps shadow-COGS
        // attributed instead of falling through to $0.
        "claude-sonnet",
        ModelPricing::new(3.0, 15.0).with_cache_rates(0.10, 1.25),
    ),
    (
        "claude_code",
        "claude-haiku-4",
        ModelPricing::new(0.80, 4.0).with_cache_rates(0.10, 1.25),
    ),
    // Cohere — Command A and Command R family.
    // Entries are ordered longest-prefix-first so `command-a-reasoning` and
    // `command-a-vision` match before the bare `command-a` prefix and the
    // R-family entries don't accidentally swallow R+ / R7B.
    (
        "cohere",
        "command-a-reasoning",
        ModelPricing::new(2.50, 10.0),
    ),
    ("cohere", "command-a-vision", ModelPricing::new(2.50, 10.0)),
    ("cohere", "command-a", ModelPricing::new(2.50, 10.0)),
    ("cohere", "command-r-plus", ModelPricing::new(2.50, 10.0)),
    ("cohere", "command-r7b", ModelPricing::new(0.0375, 0.15)),
    ("cohere", "command-r", ModelPricing::new(0.15, 0.60)),
    // OpenAI API models
    (
        "openai_api",
        "gpt-4o",
        ModelPricing::new(2.50, 10.0).with_cache_rates(0.50, 1.00),
    ),
    (
        "openai_api",
        "gpt-4o-mini",
        ModelPricing::new(0.15, 0.60).with_cache_rates(0.50, 1.00),
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
        ModelPricing::new(0.12, 0.30),
    ),
    (
        "openrouter",
        "meta-llama/llama-3.1-8b-instruct",
        ModelPricing::new(0.02, 0.03),
    ),
    (
        "openrouter",
        "anthropic/claude-3.5-sonnet",
        ModelPricing::new(3.0, 15.0).with_cache_rates(0.10, 1.25),
    ),
    (
        "openrouter",
        "anthropic/claude-3.5-haiku",
        ModelPricing::new(0.80, 4.0).with_cache_rates(0.10, 1.25),
    ),
    (
        "openrouter",
        "openai/gpt-4o-mini",
        ModelPricing::new(0.15, 0.60).with_cache_rates(0.50, 1.00),
    ),
    (
        "openrouter",
        "openai/gpt-4o",
        ModelPricing::new(2.50, 10.0).with_cache_rates(0.50, 1.00),
    ),
    (
        "openrouter",
        "google/gemini-2.0-flash-001",
        ModelPricing::new(0.10, 0.40),
    ),
    (
        "openrouter",
        "google/gemini-pro-1.5",
        ModelPricing::new(1.25, 5.0),
    ),
    (
        "openrouter",
        "mistralai/mistral-large",
        ModelPricing::new(2.0, 6.0),
    ),
    (
        "openrouter",
        "mistralai/mistral-nemo",
        ModelPricing::new(0.03, 0.07),
    ),
    (
        "openrouter",
        "qwen/qwen-2.5-72b-instruct",
        ModelPricing::new(0.13, 0.40),
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

/// The token counts a single LLM call reports, in the shape they bill.
///
/// `cached_read` and `cached_write` are both subsets of `prompt`: a provider
/// reports the gross prompt count and then breaks out how much of it was
/// served from cache and how much was written into it. `reasoning` is the
/// exception — providers that report it exclude it from `completion`, so it
/// is additive on the output side.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TokenCounts {
    /// Gross prompt tokens, cached portions included.
    pub prompt: i64,
    /// Prompt tokens served from the provider's context cache.
    pub cached_read: i64,
    /// Prompt tokens written into the provider's context cache this call.
    pub cached_write: i64,
    /// Completion tokens, excluding separately-reported reasoning tokens.
    pub completion: i64,
    /// Reasoning / "thought" tokens, when the provider reports them apart
    /// from `completion`. Billed at the output rate.
    pub reasoning: i64,
}

impl TokenCounts {
    /// The two counts every provider reports.
    #[must_use]
    pub const fn new(prompt: i64, completion: i64) -> Self {
        Self {
            prompt,
            cached_read: 0,
            cached_write: 0,
            completion,
            reasoning: 0,
        }
    }

    /// Attach the cache read/write split of `prompt`.
    #[must_use]
    pub const fn with_cache(mut self, read: i64, write: i64) -> Self {
        self.cached_read = read;
        self.cached_write = write;
        self
    }

    /// Attach a separately-reported reasoning-token count.
    #[must_use]
    pub const fn with_reasoning(mut self, reasoning: i64) -> Self {
        self.reasoning = reasoning;
        self
    }
}

/// Calculate the cost of an LLM request using compile-time pricing.
///
/// Returns the cost in USD. Returns 0.0 for unknown provider/model combinations
/// (with a warning log). Treats the whole prompt as uncached and assumes no
/// separately-reported reasoning tokens — callers holding either count must
/// use [`calculate_cost_with_cache`] or [`calculate_cost_for`].
#[must_use]
pub fn calculate_cost(
    provider: &str,
    model: &str,
    prompt_tokens: i64,
    completion_tokens: i64,
) -> f64 {
    calculate_cost_for(
        provider,
        model,
        &TokenCounts::new(prompt_tokens, completion_tokens),
    )
}

/// Calculate the cost of an LLM request from a full [`TokenCounts`].
///
/// The complete entry point: every count the provider reported, priced with
/// that model's own cache multipliers. Returns 0.0 for unpriced pairs on the
/// same terms as [`calculate_cost_with_cache`].
#[must_use]
pub fn calculate_cost_for(provider: &str, model: &str, counts: &TokenCounts) -> f64 {
    let Some(pricing) = lookup_pricing(provider, model) else {
        return zero_cost_for_unpriced(provider, model, None);
    };

    cost_from_pricing(&pricing, counts)
}

/// Calculate the cost of an LLM request, breaking the prompt into cached + fresh tokens.
///
/// Cache-read tokens bill at the model's [`ModelPricing::cache_read_multiplier`]
/// of its input rate; the remaining `(prompt_tokens - cached_tokens)` bill at
/// the full input rate. Completion tokens always bill at the full output rate.
/// Reports no cache *writes* — use [`calculate_cost_for`] when the provider
/// broke those out. Returns
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
    calculate_cost_for(
        provider,
        model,
        &TokenCounts::new(prompt_tokens, completion_tokens).with_cache(cached_tokens, 0),
    )
}

/// Compute the USD cost from a resolved [`ModelPricing`] + token counts.
/// Shared by the compile-time lookup above and the [`PricingRegistry`]
/// override path so both sources price a call identically.
///
/// Four terms, because a prompt token can bill at three different rates:
/// fresh input, a cache read at a discount, and a cache write at a premium.
/// Reasoning tokens join completion on the output side.
fn cost_from_pricing(pricing: &ModelPricing, counts: &TokenCounts) -> f64 {
    let divisor = 1_000_000.0;
    let prompt = counts.prompt.max(0);
    // Reads and writes are carved out of the gross prompt in that order, so a
    // provider that over-reports one can never push the fresh remainder below
    // zero or bill a token twice.
    let cached_read = counts.cached_read.clamp(0, prompt);
    let cached_write = counts.cached_write.clamp(0, prompt - cached_read);
    let fresh_prompt = prompt - cached_read - cached_write;

    let input = pricing.input_per_million;
    let fresh_prompt_cost = fresh_prompt as f64 * input / divisor;
    let cached_read_cost = cached_read as f64 * input * pricing.cache_read_multiplier / divisor;
    let cached_write_cost = cached_write as f64 * input * pricing.cache_write_multiplier / divisor;
    // Providers that report reasoning tokens exclude them from the completion
    // count, so they are added rather than carved out.
    let output_tokens = counts.completion.max(0) + counts.reasoning.max(0);
    let output_cost = output_tokens as f64 * pricing.output_per_million / divisor;

    fresh_prompt_cost + cached_read_cost + cached_write_cost + output_cost
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
        counts: &TokenCounts,
    ) -> f64 {
        let Some(pricing) = self.resolve(tenant_id, provider, model) else {
            return zero_cost_for_unpriced(provider, model, tenant_id);
        };
        cost_from_pricing(&pricing, counts)
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
    calculate_cost_for(
        &record.provider,
        &record.model,
        &TokenCounts::new(record.prompt_tokens, record.completion_tokens)
            .with_cache(record.cached_tokens, record.cached_write_tokens)
            .with_reasoning(record.reasoning_tokens),
    )
}

/// Cost of a grouped usage row, crediting its summed cache reads and
/// charging its summed cache writes and reasoning tokens.
///
/// Counterpart to [`cost_for_record`] for the aggregate and daily series.
#[must_use]
pub fn cost_for_aggregate(row: &LlmUsageAggregateRow) -> f64 {
    calculate_cost_for(
        &row.provider,
        &row.model,
        &TokenCounts::new(row.prompt_tokens, row.completion_tokens)
            .with_cache(row.cached_tokens, row.cached_write_tokens)
            .with_reasoning(row.reasoning_tokens),
    )
}
