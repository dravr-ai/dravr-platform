// ABOUTME: Serves a read the athlete's session could not answer: its pair's OAuth grant first, then their other connections
// ABOUTME: One fallback (carnet#1494): the head reads, the flagged session and the dead primary all ask the grant the same way
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The fallback serve behind a dead or unreachable elected provider.
//!
//! Sciotte first (carnet#1494): an athlete's usr/pwd session serves their
//! reads, and the provider's own OAuth grant stands behind it while it is
//! live (`backend_resolver::read_plan`). Every read the session cannot answer
//! asks that grant first, the same way: a head read that falls back
//! ([`serve_head`]), a session already flagged, and a primary that cannot
//! authenticate ([`grant_window`], then [`serve_without_primary`]'s other
//! connections).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, ConnectionStatus, ProviderConnection, TenantId};
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::deduplication::{merge_duplicates, DedupConfig, FragmentReport};
use pierre_providers::{ai_scope, backend_resolver};
use tracing::warn;
use uuid::Uuid;

use super::pushed::{connection_grant_beside_session, pushed_head};
use super::sync_verdict::{
    falls_back, log_head_failure, read_provider_head, record_head_attempt, HeadRead, HeadVerdict,
    LiveRead,
};
use super::{
    fetch_provider_activities, read_cached_window, read_exact_backend, serve_stale_activities,
    uploads, Routing,
};
use crate::capture_sweep::FLAG_REASON;
use crate::context::ToolExecutionContext;
use crate::protocol::reauth_notice;
use crate::runtime::ToolRuntime;

/// The OAuth grant standing behind `backend` for this athlete: the read
/// plan's fallback, when `backend` is that plan's primary.
///
/// `None` for a backend that is not its pair's primary — a grant read
/// exactly as named (a Strava webhook, the grant's own fallback read) has
/// nothing behind it — and for a session with no live
/// grant beside it.
pub(super) async fn pair_grant(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant: TenantId,
    backend: &str,
) -> Option<String> {
    let plan = backend_resolver::read_plan(runtime.repos(), user_id, tenant, backend).await;
    plan.fallback.filter(|_| plan.primary == backend)
}

/// Serve a head read of `live` the session answered with `read`: that read,
/// or, when it falls back ([`falls_back`]) and a grant stands behind the
/// session ([`pair_grant`]), the grant's live read of the same window in its
/// place, marked [`HeadRead::served_by_grant`].
///
/// Records stay per backend: the session's failure is recorded under the
/// session at once ([`record_head_attempt`]), the grant's answer under the
/// grant (the caller writes it through under [`write_key`]), and a grant
/// that fails too under the grant. The page is decided once both are known:
/// `error` with the session's cause only when nothing served, a constant
/// `warn` when the grant served. A session the read found dead is flagged
/// for its sign-in while its grant keeps the athlete served. A push-only
/// grant (Garmin) serves only when it holds rows for the window.
pub(super) async fn serve_head(
    runtime: &Arc<dyn ToolRuntime>,
    live: LiveRead<'_>,
    tenant: TenantId,
    read: AppResult<HeadRead>,
) -> AppResult<HeadRead> {
    let LiveRead {
        provider_slug,
        user_id,
        params,
        ..
    } = live;
    let grant = if falls_back(&read) {
        pair_grant(runtime, user_id, tenant, provider_slug).await
    } else {
        None
    };
    let page_owed =
        record_head_attempt(runtime, user_id, tenant, provider_slug, params, &read).await;
    let Some(grant) = grant else {
        if page_owed {
            log_head_failure(&read, user_id, provider_slug);
        }
        return read;
    };
    answer_with_grant(runtime, live, tenant, &grant, read, page_owed).await
}

/// [`serve_head`] once a grant stands behind the session whose `read` fell
/// back: the grant's read when it vouched for the window, else the session's
/// own `read`, paged now when `page_owed`.
async fn answer_with_grant(
    runtime: &Arc<dyn ToolRuntime>,
    live: LiveRead<'_>,
    tenant: TenantId,
    grant: &str,
    read: AppResult<HeadRead>,
    page_owed: bool,
) -> AppResult<HeadRead> {
    let LiveRead {
        provider_slug,
        user_id,
        params,
        attempt_started_at,
        ..
    } = live;
    let grant_read =
        ai_scope::unfiltered(read_grant_head(runtime, grant, user_id, tenant, params)).await;
    // A push-only grant always answers complete, so an empty pushed window
    // vouches for nothing a failed session read missed: it does not serve.
    let push_only = runtime.provider_registry().is_push_only(grant);
    match grant_read {
        Ok(mut head)
            if head.verdict == HeadVerdict::Complete
                && !(push_only && head.activities.is_empty()) =>
        {
            if let Err(e) = &read {
                flag_refusal(
                    runtime,
                    user_id,
                    tenant,
                    provider_slug,
                    e,
                    attempt_started_at,
                )
                .await;
            }
            warn!(
                user_id = %user_id,
                provider = %provider_slug,
                grant = %grant,
                cause = %head_cause(&read),
                "Live activity head read failed on the session; its grant answered in its place"
            );
            head.served_by_grant = Some(grant.to_owned());
            Ok(head)
        }
        grant_read => {
            // A grant refused behind a live session is not flagged here: its
            // own refresh flags a dead grant, and a grant dying beside a
            // session asks nothing of the athlete (carnet#1494).
            record_head_attempt(runtime, user_id, tenant, grant, params, &grant_read).await;
            warn!(
                user_id = %user_id,
                provider = %provider_slug,
                grant = %grant,
                cause = %head_cause(&grant_read),
                "The grant behind a failed session read did not answer either"
            );
            if page_owed {
                log_head_failure(&read, user_id, provider_slug);
            }
            read
        }
    }
}

/// What a head read that did not serve says went wrong, for a log line.
fn head_cause(read: &AppResult<HeadRead>) -> String {
    match read {
        Ok(head) => head.verdict.label().to_owned(),
        Err(e) => e.to_string(),
    }
}

/// Flag the session `backend` when `error` is its own refusal for
/// authentication (`reauth_notice::refused_backend`): a dead session asks
/// for its sign-in while its grant serves. A list read
/// ([`answer_with_grant`]) and a by-id read
/// (`protocol::auth::grant_by_id`) that fall back flag it the same way.
pub(crate) async fn flag_refusal(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant: TenantId,
    backend: &str,
    error: &AppError,
    attempt_started_at: DateTime<Utc>,
) {
    if reauth_notice::refused_backend(error, backend).as_deref() == Some(backend) {
        reauth_notice::flag_refused(
            runtime,
            user_id,
            tenant,
            backend,
            FLAG_REASON,
            attempt_started_at,
        )
        .await;
    }
}

/// The grant's live read of `params`, authenticated exactly as `grant` with
/// its own token row, judged and never recorded or written: a push-only
/// grant (Garmin) answers with what it pushed.
async fn read_grant_head(
    runtime: &Arc<dyn ToolRuntime>,
    grant: &str,
    user_id: Uuid,
    tenant: TenantId,
    params: &ActivityQueryParams,
) -> AppResult<HeadRead> {
    if runtime.provider_registry().is_push_only(grant) {
        return pushed_head(runtime, grant, user_id, Some(tenant), params).await;
    }
    let tenant_id = tenant.to_string();
    let live = LiveRead {
        provider_slug: grant,
        user_id,
        tenant_id: Some(&tenant_id),
        params,
        attempt_started_at: Utc::now(),
    };
    read_provider_head(runtime, live, Routing::Exact).await
}

/// The cache key a served head read of `provider_slug` is written through
/// under: its own, or the grant's when the grant answered, and none for a
/// push-only grant, whose rows are exactly what its pushes stored.
pub(super) fn write_key<'a>(
    runtime: &Arc<dyn ToolRuntime>,
    provider_slug: &'a str,
    head: &'a HeadRead,
) -> Option<&'a str> {
    match head.served_by_grant.as_deref() {
        None => Some(provider_slug),
        Some(grant) if runtime.provider_registry().is_push_only(grant) => None,
        Some(grant) => Some(grant),
    }
}

/// The window `params` read from the grant standing behind the session
/// `primary` ([`pair_grant`]), with the grant's name, or `None` when no grant
/// stands behind it or the grant's read failed.
///
/// Read exactly as the grant ([`read_exact_backend`]): judged, recorded and
/// written through under the grant, a push-only grant from its pushes. For a
/// session that is flagged or cannot authenticate, and for a read pinned to
/// the provider: the grant is the same provider, so a pinned read still
/// reaches it.
pub(crate) async fn grant_window(
    runtime: &Arc<dyn ToolRuntime>,
    primary: &str,
    user_id: Uuid,
    tenant: TenantId,
    params: &ActivityQueryParams,
) -> Option<(String, Vec<Activity>)> {
    let grant = pair_grant(runtime, user_id, tenant, primary).await?;
    match read_exact_backend(runtime, &grant, user_id, &tenant.to_string(), params).await {
        Ok(rows) => Some((grant, rows)),
        Err(e) => {
            warn!(
                user_id = %user_id,
                provider = %primary,
                grant = %grant,
                error = %e,
                "the grant behind the session could not serve the window"
            );
            None
        }
    }
}

/// A sibling connection's rows for [`serve_without_primary`], read as its
/// health allows: a flagged one from its durable cache, a deep historical
/// window on a scrape-backed mirror from its cache too, any other live.
async fn sibling_rows(
    context: &ToolExecutionContext,
    conn: &ProviderConnection,
    canonical: &str,
    is_historical: bool,
    params: &ActivityQueryParams,
) -> Option<Vec<Activity>> {
    if conn.status.requires_reauth() {
        let conn_tenant = TenantId::parse_str(&conn.tenant_id).ok()?;
        serve_stale_activities(
            &context.resources,
            canonical,
            context.user_id,
            conn_tenant,
            params,
        )
        .await
    } else if is_historical && backend_resolver::is_mirror_backend(canonical) {
        let conn_tenant = TenantId::parse_str(&conn.tenant_id).ok()?;
        read_cached_window(
            &context.resources,
            canonical,
            context.user_id,
            conn_tenant,
            params,
        )
        .await
    } else {
        fetch_provider_activities(
            &context.resources,
            &conn.provider,
            context.user_id,
            &conn.tenant_id,
            params,
        )
        .await
    }
}

/// Ask the grant standing behind `primary_backend` first, for
/// [`serve_without_primary`]: its rows join `served` under its name, and it
/// joins the backends `asked`, so no sibling reads it again. The grant's
/// name when it served a row.
async fn ask_grant_first(
    context: &ToolExecutionContext,
    primary_backend: &str,
    params: &ActivityQueryParams,
    (served, served_by, asked): (&mut Vec<Activity>, &mut Vec<String>, &mut Vec<String>),
) -> Option<String> {
    let tenant = TenantId::from_uuid(context.tenant_id?);
    let (grant, rows) = Box::pin(grant_window(
        &context.resources,
        primary_backend,
        context.user_id,
        tenant,
        params,
    ))
    .await?;
    asked.push(grant.clone());
    if rows.is_empty() {
        return None;
    }
    served_by.push(backend_resolver::user_facing_name(&grant).to_owned());
    served.extend(rows);
    Some(grant)
}

/// A window the athlete's other connections served after the elected primary
/// failed to authenticate, dead or only unreachable just now.
#[derive(Debug)]
pub struct FallbackServe {
    /// The activities those connections produced, every recording of one
    /// workout merged into a single session.
    pub activities: Vec<Activity>,
    /// The groups that merge collapsed and the fields each session gained.
    pub merge_report: FragmentReport,
    /// User-facing names of the connections that contributed at least one row,
    /// so the response names its real sources instead of the dead provider.
    pub served_by: Vec<String>,
    /// The pair's grant that answered first, in the session's place
    /// ([`grant_window`]); `None` when no grant stands behind the primary or
    /// it could not answer.
    pub served_by_grant: Option<String>,
}

/// Serve the requested window from the athlete's OTHER connections when the
/// elected primary cannot authenticate.
///
/// The pair's grant is the first one asked ([`grant_window`]): the same
/// provider, read through its API in the session's place (carnet#1494). A
/// grant beside a session is never read as a sibling after it
/// (`backend_resolver::grant_beside_session`), so the API is read once.
///
/// A multi-source aggregator answers with what it holds: an athlete whose WHOOP
/// token died, or whose Strava refresh is rate limited, still has years of
/// history behind a healthy connection, and why the primary is missing belongs
/// BESIDE that answer rather than instead of it. The caller keeps that signal
/// (a reconnect prompt, or a "could not be reached" note) and attaches it as a
/// caveat; only when this returns `None` — no other connection produced a
/// single row — does the turn become the primary's failure alone.
///
/// Health-aware: a sibling already flagged `needs_reauth` is not fetched into the
/// same failure; its durable cache answers instead, exactly as
/// [`fetch_provider_activities`] answers the refusal that flagged it — so the
/// second read of a turn serves the rows the first one did, rather than losing
/// them to the flag the first read raised. A `revoked` sibling is skipped: the
/// athlete withdrew that access. Cross-tenant like the peer path, so
/// a provider connected under the athlete's own tenant answers from a group
/// conversation. A deep historical window on a scrape-backed mirror reads that
/// sibling's durable cache rather than scraping inline — the historical branch
/// exists precisely to keep a multi-year page out of the turn. The union is
/// merged the same way [`maybe_merge_other_connections`] merges it, so a
/// watch's misclassified twin of a GPS session collapses into it.
pub async fn serve_without_primary(
    context: &ToolExecutionContext,
    primary_backend: &str,
    is_historical: bool,
    params: &ActivityQueryParams,
) -> Option<FallbackServe> {
    let tenant = context.tenant_id.map(TenantId::from_uuid);
    let connections = context
        .resources
        .repos()
        .provider_connections
        .get_for_user(context.user_id, None)
        .await
        .ok()?;

    let mut served: Vec<Activity> = Vec::new();
    let mut served_by: Vec<String> = Vec::new();
    // Backends already asked, so two connection rows that resolve to the SAME
    // backend are fetched once instead of returning every session twice.
    let mut asked: Vec<String> = vec![primary_backend.to_owned()];
    let served_by_grant = ask_grant_first(
        context,
        primary_backend,
        params,
        (&mut served, &mut served_by, &mut asked),
    )
    .await;
    for conn in &connections {
        if conn.status == ConnectionStatus::Revoked
            || connection_grant_beside_session(&context.resources, context.user_id, conn).await
        {
            continue;
        }
        let canonical = backend_resolver::resolve_backend(
            context.resources.repos(),
            context.user_id,
            tenant,
            &conn.provider,
        )
        .await;
        if asked.contains(&canonical) {
            continue;
        }
        asked.push(canonical.clone());
        let fetched = sibling_rows(context, conn, &canonical, is_historical, params).await;
        if let Some(rows) = fetched {
            if !rows.is_empty() {
                served_by.push(backend_resolver::user_facing_name(&canonical).to_owned());
                served.extend(rows);
            }
        }
    }
    let held = uploads::held_user_provided(context).await;
    for key in held {
        let rows = uploads::user_provided_window(context, &[key], params).await;
        if !rows.is_empty() {
            served_by.push(key.to_owned());
            served.extend(rows);
        }
    }

    if served.is_empty() {
        return None;
    }
    let (served, merge_report) = merge_duplicates(served, &DedupConfig::from_env());
    warn!(
        user_id = %context.user_id,
        primary = %primary_backend,
        count = served.len(),
        served_by = %served_by.join(", "),
        "elected provider could not authenticate; serving the window from the athlete's other connections"
    );
    Some(FallbackServe {
        activities: served,
        merge_report,
        served_by,
        served_by_grant,
    })
}
