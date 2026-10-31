// ABOUTME: Which backend serves a connected provider card, judged by the rule routing reads by
// ABOUTME: A live usr/pwd session beside a Garmin grant serves the card; an empty grant awaits its first push

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The backend behind a connected card, and whether its OAuth grant still
//! awaits activities (carnet#1302).

use std::collections::HashSet;

use pierre_core::models::TenantId;
use pierre_providers::backend_resolver::{self, OAuthGrant};
use uuid::Uuid;

use crate::AuthRoutesContext;

/// The backend serving `provider_name`'s card for `user_id`, and the state of
/// its OAuth grant.
///
/// The row behind the card is taken in the order `resolve_backend` routes by
/// (the Strava card may be served by the native OAuth grant or the mirror
/// session). An OAuth row is then judged by the rule reads route by
/// ([`backend_resolver::oauth_grant`]): a live usr/pwd session beside a Garmin
/// grant serves the card whatever the grant holds, and with none the grant
/// serves, awaiting its first push while empty.
pub(crate) async fn card_backend(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Option<Uuid>,
    provider_name: &str,
    connected_providers: &HashSet<String>,
) -> (Option<String>, OAuthGrant) {
    let row_backend = backend_resolver::serving_backends(provider_name)
        .into_iter()
        .find(|backend| connected_providers.contains(backend.as_str()));
    let user_facing = backend_resolver::user_facing_name(provider_name);
    let grant = match tenant_id {
        Some(tid) if row_backend.as_deref() == Some(user_facing) => {
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
    let backend = if grant == OAuthGrant::DefersToMirror {
        backend_resolver::mirror_backend_for(user_facing).map(str::to_owned)
    } else {
        row_backend
    };
    (backend, grant)
}
