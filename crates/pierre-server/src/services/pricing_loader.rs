// ABOUTME: Startup hook that loads admin_config_overrides cat_llm_pricing rows into the global PricingRegistry
// ABOUTME: Runs once during ServerResources construction so per-call cost computation sees overrides immediately
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::admin::models::AdminConfigOverrideRow;
use pierre_database::repositories::LlmCredentialRepository;
use pierre_llm::pricing::{ModelPricing, PricingOverrideMap, GLOBAL_PRICING_REGISTRY};
use serde::Deserialize;
use std::collections::HashMap;
use tracing::{info, warn};

/// JSON shape that operators write into `admin_config_overrides.config_value`
/// for `category = 'cat_llm_pricing'`.
///
/// Keys mirror [`pierre_llm::pricing::ModelPricing`] but the JSON form is
/// a contract surface — operators edit it through the admin UI — so the
/// fields are renamed for clarity in the audit log.
#[derive(Debug, Deserialize)]
struct PricingOverridePayload {
    /// USD per 1 million input (prompt) tokens.
    input_per_million: f64,
    /// USD per 1 million output (completion) tokens.
    output_per_million: f64,
}

impl From<PricingOverridePayload> for ModelPricing {
    fn from(p: PricingOverridePayload) -> Self {
        Self {
            input_per_million: p.input_per_million,
            output_per_million: p.output_per_million,
        }
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
/// [`pierre_llm::pricing::GLOBAL_PRICING_REGISTRY`].
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
    let key = (provider.to_owned(), model_prefix.to_owned());
    Some((key, payload.into(), row.tenant_id.clone()))
}
