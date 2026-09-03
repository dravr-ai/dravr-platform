// ABOUTME: Background scheduler that periodically pushes group weekly digests
// ABOUTME: Reads the weekly_digest tier flag to gate which tenants' groups are eligible
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Group Weekly-Digest Scheduler
//!
//! Closes the gap where [`pierre_groups::GroupService::compute_weekly_report`]
//! was reachable only via the on-demand `/api/groups/{id}/report` handler, so
//! the `weekly_digest` tier flag
//! ([`pierre_groups::strategies::tier::GroupFeatureFlags::weekly_digest`],
//! `true` for Professional/Enterprise) was set but never read.
//!
//! This service ticks every [`DEFAULT_TICK_INTERVAL`] (one week) from a Tokio
//! task spawned at server bootstrap next to the coach-followup scheduler. On
//! each tick it:
//!
//! 1. Enumerates every tenant ([`pierre_database::repositories::TenantRepository::get_all`]).
//! 2. Skips tenants whose plan tier does not enable `weekly_digest` — this is
//!    the read that makes the tier flag load-bearing.
//! 3. For each eligible tenant, lists its active groups
//!    ([`pierre_database::repositories::CoachingGroupRepository::list_active_groups_for_tenant`]).
//! 4. Builds member fitness snapshots via the canonical
//!    [`fetch_member_snapshots`] builder (the same all-providers + deduplicated
//!    path the chat coach and the REST analytics endpoints use), computes the
//!    weekly report with [`pierre_groups::GroupService::compute_weekly_report`],
//!    and dispatches it as a `Coach`-category notification to every member who
//!    can manage the group (owner + admins).
//!
//! Dispatch is best-effort: a failed snapshot fetch or notification send is
//! logged and counted but never aborts the rest of the sweep. There is no
//! "already sent" persistence — the weekly cadence is enforced by the tick
//! interval, so a server restart re-arms the timer rather than re-sending.

use std::sync::Arc;
use std::time::Duration as StdDuration;

use pierre_core::errors::AppResult;
use pierre_core::models::groups::{CoachingGroup, GroupMember};
use pierre_core::models::TenantId;
use pierre_groups::strategies::tier::tier_strategy_for;
use pierre_runtime_context::{GroupsCtx, MiddlewareCtx};
use pierre_services::periodic::spawn_periodic;
use pierre_tool_runtime::group_fitness::fetch_member_snapshots;
use pierre_tool_runtime::runtime::ToolRuntime;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

#[cfg(feature = "client-notifications")]
use pierre_notifications::{
    models::NotificationCategory, DispatchRequest, NotificationService, PushTier,
    TenantId as CommTenantId,
};

/// How often the digest tick fires. One week — the digest cadence.
pub const DEFAULT_TICK_INTERVAL: StdDuration = StdDuration::from_hours(168);

/// Outcome of a single scheduler tick — exposed for tests and metrics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DigestTickOutcome {
    /// Tenants examined this tick.
    pub tenants_scanned: usize,
    /// Tenants whose tier enables `weekly_digest`.
    pub tenants_eligible: usize,
    /// Active groups for which a report was computed.
    pub groups_reported: usize,
    /// Notification dispatch attempts (a no-op when no notification service).
    pub dispatched: usize,
    /// Dispatch or snapshot errors. Logged; the sweep continues.
    pub errors: usize,
}

/// Run a single digest sweep across all tenants.
///
/// Exposed (rather than only the loop) so integration tests can drive one
/// sweep without `tokio::sleep`. Production ticks it from
/// [`start_digest_scheduler`].
///
/// `notification_service` is borrowed as an `Option` because some build
/// profiles compile out the notifications feature entirely.
///
/// # Errors
///
/// Returns the database error only if the initial `get_all` tenant query
/// fails. Per-tenant and per-group errors are counted in
/// `DigestTickOutcome.errors` but do not abort the sweep.
pub async fn tick<C>(
    ctx: &Arc<C>,
    runtime: &Arc<dyn ToolRuntime>,
    #[cfg(feature = "client-notifications")] notification_service: Option<&NotificationService>,
) -> AppResult<DigestTickOutcome>
where
    C: ToolRuntime + GroupsCtx + MiddlewareCtx,
{
    let tenants = MiddlewareCtx::repos(ctx.as_ref()).tenants.get_all().await?;

    let mut outcome = DigestTickOutcome {
        tenants_scanned: tenants.len(),
        ..DigestTickOutcome::default()
    };

    for tenant in tenants {
        if !tier_enables_digest(&tenant.plan) {
            continue;
        }
        outcome.tenants_eligible += 1;
        process_tenant(
            ctx,
            runtime,
            #[cfg(feature = "client-notifications")]
            notification_service,
            tenant.id,
            &mut outcome,
        )
        .await;
    }

    if outcome.tenants_eligible > 0 {
        info!(
            tenants_scanned = outcome.tenants_scanned,
            tenants_eligible = outcome.tenants_eligible,
            groups_reported = outcome.groups_reported,
            dispatched = outcome.dispatched,
            errors = outcome.errors,
            "group weekly-digest sweep complete"
        );
    } else {
        debug!(
            tenants_scanned = outcome.tenants_scanned,
            "group weekly-digest sweep: no tenant tier enables weekly_digest"
        );
    }

    Ok(outcome)
}

/// Whether a tenant plan's tier enables the `weekly_digest` feature. This is
/// the read that makes the previously-dormant tier flag load-bearing.
#[must_use]
pub fn tier_enables_digest(plan: &str) -> bool {
    tier_strategy_for(plan).allowed_features().weekly_digest
}

/// Process every active group for one eligible tenant.
async fn process_tenant<C>(
    ctx: &Arc<C>,
    runtime: &Arc<dyn ToolRuntime>,
    #[cfg(feature = "client-notifications")] notification_service: Option<&NotificationService>,
    tenant_id: TenantId,
    outcome: &mut DigestTickOutcome,
) where
    C: ToolRuntime + GroupsCtx + MiddlewareCtx,
{
    let groups = match MiddlewareCtx::repos(ctx.as_ref())
        .groups
        .list_active_groups_for_tenant(tenant_id)
        .await
    {
        Ok(groups) => groups,
        Err(e) => {
            error!(tenant_id = %tenant_id, error = %e, "digest: failed to list active groups");
            outcome.errors += 1;
            return;
        }
    };

    for group in groups {
        process_group(
            ctx,
            runtime,
            #[cfg(feature = "client-notifications")]
            notification_service,
            tenant_id,
            &group,
            outcome,
        )
        .await;
    }
}

/// Build the report for one group and dispatch it to its managers.
async fn process_group<C>(
    ctx: &Arc<C>,
    runtime: &Arc<dyn ToolRuntime>,
    #[cfg(feature = "client-notifications")] notification_service: Option<&NotificationService>,
    tenant_id: TenantId,
    group: &CoachingGroup,
    outcome: &mut DigestTickOutcome,
) where
    C: ToolRuntime + GroupsCtx + MiddlewareCtx,
{
    let group_id = group.id.to_string();
    let members = match MiddlewareCtx::repos(ctx.as_ref())
        .groups
        .list_members(&group_id)
        .await
    {
        Ok(members) => members,
        Err(e) => {
            error!(group_id = %group_id, error = %e, "digest: failed to list members");
            outcome.errors += 1;
            return;
        }
    };

    let user_ids: Vec<Uuid> = members.iter().map(|m| m.user_id).collect();
    let snapshots = fetch_member_snapshots(runtime, &user_ids, tenant_id).await;
    let report = ctx
        .group_service()
        .compute_weekly_report(&snapshots, &group.name);
    outcome.groups_reported += 1;

    #[cfg(feature = "client-notifications")]
    dispatch_to_managers(
        notification_service,
        tenant_id,
        group,
        &members,
        &report.summary,
        outcome,
    )
    .await;

    debug!(
        group_id = %group_id,
        members = members.len(),
        "digest: weekly report computed"
    );
}

/// Dispatch the digest summary to every member who can manage the group
/// (owner + admins). Best-effort per recipient.
#[cfg(feature = "client-notifications")]
async fn dispatch_to_managers(
    service: Option<&NotificationService>,
    tenant_id: TenantId,
    group: &CoachingGroup,
    members: &[GroupMember],
    summary: &str,
    outcome: &mut DigestTickOutcome,
) {
    let Some(service) = service else { return };
    for member in members.iter().filter(|m| m.role.can_manage_members()) {
        outcome.dispatched += 1;
        if let Err(e) = dispatch_digest(service, tenant_id, group, member.user_id, summary).await {
            warn!(
                group_id = %group.id,
                user_id = %member.user_id,
                error = %e,
                "digest: notification dispatch failed (best-effort)"
            );
            outcome.errors += 1;
        }
    }
}

#[cfg(feature = "client-notifications")]
async fn dispatch_digest(
    service: &NotificationService,
    tenant_id: TenantId,
    group: &CoachingGroup,
    user_id: Uuid,
    summary: &str,
) -> Result<pierre_notifications::DispatchOutcome, pierre_notifications::CommereError> {
    let request = DispatchRequest {
        user_id,
        tenant_id: CommTenantId(tenant_id.into()),
        category: NotificationCategory::Coach,
        notification_type: "group_weekly_digest".to_owned(),
        title: format!("Weekly digest: {}", group.name),
        body: summary.to_owned(),
        data: None,
        image_url: None,
        actions: None,
        bypass_frequency_cap: false,
    };
    // P3: a weekly roll-up is ambient by construction — any persona floor
    // below "everything" prefers it in the in-app list over a push.
    service.dispatch_with_tier(&request, PushTier::P3).await
}

/// Spawn the weekly-digest scheduler as a background tokio task.
///
/// Called once at server bootstrap from `spawn_background_workers`. The task
/// runs for the server's lifetime; the
/// [`AbortHandle`](tokio::task::AbortHandle) is discarded because the scheduler
/// is best-effort and a server restart re-arms it.
pub fn start_digest_scheduler<C>(
    ctx: Arc<C>,
    #[cfg(feature = "client-notifications")] notification_service: Option<Arc<NotificationService>>,
) where
    C: ToolRuntime + GroupsCtx + MiddlewareCtx,
{
    // The same context, once as itself and once as the runtime trait object
    // the tick signature takes; both are cloned per tick so the closure stays
    // callable for the life of the worker.
    let cloned: Arc<C> = Arc::clone(&ctx);
    let runtime: Arc<dyn ToolRuntime> = cloned;

    spawn_periodic(
        "group weekly-digest scheduler",
        DEFAULT_TICK_INTERVAL,
        move || {
            let ctx = Arc::clone(&ctx);
            let runtime = Arc::clone(&runtime);
            #[cfg(feature = "client-notifications")]
            let notification_service = notification_service.clone();
            async move {
                tick(
                    &ctx,
                    &runtime,
                    #[cfg(feature = "client-notifications")]
                    notification_service.as_deref(),
                )
                .await?;
                Ok(())
            }
        },
    );
}
