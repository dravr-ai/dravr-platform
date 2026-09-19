// ABOUTME: The platform's layer over embacle's price table: admin-editable overrides and usage-row adapters
// ABOUTME: GLOBAL_PRICING_REGISTRY is loaded from cat_llm_pricing at boot and read per call by the chat pipeline
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # LLM pricing: the tenant-aware registry
//!
//! The compile-time price table, the token-count arithmetic and the
//! cache-rate semantics live in [`embacle::pricing`], keyed on the name each
//! embacle runner reports. What is platform-shaped stays here:
//!
//! - [`PricingRegistry`] layers per-tenant and global operator overrides on
//!   top of the table. Overrides are fetched from `admin_config_overrides`
//!   under the `cat_llm_pricing` category by [`crate::pricing_loader`] and
//!   cached in-process; the table remains the fallback for models the
//!   operator has not repriced.
//! - [`cost_for_record`] and [`cost_for_aggregate`] price a stored
//!   `llm_usage` row, so every read path credits cache reads the same way.

use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};

use embacle::pricing::{
    calculate_cost_for, cost_from_pricing, lookup_pricing, zero_cost_for_unpriced, ModelPricing,
    TokenCounts,
};
use pierre_core::models::usage::{LlmUsageAggregateRow, LlmUsageRecord};

/// Process-wide [`PricingRegistry`] singleton.
///
/// The pierre-server startup hook reads `cat_llm_pricing` overrides from
/// `admin_config_overrides` once the database is ready and calls
/// [`PricingRegistry::replace_global`] on this instance. Per-call cost
/// computation in the chat pipeline reads back from the same singleton
/// so overrides take effect without restart.
pub static GLOBAL_PRICING_REGISTRY: LazyLock<PricingRegistry> = LazyLock::new(PricingRegistry::new);

/// Map keyed by `(provider, model_prefix)` to a single pricing entry.
pub type PricingOverrideMap = HashMap<(String, String), ModelPricing>;

/// Admin-editable layer over embacle's compile-time price table.
///
/// Overrides are stored in `admin_config_overrides` under the
/// `cat_llm_pricing` category and loaded into this registry at server
/// startup (and on config-change broadcast). Lookup order:
///
/// 1. Tenant-scoped override for `(provider, model_prefix)`
/// 2. Global override for `(provider, model_prefix)`
/// 3. Compile-time [`embacle::pricing::PRICING_TABLE`] entry
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
    /// should use [`embacle::pricing::calculate_cost_with_cache`] directly.
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
