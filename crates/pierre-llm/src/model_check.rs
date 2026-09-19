// ABOUTME: The boot-time check that the active model is one its provider publishes.
// ABOUTME: A warning, not a refusal — the published list can lag the vendor's catalogue.
//
// SPDX-License-Identifier: Apache-2.0
// Copyright (c) 2026 dravr.ai

use tracing::{info, warn};

use crate::provider::ChatProvider;

/// Warn when the active model is not in the provider's published list.
///
/// A hint: embacle's list is a constant; ACP reports 28 models to its 21 (carnet#98).
pub fn validate_model_for_provider(provider: &ChatProvider) {
    let model = provider.default_model();
    let available = provider.available_models();

    if available.is_empty() {
        // Provider doesn't publish a model list — skip validation
        return;
    }

    if available.iter().any(|m| m == model) {
        info!(
            provider = provider.name(),
            model,
            available_count = available.len(),
            "Model validated against provider's available models"
        );
    } else {
        warn!(
            provider = provider.name(),
            model,
            available = ?available,
            "Model is not in this provider's published list — either that list is \
             stale, or PIERRE_LLM_MODEL holds the previous provider's id. Dispatch \
             errors naming this model mean the second"
        );
    }
}
