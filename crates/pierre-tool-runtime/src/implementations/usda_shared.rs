// ABOUTME: Process-wide shared USDA FoodData Central client for the nutrition and recipe tools
// ABOUTME: One client per process so its 24h response caches and rate limiter actually apply

//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Shared USDA client.
//!
//! `UsdaClient` carries per-instance 24-hour search/details caches and a
//! per-instance rate limiter. Constructing one per tool call — as
//! `validate_recipe` and the nutrition tools did — made both permanently
//! inert: every call started with cold caches (so every ingredient hit the
//! real USDA API every time) and a fresh rate limiter (so the limit never
//! bound anything across calls). The API key is process-level configuration
//! (`USDA_API_KEY`), never per-tenant, so one process-wide client is correct.

use std::sync::{Arc, OnceLock};

use pierre_external::usda_client::{UsdaClient, UsdaClientConfig};

/// Upper bound on caller-supplied ingredient arrays for the USDA fan-outs.
///
/// `validate_recipe` costs two API calls per ingredient,
/// `analyze_meal_nutrition` one. Generous for a real recipe or meal while
/// keeping the worst-case cold fan-out bounded by a constant instead of by
/// whatever array length a caller sends.
pub const MAX_USDA_INGREDIENTS: usize = 30;

static SHARED_USDA_CLIENT: OnceLock<Arc<UsdaClient>> = OnceLock::new();

/// The process-wide USDA client for `api_key`, built on first use.
///
/// The key comes from process environment configuration, so the first
/// caller's key is every caller's key; the `OnceLock` just makes the caches
/// and rate limiter live for the process instead of one call.
pub(crate) fn shared_usda_client(api_key: String) -> Arc<UsdaClient> {
    SHARED_USDA_CLIENT
        .get_or_init(|| {
            Arc::new(UsdaClient::new(UsdaClientConfig {
                api_key,
                ..UsdaClientConfig::default()
            }))
        })
        .clone()
}
