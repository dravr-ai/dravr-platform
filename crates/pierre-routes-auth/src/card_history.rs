// ABOUTME: Where a provider card's history stands, and the export source the card offers to bring it in
// ABOUTME: The state is pierre_services::provider_history's rule, the one the chat reads too (carnet#1623)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A card's history (carnet#1386, carnet#1494).
//!
//! Where a push-fed grant's history stands is
//! [`provider_history::grant_history`]'s rule, which the lead's first welcome
//! and the agent's prompt read as well. The card adds what only this server's
//! routes know: the provider's own export source among those it offers.

use pierre_auth::dto::auth::HistoryStatus;
use pierre_core::constants::oauth::providers::companion_keys;
use pierre_core::models::TenantId;
use pierre_providers::backend_resolver;
use pierre_services::provider_history;
use uuid::Uuid;

use crate::AuthRoutesContext;

/// The history of `card` for `user_id`, whose own grant serves it first when
/// `grant_serves` ([`provider_history::grant_serves`]).
///
/// The state and the sign-in are [`provider_history::grant_history`]'s.
/// `archive_source` is the provider's own export source among those this
/// server offers, whatever the state.
pub(crate) async fn card_history(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
    card: &str,
    grant_serves: bool,
) -> HistoryStatus {
    let archive_source = companion_keys(backend_resolver::user_facing_name(card))
        .iter()
        .find(|key| resources.offered_archive_sources.contains(key))
        .map(|key| (*key).to_owned());
    let history = provider_history::grant_history(
        &resources.repos,
        user_id,
        tenant_id.map(TenantId::from_uuid),
        card,
        grant_serves,
    )
    .await;
    HistoryStatus {
        state: history.state,
        sign_in_target: history.sign_in_target.map(str::to_owned),
        archive_source,
    }
}
