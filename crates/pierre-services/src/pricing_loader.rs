// ABOUTME: Startup hook that loads admin_config_overrides cat_llm_pricing rows into the global PricingRegistry
// ABOUTME: Runs once during ServerContext construction so per-call cost computation sees overrides immediately
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::pricing::{PricingOverrideMap, GLOBAL_PRICING_REGISTRY};
use embacle::pricing::{lookup_pricing, LongPromptRates, ModelPricing};
use pierre_core::admin::models::AdminConfigOverrideRow;
use pierre_database::repositories::LlmCredentialRepository;
use serde::Deserialize;
use std::collections::HashMap;
use tracing::{info, warn};

/// JSON shape that operators write into `admin_config_overrides.config_value`
/// for `category = 'cat_llm_pricing'`.
///
/// Keys mirror [`embacle::pricing::ModelPricing`] but the JSON form is
/// a contract surface — operators edit it through the admin UI — so the
/// fields are renamed for clarity in the audit log.
#[derive(Debug, Deserialize)]
struct PricingOverridePayload {
    /// USD per 1 million input (prompt) tokens.
    input_per_million: f64,
    /// USD per 1 million output (completion) tokens.
    output_per_million: f64,
    /// Fraction of the input rate at which a cache *read* bills. Omitted by
    /// every override written before the multipliers existed, so it falls
    /// back to the compile-time default rather than repricing to zero.
    cache_read_multiplier: Option<f64>,
    /// Fraction of the input rate at which a cache *write* bills. On
    /// Anthropic this is a premium (1.25), not a discount.
    cache_write_multiplier: Option<f64>,
    /// The second rate card above a prompt-length threshold
    /// (`{"above_prompt_tokens", "input_per_million", "output_per_million"}`).
    /// Omitted, the compile-time table's card for the same row carries over:
    /// an operator repricing Haiku 5.5's base rates must not silently bill
    /// its prompts over 100K tokens at the base card.
    long_prompt: Option<LongPromptRates>,
}

impl PricingOverridePayload {
    /// The pricing this override sets for `(provider, model_prefix)`.
    ///
    /// Every rate the payload names wins. A cache multiplier it omits is the
    /// compile-time default; a long-prompt card it omits is the one the
    /// compile-time table holds for that row, when it holds one.
    fn into_pricing(self, provider: &str, model_prefix: &str) -> ModelPricing {
        let base = ModelPricing::new(self.input_per_million, self.output_per_million);
        let pricing = base.with_cache_rates(
            self.cache_read_multiplier
                .unwrap_or(base.cache_read_multiplier),
            self.cache_write_multiplier
                .unwrap_or(base.cache_write_multiplier),
        );
        let long_prompt = self
            .long_prompt
            .or_else(|| lookup_pricing(provider, model_prefix).and_then(|table| table.long_prompt));
        long_prompt.map_or(pricing, |long| {
            pricing.with_long_prompt_rates(
                long.above_prompt_tokens,
                long.input_per_million,
                long.output_per_million,
            )
        })
    }
}

/// Bucketed result of parsing a batch of admin override rows. The maps
/// drop straight into the registry's `replace_global` / `replace_tenant`
/// surfaces; the loader entry point handles only the IO + reporting.
struct OverrideBuckets {
    global: PricingOverrideMap,
    per_tenant: HashMap<String, PricingOverrideMap>,
}

/// Load every `cat_llm_pricing` override row into the process-wide
/// [`crate::pricing::GLOBAL_PRICING_REGISTRY`].
///
/// Override `config_key` is `<provider>.<model_prefix>` (e.g.
/// `gemini.gemini-2.0-flash`). Tenant-scoped rows go into a per-tenant
/// override map; null-tenant rows go into the global map. Failures parse
/// as warnings — a malformed override never blocks server startup or
/// degrades into a free-tier shell, the compile-time `PRICING_TABLE`
/// remains the safe fallback.
pub async fn load_pricing_overrides(repo: &dyn LlmCredentialRepository) {
    let rows = match repo
        .list_admin_config_overrides_by_category("cat_llm_pricing")
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!("Failed to read cat_llm_pricing overrides at startup: {e}");
            return;
        }
    };

    let buckets = bucket_overrides(rows);
    let global_count = buckets.global.len();
    let tenant_count = buckets.per_tenant.len();

    GLOBAL_PRICING_REGISTRY.replace_global(buckets.global);
    for (tid, map) in buckets.per_tenant {
        GLOBAL_PRICING_REGISTRY.replace_tenant(tid, map);
    }

    info!(
        global_overrides = global_count,
        tenants_with_overrides = tenant_count,
        "Loaded cat_llm_pricing overrides into GLOBAL_PRICING_REGISTRY"
    );
}

/// Parse a batch of admin override rows into global + per-tenant maps.
///
/// Each malformed row (bad `config_key` shape, bad `config_value` JSON)
/// is logged and skipped — never fatal — so a single bad operator edit
/// can't take the server out at boot.
fn bucket_overrides(rows: Vec<AdminConfigOverrideRow>) -> OverrideBuckets {
    let mut buckets = OverrideBuckets {
        global: HashMap::new(),
        per_tenant: HashMap::new(),
    };
    for row in rows {
        if let Some((key, pricing, tenant_id)) = parse_override_row(&row) {
            match tenant_id {
                Some(tid) => {
                    buckets
                        .per_tenant
                        .entry(tid)
                        .or_default()
                        .insert(key, pricing);
                }
                None => {
                    buckets.global.insert(key, pricing);
                }
            }
        }
    }
    buckets
}

/// Parse a single override row into the registry's key/value/scope tuple.
/// Returns `None` when the row is malformed (with a warning logged).
fn parse_override_row(
    row: &AdminConfigOverrideRow,
) -> Option<((String, String), ModelPricing, Option<String>)> {
    let Some((provider, model_prefix)) = row.config_key.split_once('.') else {
        warn!(
            config_key = %row.config_key,
            "Skipping cat_llm_pricing override with malformed config_key (expected '<provider>.<model_prefix>')"
        );
        return None;
    };
    let payload: PricingOverridePayload = match serde_json::from_str(&row.config_value) {
        Ok(p) => p,
        Err(e) => {
            warn!(
                config_key = %row.config_key,
                "Skipping cat_llm_pricing override with malformed JSON: {e}"
            );
            return None;
        }
    };
    let pricing = payload.into_pricing(provider, model_prefix);
    let key = (provider.to_owned(), model_prefix.to_owned());
    Some((key, pricing, row.tenant_id.clone()))
}

#[cfg(test)]
mod tests {
    use super::PricingOverridePayload;

    fn payload(json: &str) -> PricingOverridePayload {
        serde_json::from_str(json).expect("a valid override payload")
    }

    #[test]
    fn an_override_without_a_long_prompt_card_keeps_the_tables() {
        let pricing = payload(r#"{"input_per_million": 0.12, "output_per_million": 0.60}"#)
            .into_pricing("anthropic", "claude-haiku-5-5");
        assert!((pricing.input_per_million - 0.12).abs() < f64::EPSILON);
        let long = pricing
            .long_prompt
            .expect("Haiku 5.5's 100K card must survive a base-rate override");
        assert_eq!(long.above_prompt_tokens, 100_000);
        assert!((long.input_per_million - 0.50).abs() < f64::EPSILON);
        assert!((long.output_per_million - 2.50).abs() < f64::EPSILON);
    }

    #[test]
    fn an_override_naming_a_long_prompt_card_wins() {
        let pricing = payload(
            r#"{"input_per_million": 0.10, "output_per_million": 0.50,
                "long_prompt": {"above_prompt_tokens": 200000, "input_per_million": 0.40, "output_per_million": 2.0}}"#,
        )
        .into_pricing("anthropic", "claude-haiku-5-5");
        let long = pricing.long_prompt.expect("the payload's card");
        assert_eq!(long.above_prompt_tokens, 200_000);
        assert!((long.input_per_million - 0.40).abs() < f64::EPSILON);
    }

    #[test]
    fn a_row_with_no_long_prompt_card_gets_none() {
        let pricing = payload(r#"{"input_per_million": 2.0, "output_per_million": 10.0}"#)
            .into_pricing("anthropic", "claude-sonnet-5-5");
        assert!(pricing.long_prompt.is_none());
        let unknown = payload(r#"{"input_per_million": 1.0, "output_per_million": 1.0}"#)
            .into_pricing("nobody", "no-model");
        assert!(unknown.long_prompt.is_none());
    }

    #[test]
    fn an_older_payload_without_multipliers_still_parses() {
        let pricing = payload(r#"{"input_per_million": 0.999, "output_per_million": 9.99}"#)
            .into_pricing("gemini", "gemini-2.0-flash");
        assert!((pricing.output_per_million - 9.99).abs() < f64::EPSILON);
    }
}
