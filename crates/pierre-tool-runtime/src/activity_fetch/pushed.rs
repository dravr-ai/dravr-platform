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

use std::sync::Arc;

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, TenantId};
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::{ai_scope, backend_resolver};
use uuid::Uuid;

use super::sync_verdict::{HeadRead, HeadVerdict};
use super::{cached_window_bounds, STALE_FALLBACK_LIMIT};
use crate::runtime::ToolRuntime;

/// Whether this user's reads of `provider_slug` are served from what a
/// push-only provider pushed: the slug is push-only AND it is the backend
/// the user's reads of it resolve to.
///
/// The resolution matters because a caller may name the user-facing
/// provider: `garmin` resolves to the credential connection
/// (`sciotte_garmin`) for an athlete who has one, and that connection is read
/// on request exactly as before. The resolver is consulted only for a
/// push-only slug, so no other provider's read pays for it.
pub(super) async fn reads_pushed_store(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &str,
    user_id: Uuid,
    tenant: Option<TenantId>,
) -> bool {
    if !runtime.provider_registry().is_push_only(provider_slug) {
        return false;
    }
    backend_resolver::resolve_backend(
        &runtime.repos().auth_repos(),
        user_id,
        tenant,
        provider_slug,
    )
    .await
        == provider_slug
}

/// A push-only provider's window: its cached rows for `params`, newest
/// first, `params.offset` skipped.
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
    let rows = runtime
        .repos()
        .activity_cache
        .get_cached_activities(user_id, &tenant, Some(provider_slug), start, end, read)
        .await?;
    Ok(rows.into_iter().skip(skip).take(limit).collect())
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
