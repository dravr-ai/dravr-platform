// ABOUTME: A push-only provider's activity reads: the window its pushes stored, never a live read or a write-through
// ABOUTME: The decision keys on the descriptor's PUSH_ONLY flag and on the backend the user's reads resolve to
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Push-only providers in the live-read paths (carnet#1155).
//!
//! A provider declaring
//! [`PUSH_ONLY`](pierre_providers::spi::ProviderCapabilities::PUSH_ONLY) has no list to
//! read: every activity it delivers was pushed to Dravr and cached when it
//! arrived. The head reads ([`super::fetch_provider_head`],
//! [`super::read_live_window`]) serve such a provider's cached window
//! instead, and the stale-head top-up ([`super::refresh_stale_head`]) skips
//! it, so its freshness stays the last push the ingest recorded.
//!
//! The window also serves the history a usr/pwd session of the same provider
//! stored under its mirror's key (`sciotte_garmin` beside `garmin`): the
//! grant is the one ongoing source (carnet#1386), so what the session read
//! before it stopped keeps being served after it dies, merged with the pushes
//! so one workout recorded by both is one session.

use std::sync::Arc;

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, ProviderConnection, TenantId};
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::{ai_scope, backend_resolver};
use uuid::Uuid;

use super::sync_verdict::{HeadRead, HeadVerdict};
use super::{cached_history, cached_window_bounds, STALE_FALLBACK_LIMIT};
use crate::runtime::ToolRuntime;

/// Whether this user's reads of `provider_slug` are served from what a
/// push-only provider pushed: the slug is push-only AND it is the backend
/// the user's reads of it resolve to.
///
/// The resolution matters because a caller may name the user-facing
/// provider: `garmin` resolves to the Garmin API backend for an athlete who
/// holds its grant, whatever credential connection (`sciotte_garmin`) stands
/// beside it, and to that connection, read on request, for an athlete who
/// holds no grant (carnet#1386, `backend_resolver::oauth_grant`). The
/// resolver is consulted only for a push-only slug, so no other provider's
/// read pays for it.
pub(super) async fn reads_pushed_store(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant: Option<TenantId>,
) -> bool {
    if !runtime.provider_registry().is_push_only(provider_slug) {
        return false;
    }
    backend_resolver::resolve_backend(runtime.repos(), user_id, tenant, provider_slug).await
        == provider_slug
}

/// Whether `connection` is a usr/pwd session beside a grant that serves its
/// provider (`backend_resolver::session_beside_grant`).
///
/// Judged in the tenant the connection lives in. Such a session serves nothing (carnet#1386), so
/// no walk of the athlete's connections reads it: the grant's own connection
/// is read instead, and a Garmin grant's window carries what the session
/// stored ([`pushed_window`]). A row whose tenant does not parse is not one.
pub(crate) async fn connection_beside_grant(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    connection: &ProviderConnection,
) -> bool {
    match TenantId::parse_str(&connection.tenant_id) {
        Ok(tenant) => {
            backend_resolver::session_beside_grant(
                runtime.repos(),
                user_id,
                tenant,
                &connection.provider,
            )
            .await
        }
        Err(_) => false,
    }
}

/// The backend a read naming `provider_slug` is served by.
///
/// The slug itself, unless it is a mirror routing resolves away from itself
/// — a usr/pwd session beside a Garmin grant
/// (`backend_resolver::resolve_backend`) — which is read, flagged, cached and
/// judged as the grant it stands beside.
/// Keying the read on the session instead re-armed the grant from the
/// session's read and wrote the grant's pushes under the session's key.
pub(crate) async fn serving_slug(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant: Option<TenantId>,
) -> String {
    if !backend_resolver::is_mirror_backend(provider_slug) {
        return provider_slug.to_owned();
    }
    backend_resolver::resolve_backend(runtime.repos(), user_id, tenant, provider_slug).await
}

/// A push-only provider's window: its cached rows for `params`, with those
/// its mirror's usr/pwd session stored, every recording of one workout merged
/// into one session (`super::cached_history`, whose merge's precedence ranks
/// the API's copy over a capture), newest first, `params.offset` skipped.
///
/// Each key is read to `offset + limit` rows: holding at most one row per
/// workout, a key's copy of any of the newest `offset + limit` sessions is
/// among its own newest that many, so the merged page is the true one.
///
/// No live read exists, so nothing is judged, recorded or written back: the
/// rows are exactly what the push ingest stored, and their freshness is the
/// last push, which the ingest records. A write-through here would restamp
/// every row's `synced_at` to now and make a provider that stopped pushing
/// read as current. An empty window is the honest answer for a period
/// nothing was pushed in.
///
/// # Errors
///
/// A read outside a tenant (pushed rows belong to the tenant the connection
/// lives in) and a failed cache read.
pub(super) async fn pushed_window(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant: Option<TenantId>,
    params: &ActivityQueryParams,
) -> AppResult<Vec<Activity>> {
    let tenant = tenant.ok_or_else(|| {
        AppError::invalid_input(format!(
            "{} data is read within the tenant its connection lives in",
            backend_resolver::user_facing_name(provider_slug)
        ))
    })?;
    let (start, end) = cached_window_bounds(params, Utc::now());
    let skip = params.offset.unwrap_or(0);
    let limit = params
        .limit
        .unwrap_or_else(|| usize::try_from(STALE_FALLBACK_LIMIT).unwrap_or(usize::MAX));
    let read = i64::try_from(skip.saturating_add(limit)).unwrap_or(i64::MAX);
    let sessions =
        cached_history(runtime, provider_slug, user_id, tenant, (start, end), read).await?;
    Ok(sessions.into_iter().skip(skip).take(limit).collect())
}

/// A push-only provider's head read: its pushed window, never judged,
/// recorded or written back ([`pushed_window`]), handed back as a model may
/// see it.
///
/// # Errors
///
/// As [`pushed_window`].
pub(super) async fn pushed_head(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant: Option<TenantId>,
    params: &ActivityQueryParams,
) -> AppResult<HeadRead> {
    let activities = ai_scope::unfiltered(pushed_window(
        runtime,
        provider_slug,
        user_id,
        tenant,
        params,
    ))
    .await?;
    Ok(HeadRead {
        activities: ai_scope::filter_activities(runtime.provider_registry().as_ref(), activities),
        verdict: HeadVerdict::Complete,
    })
}

#[cfg(test)]
mod tests {
    use pierre_core::constants::oauth_providers;
    use pierre_providers::registry::global_registry;

    /// The providers every build registers keep their live reads: the flag is
    /// opt-in, and the usr/pwd Garmin connection is not push-only.
    #[test]
    fn the_registered_providers_read_on_request_keep_their_live_reads() {
        let registry = global_registry();
        for slug in [
            oauth_providers::STRAVA,
            oauth_providers::SCIOTTE,
            oauth_providers::SCIOTTE_GARMIN,
            oauth_providers::WHOOP,
            "no_such_provider",
        ] {
            assert!(
                !registry.is_push_only(slug),
                "bug: {slug} is read on request and must keep its live reads"
            );
        }
    }
}
