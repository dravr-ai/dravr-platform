// ABOUTME: Reads what kind of TrainingPeaks account a stored session signed in with, off the request that stored it
// ABOUTME: A coach account gets no prefetch, since it keeps no calendar; an athlete account's activities are prefetched
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Every `TrainingPeaks` login — the web and mobile modals, the hosted page,
//! the reconnect link — lands in the session store, and every one of them is
//! probed here for the account's role
//! ([`probe_trainingpeaks_role`](pierre_services::trainingpeaks_accounts::probe_trainingpeaks_role)).
//! The probe is one profile read on the scraper service, so it runs in a task
//! of its own and the login answers as soon as the session is stored.

use std::sync::Arc;

use dravr_sciotte::models::AuthSession;
use pierre_core::constants::oauth_providers::SCIOTTE_TRAININGPEAKS;
use pierre_core::models::{ConnectionType, ProviderAccountRole, TenantId};
use pierre_services::trainingpeaks_accounts::probe_trainingpeaks_role;
use tracing::{info, warn};
use uuid::Uuid;

use pierre_providers::connect_prefetch::PrefetchWait;

use crate::sciotte_prefetch::{
    begin_prefetch, prefetch_activities, prefetch_in_flight, PrefetchTarget,
};
use crate::AuthRoutesContext;

/// After a `TrainingPeaks` login stored `session`, read and record the
/// account's role, then warm the activity cache the way every other login
/// does — unless the account is a coach's, which has no calendar of its own
/// and whose prefetch could only be refused.
///
/// A probe that cannot read the profile leaves the role unknown and still
/// prefetches: the read path records a coach account from the scraper's
/// refusal.
///
/// The pre-fetch is registered before the probe starts, so a list read that
/// arrives meanwhile waits for both instead of scraping the session beside
/// them; the returned wait is on it, or on the pre-fetch already in flight
/// when there is one.
pub fn spawn_login_probe(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Uuid,
    session: &AuthSession,
    session_json: &str,
) -> Option<PrefetchWait> {
    let ticket = begin_prefetch(user_id, tenant_id, SCIOTTE_TRAININGPEAKS);
    let wait = ticket.as_ref().map_or_else(
        || prefetch_in_flight(user_id, tenant_id, SCIOTTE_TRAININGPEAKS),
        |ticket| Some(ticket.wait()),
    );
    let repos = Arc::clone(&resources.repos);
    let registry = Arc::clone(&resources.provider_registry);
    let cache = Arc::clone(&resources.cache);
    let session = session.clone();
    let session_json = session_json.to_owned();
    tokio::spawn(async move {
        let tenant = TenantId::from_uuid(tenant_id);
        match probe_trainingpeaks_role(&repos, &session, user_id, tenant).await {
            Ok(ProviderAccountRole::Coach) => {
                info!(
                    user_id = %user_id,
                    "TrainingPeaks login is a coach account: no calendar of its own to prefetch"
                );
                return;
            }
            Ok(ProviderAccountRole::Athlete) => {}
            Err(e) => warn!(
                user_id = %user_id,
                error = %e,
                "TrainingPeaks account probe failed; prefetching with the role unknown"
            ),
        }
        // A pre-fetch already in flight for the connection is the one its
        // readers wait on; this login's probe still reads its own account.
        let Some(ticket) = ticket else {
            return;
        };
        prefetch_activities(
            registry,
            cache,
            PrefetchTarget {
                user_id,
                tenant_id,
                provider_name: SCIOTTE_TRAININGPEAKS.to_owned(),
                session_json,
            },
            ticket,
        )
        .await;
    });
    wait
}

/// Probe the stored `session` of a `TrainingPeaks` connection whose role was
/// never read — one made before roles were recorded — when a login reuses it
/// instead of signing in again.
///
/// A connection whose role is known is left alone, and so is a connection
/// the user reads through someone else's account.
pub fn spawn_role_probe_if_unread(
    resources: &AuthRoutesContext,
    user_id: Uuid,
    tenant_id: Uuid,
    session: AuthSession,
) {
    let repos = Arc::clone(&resources.repos);
    tokio::spawn(async move {
        let tenant = TenantId::from_uuid(tenant_id);
        let unread = match repos
            .provider_connections
            .get_for_user(user_id, Some(tenant))
            .await
        {
            Ok(connections) => connections.iter().any(|connection| {
                connection.provider == SCIOTTE_TRAININGPEAKS
                    && connection.connection_type != ConnectionType::Delegated
                    && connection.account_role.is_none()
            }),
            Err(e) => {
                warn!(
                    user_id = %user_id,
                    error = %e,
                    "Could not read the TrainingPeaks connection to decide on a role probe"
                );
                false
            }
        };
        if !unread {
            return;
        }
        match probe_trainingpeaks_role(&repos, &session, user_id, tenant).await {
            Ok(role) => info!(
                user_id = %user_id,
                role = %role,
                "TrainingPeaks role read for a connection made before roles were recorded"
            ),
            Err(e) => warn!(
                user_id = %user_id,
                error = %e,
                "TrainingPeaks account probe failed on a reused session"
            ),
        }
    });
}
