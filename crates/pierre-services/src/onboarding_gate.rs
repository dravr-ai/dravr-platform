// ABOUTME: Single source of truth for "user has connected at least one fitness provider" check
// ABOUTME: Read by the MCP tool-dispatch chokepoint, the messaging connect card, and GET /api/me/onboarding-status
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Onboarding gate
//!
//! One answer to "does this user have a connected fitness provider", read from
//! `provider_connections` by everything that needs it.
//!
//! ## It is no longer a gate on conversation
//!
//! It was, and the reason was sound: a newly onboarded user has no activity
//! data, and letting them chat produced hallucinated specifics — *"nice 12 km
//! ride yesterday!"* — because the model cannot distinguish "no recent
//! activity" from "no connected provider at all". The response was to refuse
//! the conversation.
//!
//! That treated the symptom. The model invented a past because nothing told it
//! there was none, so Phase 5 tells it: `build_provider_context` states the
//! absence in the system prompt, the athlete-data verifier contradicts any
//! specific figure asserted with no source behind it, and the tool-dispatch
//! chokepoint refuses every `REQUIRES_PROVIDER` tool. A providerless athlete
//! now gets a coach that says what it cannot see, instead of a closed door.
//!
//! ## Who reads it now
//!
//! - `UniversalExecutor::provider_refusal` — refuses provider-requiring tools
//!   at dispatch. This is the barrier that replaced the conversation gate.
//! - `messaging_ingress::dispatch::maybe_send_connect_card` — rides the served
//!   reply with a tappable connect card while the athlete has none.
//! - `GET /api/me/onboarding-status` — drives the first-login redirect, via
//!   [`user_has_real_provider`], which excludes synthetic seed connections.
//!
//! The check is cross-tenant on purpose: a connection registered under any
//! tenant the user belongs to counts. Per-tenant tool gating remains the
//! responsibility of the tool layer.

use std::sync::Arc;

use pierre_core::errors::AppResult;
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
