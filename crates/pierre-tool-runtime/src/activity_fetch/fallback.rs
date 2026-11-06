// ABOUTME: Serves a turn's window from the athlete's other connections when the elected one cannot authenticate
// ABOUTME: A usr/pwd session beside a grant is never a sibling; it answers only with a dead Garmin grant's stored history
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The fallback serve behind a dead or unreachable elected provider.

use pierre_core::models::{Activity, ConnectionStatus, ProviderConnection, TenantId};
use pierre_providers::backend_resolver;
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::deduplication::{merge_duplicates, DedupConfig, FragmentReport};
use tracing::warn;

use super::{
    connection_beside_grant, fetch_provider_activities, read_cached_window, serve_stale_activities,
    uploads,
};
use crate::context::ToolExecutionContext;

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

/// What `conn` adds to [`serve_without_primary`] when it is a usr/pwd session
/// beside a grant, or `None` when it is not one and is read as any other
/// connection.
///
/// Such a session serves nothing (carnet#1386). What it stored rides its
/// Garmin grant's window, which no other connection reads once that grant is
/// the dead primary, so the stored window answers then, once
/// (`served_once`), and the history keeps being served. `Some(None)` leaves
/// the session out.
async fn beside_grant_rows(
    context: &ToolExecutionContext,
    conn: &ProviderConnection,
    primary_backend: &str,
    params: &ActivityQueryParams,
    served_once: &mut bool,
) -> Option<Option<Vec<Activity>>> {
    if !connection_beside_grant(&context.resources, context.user_id, conn).await {
        return None;
    }
    if *served_once
        || !backend_resolver::history_keys(primary_backend).contains(&conn.provider.as_str())
    {
        return Some(None);
    }
    *served_once = true;
    let Ok(tenant) = TenantId::parse_str(&conn.tenant_id) else {
        return Some(None);
    };
    Some(
        serve_stale_activities(
            &context.resources,
            primary_backend,
            context.user_id,
            tenant,
            params,
        )
        .await,
    )
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
}

/// Serve the requested window from the athlete's OTHER connections when the
/// elected primary cannot authenticate.
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
    // backend are fetched once instead of returning every session twice. A
    // usr/pwd session beside a grant is never read as a sibling at all
    // (`beside_grant_rows`).
    let mut asked: Vec<String> = vec![primary_backend.to_owned()];
    let mut primary_history_served = false;
    for conn in &connections {
        if conn.status == ConnectionStatus::Revoked {
            continue;
        }
        if let Some(rows) = beside_grant_rows(
            context,
            conn,
            primary_backend,
            params,
            &mut primary_history_served,
        )
        .await
        {
            if let Some(rows) = rows {
                served_by.push(backend_resolver::user_facing_name(primary_backend).to_owned());
                served.extend(rows);
            }
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
    })
}
