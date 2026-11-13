// ABOUTME: Where a provider card's history stands, read from the rows held under its history keys
// ABOUTME: A push-fed grant serving alone reads imported once its sign-in's or an export's rows are held

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A card's history (carnet#1386, carnet#1494).
//!
//! A push-fed grant (Garmin) delivers only what the athlete records after
//! connecting, so its card says whether what came before is in: rows are
//! held under the sign-in's key or an imported archive's
//! ([`HistoryState::Imported`]), or they are not ([`HistoryState::Needed`]).
//! A sign-in makes the session the card's primary source, whose own reads
//! bring the history in. Nothing new is stored for it: the rows under the
//! history keys say it.

use std::iter;

use pierre_auth::dto::auth::{HistoryState, HistoryStatus};
use pierre_core::constants::oauth::providers::companion_keys;
use pierre_core::models::TenantId;
use pierre_providers::backend_resolver;
use uuid::Uuid;

use crate::AuthRoutesContext;

/// The history of `card` for `user_id`, whose own grant serves it first when
/// `grant_serves` ([`crate::card_backend::grant_serves`]).
///
/// Rows held under the sign-in's key or an imported archive's read
/// `imported`: an athlete whose session the one-time import retired keeps
/// what it stored. Otherwise `needed`: rows the grant pushed itself start at
/// the day it connected, so they never stand for what came before it.
///
/// A card whose provider is not push-fed, or that no grant of the user's
/// serves first (a session beside it reads its own history), reads
/// `not_applicable`. `archive_source` is the provider's own
/// export source among those this server offers, whatever the state.
pub(crate) async fn card_history(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
    card: &str,
    grant_serves: bool,
) -> HistoryStatus {
    let user_facing = backend_resolver::user_facing_name(card);
    let archive_source = companion_keys(user_facing)
        .iter()
        .find(|key| resources.offered_archive_sources.contains(key))
        .map(|key| (*key).to_owned());
    let sign_in = backend_resolver::mirror_backend_for(user_facing);
    let (Some(tenant), Some(sign_in), true) = (
        tenant_id.map(TenantId::from_uuid),
        sign_in,
        grant_serves && backend_resolver::pushed_backend_for(user_facing) == Some(user_facing),
    ) else {
        return HistoryStatus {
            state: HistoryState::NotApplicable,
            sign_in_target: None,
            archive_source,
        };
    };
    let state = history_state(resources, user_id, tenant, sign_in, user_facing).await;
    HistoryStatus {
        state,
        sign_in_target: Some(sign_in.to_owned()),
        archive_source,
    }
}

/// [`card_history`]'s state for a card a push-fed grant serves: `imported`
/// when rows are held under the sign-in's key or an imported archive's, else
/// `needed`.
async fn history_state(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant: TenantId,
    sign_in: &str,
    grant_key: &str,
) -> HistoryState {
    let history_keys = iter::once(sign_in).chain(companion_keys(grant_key).iter().copied());
    for key in history_keys {
        if backend_resolver::holds_activities(&resources.repos, user_id, tenant, key).await {
            return HistoryState::Imported;
        }
    }
    HistoryState::Needed
}
