// ABOUTME: Single source of truth for "user has connected at least one fitness provider" check
// ABOUTME: Used by REST gate on chat/coach, MCP tool execution gate, and the GET /api/me/onboarding-status endpoint
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Onboarding gate
//!
//! A newly onboarded user has no entries in `provider_connections` and therefore
//! no activity data the LLM coach can reason about. Letting them chat anyway
//! produced hallucinated specifics ("nice 12 km ride yesterday!") because the
//! LLM has no way to distinguish "no recent activity" from "no connected
//! provider at all". This module centralizes the check so the REST layer, the
//! MCP tool layer, and the onboarding-status endpoint all answer the same
//! question from the same source — the `provider_connections` table.
//!
//! ## Policy
//!
//! - Zero rows in `provider_connections` for the user ⇒ user must onboard
//!   (i.e., connect a provider) before they can use messaging features.
//! - Any row ⇒ proceed.
//!
//! The check is cross-tenant on purpose: a connection registered under any
//! tenant the user belongs to is enough to unblock messaging. Per-tenant
//! tool gating remains the responsibility of the tool layer.

use std::sync::Arc;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::ConnectionType;
use pierre_database::repositories::ProviderConnectionRepository;
use uuid::Uuid;

/// Returns `true` when the user has at least one row in `provider_connections`
/// — including `Synthetic` (seed/demo) connections.
///
/// This is the **coach-access** gate: synthetic connections carry seeded
/// activity data the LLM coach can legitimately discuss, so a demo user with
/// only synthetic data is allowed through (mirrors how the OAuth callback
/// unblocks chat). For the *connect-a-real-provider* onboarding decision, use
/// [`user_has_real_provider`] instead.
///
/// # Errors
///
/// Propagates the repository error if the query against `provider_connections`
/// fails.
pub async fn user_has_connected_provider(
    repo: &Arc<dyn ProviderConnectionRepository>,
    user_id: Uuid,
) -> AppResult<bool> {
    let connections = repo.get_for_user(user_id, None).await?;
    Ok(!connections.is_empty())
}

/// Returns `true` when the user has at least one *real* (non-`Synthetic`)
/// provider connection.
///
/// Drives the first-login connect-provider redirect (`GET
/// /api/me/onboarding-status`). The synthetic-activities seeder registers a
/// `provider = "synthetic"` row for test/demo users; that's seeded data, not a
/// real provider link, so it must NOT suppress the redirect — a user whose only
/// connection is synthetic still needs to connect a real provider. Distinct from
/// [`user_has_connected_provider`] (the coach-access gate), which counts
/// synthetic so demos can chat about their seeded data.
///
/// # Errors
///
/// Propagates the repository error if the query against `provider_connections`
/// fails.
pub async fn user_has_real_provider(
    repo: &Arc<dyn ProviderConnectionRepository>,
    user_id: Uuid,
) -> AppResult<bool> {
    let connections = repo.get_for_user(user_id, None).await?;
    Ok(connections
        .iter()
        .any(|c| c.connection_type != ConnectionType::Synthetic))
}

/// Returns `Ok(())` when the user has at least one connected provider and
/// [`AppError::no_provider_connected`] otherwise.
///
/// Call this at every entry point that hands data to the LLM coach: HTTP chat,
/// coach activation, MCP tool execution. The error code is `NoProviderConnected`
/// (403) with `details.action = "connect_provider"` so frontends can redirect
/// to the onboarding flow without string-matching the message.
///
/// # Errors
///
/// - [`AppError::no_provider_connected`] — user has zero rows in
///   `provider_connections`.
/// - Repository error if the underlying query fails.
pub async fn require_connected_provider(
    repo: &Arc<dyn ProviderConnectionRepository>,
    user_id: Uuid,
) -> AppResult<()> {
    if user_has_connected_provider(repo, user_id).await? {
        Ok(())
    } else {
        Err(AppError::no_provider_connected())
    }
}
