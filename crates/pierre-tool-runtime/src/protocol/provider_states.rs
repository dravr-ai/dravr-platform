// ABOUTME: Every provider a deployment offers with the athlete's connection state on each
// ABOUTME: The one list get_connection_status answers with and the no-provider answer for an external caller reads
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use pierre_core::errors::AppResult;
use pierre_core::models::{ConnectionStatus, TenantId};
use pierre_core::untrusted::display_line;
use pierre_database::RepositoryRegistry;
use pierre_providers::backend_resolver::{self, BackendKind, CoalescedStatus};
use pierre_providers::ProviderRegistry;
use pierre_services::delegated_connections::describe_link;
use tracing::warn;
use uuid::Uuid;

/// The user-facing providers this build actually ships, in name order.
///
/// Read off [`ProviderRegistry`], which registers each provider under its own
/// cargo feature, with every mirror backend (`sciotte`, `sciotte_garmin`,
/// `sciotte_trainingpeaks`) folded into the user-facing provider it serves
/// (`strava`, `garmin`, `trainingpeaks`). Folding rather than dropping matters
/// for TrainingPeaks: it has no backend of its own, so it is only ever named
/// through its mirror. Deriving it means a provider the binary cannot connect
/// can never be named: the hand-written list this replaced went on offering
/// `fitbit`, `terra` and `coros` to MCP clients long after the 2026-Q2 cleanup
/// left them out of `server-production` (carnet#233).
pub(crate) fn user_facing_providers(registry: &ProviderRegistry) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = registry
        .supported_providers()
        .into_iter()
        .map(backend_resolver::user_facing_name)
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// Whether the connection serving `user_facing` needs the athlete to reconnect.
///
/// Connection rows are stored under the BACKEND name (`sciotte_garmin`) while
/// the status tool reports the user-facing one (`garmin`), so a lookup by the
/// user-facing name never saw a mirror's dead session. When a backend serves
/// the provider, that backend's row decides — a dormant mirror row behind a
/// working Strava OAuth grant is not a reconnect. With nothing serving, any
/// serving backend's flagged row counts: a dead session whose token is gone is
/// still a reconnect, not a fresh connect.
fn serving_row_needs_reauth(
    status_by_provider: &HashMap<String, ConnectionStatus>,
    user_facing: &str,
    backend_kind: BackendKind,
) -> bool {
    let flagged = |backend: &str| {
        status_by_provider
            .get(backend)
            .is_some_and(ConnectionStatus::requires_reauth)
    };
    match backend_kind {
        BackendKind::Oauth => flagged(user_facing),
        BackendKind::Mirror => {
            backend_resolver::mirror_backend_for(user_facing).is_some_and(flagged)
        }
        // The session is the coach's, and nothing the athlete re-authorizes
        // renews it: the coach's own state is reported instead.
        BackendKind::Delegated => false,
        BackendKind::None => backend_resolver::serving_backends(user_facing)
            .iter()
            .any(|backend| flagged(backend)),
    }
}

/// Longest coach name the status reports for a delegated provider. The name
/// is the coach's own Dravr display name, so it reaches the model as one
/// defanged line.
const COACH_NAME_MAX_CHARS: usize = 60;

/// One provider's state, as both status shapes report it.
///
/// Without the connection tools only the no-provider answer reads it, and that
/// answer uses the status word alone.
#[cfg_attr(
    not(feature = "tools-connection"),
    expect(
        dead_code,
        reason = "the other fields are read by the connection tools"
    )
)]
pub(crate) struct ProviderState {
    pub(crate) connected: bool,
    pub(crate) status: &'static str,
    pub(crate) needs_reauth: bool,
    pub(crate) backend: BackendKind,
    pub(crate) delegated_by: Option<String>,
}

/// The athlete's lifecycle status per stored connection row, keyed by the
/// name the row is stored under (a mirror backend's own name included).
///
/// # Errors
///
/// Returns the repository's error when the connections cannot be read.
pub(crate) async fn read_status_by_provider(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
) -> AppResult<HashMap<String, ConnectionStatus>> {
    Ok(repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await?
        .into_iter()
        .map(|c| (c.provider, c.status))
        .collect())
}

/// Every provider this deployment offers, in name order, with the athlete's
/// state on each: the one list `get_connection_status` answers with and the
/// no-provider answer an external caller reads.
pub(crate) async fn offered_providers(
    repos: &RepositoryRegistry,
    registry: &ProviderRegistry,
    user_id: Uuid,
    tenant_id: TenantId,
    status_by_provider: &HashMap<String, ConnectionStatus>,
) -> Vec<(&'static str, ProviderState)> {
    let mut offered = Vec::new();
    for user_facing in user_facing_providers(registry) {
        let status =
            backend_resolver::coalesced_status(repos, user_id, tenant_id, user_facing).await;
        let state = provider_state(repos, status_by_provider, user_facing, status).await;
        offered.push((user_facing, state));
    }
    offered
}

/// The state of `provider`, which `status` coalesced, as the athlete reads it.
///
/// A provider read through the group coach's session (a delegated link)
/// names the coach, and reads `coach_reconnect_needed` while the coach's own
/// session needs the coach to sign in again: the athlete has nothing to
/// re-authorize, so `needs_reauth` stays false. A link whose roster athlete
/// is not the athlete by email reads its reason instead
/// (`athlete_email_missing`, `athlete_email_mismatch`,
/// `member_email_unverified`), as every read through it is refused until the
/// coach links the athlete again.
pub(crate) async fn provider_state(
    repos: &RepositoryRegistry,
    status_by_provider: &HashMap<String, ConnectionStatus>,
    provider: &str,
    status: CoalescedStatus,
) -> ProviderState {
    let needs_reauth = serving_row_needs_reauth(status_by_provider, provider, status.backend_kind);
    let (delegated_by, coach_needs_reconnect, read_refused) = match status.delegation {
        Some(link) => match describe_link(repos, link).await {
            Ok(Some(view)) => (
                Some(display_line(&view.coach_name, COACH_NAME_MAX_CHARS)),
                view.coach_needs_reconnect,
                view.read_refused,
            ),
            Ok(None) => (None, false, None),
            Err(e) => {
                warn!(error = %e, "Could not read the coach behind a delegated provider");
                (None, false, None)
            }
        },
        None => (None, false, None),
    };
    let word = if needs_reauth {
        "needs_reauth"
    } else if let Some(reason) = read_refused {
        reason
    } else if coach_needs_reconnect {
        "coach_reconnect_needed"
    } else if status.connected {
        "connected"
    } else {
        "disconnected"
    };
    ProviderState {
        connected: status.connected,
        status: word,
        needs_reauth,
        backend: status.backend_kind,
        delegated_by,
    }
}
