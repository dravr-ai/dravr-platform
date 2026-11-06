// ABOUTME: Which backend serves a connected provider card, judged by the rule routing reads by
// ABOUTME: A Garmin grant serves the card whatever session stands beside it; an empty grant awaits its history

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The backend behind a connected card, and whether its OAuth grant still
//! awaits activities (carnet#1302, carnet#1386).

use std::collections::HashSet;

use pierre_core::models::TenantId;
use pierre_providers::backend_resolver::{self, OAuthGrant};
use uuid::Uuid;

use crate::AuthRoutesContext;

/// The backend serving `provider_name`'s card for `user_id`, and the state of
/// its OAuth grant.
///
/// The row behind the card is taken in the order `resolve_backend` routes by
/// (the Strava and Garmin cards are served by the OAuth grant whenever its row
/// exists, else by the mirror session). An OAuth row is then judged by the
/// rule reads route by ([`backend_resolver::oauth_grant`]): the grant serves,
/// awaiting the athlete's history while no activity is held for it.
pub(crate) async fn card_backend(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
    provider_name: &str,
    connected_providers: &HashSet<String>,
) -> (Option<String>, OAuthGrant) {
    let backend = backend_resolver::serving_backends(provider_name)
        .into_iter()
        .find(|backend| connected_providers.contains(backend.as_str()));
    let user_facing = backend_resolver::user_facing_name(provider_name);
    let grant = match tenant_id {
        Some(tid) if backend.as_deref() == Some(user_facing) => {
            backend_resolver::oauth_grant(
                &resources.repos,
                user_id,
                TenantId::from_uuid(tid),
                user_facing,
            )
            .await
        }
        _ => OAuthGrant::Absent,
    };
    (backend, grant)
}

/// Whether the card asks for a reconnect, given `connected_backend`, the
/// backend serving it, and `reauth_providers`, the backends flagged for
/// reconnect.
///
/// Only the serving backend counts: its session died (a dead scrape, a
/// refused OAuth refresh), so the UI prompts a reconnect instead of a healthy
/// "Connected". A usr/pwd session left beside a serving grant serves nothing,
/// so its flag asks nothing of the athlete (carnet#1386): the card reads
/// "Reconnect" only when the grant itself is flagged.
pub(crate) fn reauth_flag(
    connected_backend: Option<&str>,
    reauth_providers: &HashSet<String>,
) -> bool {
    connected_backend.is_some_and(|backend| reauth_providers.contains(backend))
}
