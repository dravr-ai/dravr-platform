// ABOUTME: Per-user context stage — renders the Dossier's OKF bundle into the system prompt
// ABOUTME: The single fact->prompt surface; composes the read-time Dossier then renders OKF markdown
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Per-user pillar-context injection.
//!
//! Composes the read-time [`Dossier`](pierre_core::models::Dossier) for the
//! current (tenant, user) and renders its North Star + pillar + medical facts
//! as an OKF markdown bundle appended to the system prompt. This is the only
//! place stored [`UserFact`](pierre_memory::UserFact)s become prompt text.
//!
//! Complementary to [`pierre_services::memory_extraction`], which runs after
//! the turn completes and distills new facts from the exchange.

use uuid::Uuid;

use pierre_core::models::TenantId;
use pierre_database::repositories::DossierRepository;
use pierre_services::okf::render_okf_bundle_default;

/// Append the per-user OKF context bundle to the system prompt.
///
/// Composes the dossier for the given (tenant, user) and renders its pillar
/// context. Errors and empty context both pass through silently — the bundle
/// is a best-effort enhancement, not a hard dependency of the dispatch path.
pub async fn inject_okf_bundle(
    dossier_repo: &dyn DossierRepository,
    tenant_id: TenantId,
    user_id: Uuid,
    base_prompt: String,
) -> String {
    match dossier_repo.compose_dossier(tenant_id, user_id).await {
        Ok(dossier) => match render_okf_bundle_default(&dossier) {
            Some(block) => format!("{base_prompt}{block}"),
            None => base_prompt,
        },
        Err(e) => {
            tracing::warn!(error = %e, "okf bundle compose failed; continuing without pillar context");
            base_prompt
        }
    }
}
