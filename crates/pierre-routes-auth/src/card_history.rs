// ABOUTME: Where a provider card's history stands, read from the backfill ledger and the rows under its history keys
// ABOUTME: And the settling of a usr/pwd session a new Garmin grant leaves serving nothing (carnet#1386)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A card's history (carnet#1386).
//!
//! A push-fed grant (Garmin) delivers only what the athlete records after
//! connecting, so its card says whether what came before is in: a one-time
//! import of a usr/pwd sign-in runs ([`HistoryState::Importing`]), ended
//! before its history landed ([`HistoryState::Incomplete`]), or rows are held
//! ([`HistoryState::Imported`]). Nothing new is stored for it: the backfill
//! ledger's row for the session and the rows under the history keys say it.

use std::iter;

use pierre_auth::dto::auth::{HistoryState, HistoryStatus};
use pierre_core::constants::oauth::providers::companion_keys;
use pierre_core::models::TenantId;
use pierre_database::repositories::BackfillJobState;
use pierre_mcp_transport::OAuthCallbackResponse;
use pierre_providers::backend_resolver;
use tracing::warn;
use uuid::Uuid;

use crate::AuthRoutesContext;

/// The history of `card` for `user_id`, whose own grant serves it first when
/// `grant_serves` ([`crate::card_backend::grant_serves`]).
///
/// Read in this order, the first that holds:
/// 1. a one-time import of the card's sign-in is owed: `importing`;
/// 2. rows are held under the sign-in's key or an imported archive's:
///    `imported` — a usr/pwd athlete who moved to the grant keeps what their
///    session stored, whatever a later top-up did;
/// 3. the last import ended before its history landed: `incomplete`;
/// 4. otherwise `needed` — rows the grant pushed itself start at the day it
///    connected, so they never stand for what came before it.
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

/// [`card_history`]'s state for a card a push-fed grant serves.
async fn history_state(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant: TenantId,
    sign_in: &str,
    grant_key: &str,
) -> HistoryState {
    let job = match resources
        .repos
        .activity_backfill_jobs
        .backfill_job_state(user_id, tenant, sign_in)
        .await
    {
        Ok(job) => job,
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "Card history: the import ledger could not be read");
            None
        }
    };
    if job == Some(BackfillJobState::Owed) {
        return HistoryState::Importing;
    }
    let history_keys = iter::once(sign_in).chain(companion_keys(grant_key).iter().copied());
    for key in history_keys {
        if backend_resolver::holds_activities(&resources.repos, user_id, tenant, key).await {
            return HistoryState::Imported;
        }
    }
    if job == Some(BackfillJobState::Incomplete) {
        return HistoryState::Incomplete;
    }
    HistoryState::Needed
}

/// Settle the usr/pwd session standing beside the grant the user just
/// connected over OAuth (`connected`), in the tenant the callback stored it
/// under: one import then retirement when it is live, retirement at once
/// when it is not (`HistoryImports::grant_connected`). Runs detached; the
/// callback answers at once. A provider that is not push-fed, and a build
/// with no import rail, settle nothing.
pub(crate) fn spawn_grant_history_settlement(
    resources: &AuthRoutesContext,
    connected: &OAuthCallbackResponse,
) {
    let provider = connected.provider.as_str();
    if backend_resolver::pushed_backend_for(provider) != Some(provider) {
        return;
    }
    let Some(imports) = resources.history_imports.clone() else {
        return;
    };
    let (Ok(user), Ok(tenant)) = (
        Uuid::parse_str(&connected.user_id),
        TenantId::parse_str(&connected.tenant_id),
    ) else {
        warn!(
            user_id = %connected.user_id,
            provider,
            "The new grant's user or tenant is not a UUID; its history is left unsettled"
        );
        return;
    };
    let grant = provider.to_owned();
    tokio::spawn(async move {
        imports.grant_connected(user, tenant, &grant).await;
    });
}
