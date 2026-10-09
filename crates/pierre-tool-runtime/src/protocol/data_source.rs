// ABOUTME: Whether an athlete has any fitness data behind them: a connection, a provider token, or an uploaded file
// ABOUTME: The evidence the dispatch chokepoint reads before refusing a provider-backed tool; it fails open

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Display;
use std::sync::Arc;

use pierre_core::models::TenantId;
use pierre_services::onboarding_gate::user_has_connected_provider;
use tracing::warn;
use uuid::Uuid;

use crate::runtime::ToolRuntime;

/// Log a lookup that failed at the chokepoint; the caller then proceeds.
fn proceed_after(tool_name: &str, lookup: &str, error: &dyn Display) {
    warn!(
        tool_name = %tool_name,
        lookup = %lookup,
        error = %error,
        "data-source lookup failed at the chokepoint — proceeding; \
         the tool body re-resolves and refuses if the absence is real"
    );
}

/// Whether this athlete has any fitness data source behind them.
///
/// Answers `true` on the slightest evidence, and that bias is deliberate:
/// the caller refuses when this is `false`, so a wrong `false` locks a
/// working account out of every provider-backed tool. A lookup that fails
/// therefore answers `true` — the tool body re-resolves through the same
/// tables and refuses properly if the absence is real, whereas failing
/// closed would turn a transient database error into a refusal on a
/// connected athlete.
///
/// Three kinds of evidence, any one sufficient: a connection row, a provider
/// token (the two are written by different paths and are known to drift), and
/// a `.fit` file the athlete uploaded in `tenant` (carnet#818) — an athlete
/// who never connected a provider still has those workouts to answer from.
pub(super) async fn athlete_has_data_source(
    resources: &Arc<dyn ToolRuntime>,
    tool_name: &str,
    user_uuid: Uuid,
    tenant: Option<Uuid>,
) -> bool {
    let repos = resources.repos();
    match user_has_connected_provider(&repos.provider_connections, user_uuid).await {
        Ok(true) => return true,
        Ok(false) => {}
        Err(e) => {
            proceed_after(tool_name, "provider_connections", &e);
            return true;
        }
    }
    match repos.oauth_tokens.get_tokens(user_uuid, None).await {
        // Drift: a token without its connection row still means the athlete
        // has a real data source, so the tool body gets to run.
        Ok(tokens) if !tokens.is_empty() => return true,
        Ok(_) => {}
        Err(e) => {
            proceed_after(tool_name, "oauth_tokens", &e);
            return true;
        }
    }
    let Some(tenant) = tenant.map(TenantId::from_uuid) else {
        return false;
    };
    repos
        .uploaded_activity_files
        .has_uploaded_files(&tenant, user_uuid)
        .await
        .unwrap_or_else(|e| {
            proceed_after(tool_name, "uploaded_activity_files", &e);
            true
        })
}
