// ABOUTME: Background scheduler that rolls persona-gated notifications into a weekly digest push
// ABOUTME: Weekly tick per armed weekly-digest user; the digest dispatches at P0 so it always lands

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Persona notification digest scheduler
//!
//! The armed persona notification policy persists a gated push instead of
//! delivering it (see `pierre_notifications::NotificationService::dispatch_with_tier`),
//! and the persona contracts promise those held notifications come back as a
//! digest. This scheduler keeps that promise for the `weekly` cadence.
//!
//! Mirrors the tick/run-loop shape of
//! `pierre_routes_groups::group_digest_scheduler`: a Tokio task spawned at
//! server bootstrap ticks every [`DEFAULT_TICK_INTERVAL`], and the testable
//! [`tick`] does one full sweep. On each tick it:
//!
//! 1. Enumerates every tenant, then each tenant's active users.
//! 2. Resolves each user's [`PushPolicy`] through the [`PersonaPolicyGate`]
//!    and keeps only users whose policy is **armed** with a **weekly** digest
//!    cadence. The `per_session` and `per_athlete` cadences pass through
//!    unbatched — their events already deliver ungated today, and their
//!    batching semantics are the remainder registered as registre#7.
//! 3. Counts the persisted notifications carrying the `persona_gated` marker
//!    since the user's previous digest (falling back to a 7-day window when
//!    no digest exists yet) and, when any exist, dispatches ONE localized
//!    `persona_digest` System notification at [`PushTier::P0`] — P0 so the
//!    digest itself can never be persona-gated.
//!
//! "Since the previous digest" is read from the newest persisted
//! `persona_digest` row, so a restart or an extra tick re-sends nothing: a
//! sweep that finds no newer gated rows sends no digest.
//!
//! [`PushPolicy`]: pierre_notifications::PushPolicy

use std::sync::Arc;
use std::time::Duration as StdDuration;

use crate::periodic::spawn_periodic;
use chrono::{Duration, Utc};
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_core::errors::AppResult;
use pierre_core::models::{TenantId, User};
use pierre_database::RepositoryRegistry;
use pierre_notifications::events::event_data;
use pierre_notifications::models::NotificationCategory;
use pierre_notifications::{
    to_app_error, DigestCadence, DispatchRequest, NotificationEvent, NotificationService,
    PersonaPolicyGate, PushTier, TenantId as CommTenantId, PERSONA_GATED_DATA_KEY,
};
use serde_json::{json, Map, Value};
use tracing::{debug, error, info, warn};

use crate::notification_text::NotificationTextRenderer;

/// How often the digest tick fires. One week — the cadence the contracts name.
pub const DEFAULT_TICK_INTERVAL: StdDuration = StdDuration::from_hours(168);

/// `notification_type` of the digest notification itself. Also the marker the
/// next sweep reads to find where the previous digest window ended.
pub const PERSONA_DIGEST_TYPE: &str = NotificationEvent::PersonaDigest.wire();

/// Upper bound on notifications examined per user per sweep. Bounds the scan
/// the way pagination clamps a list endpoint; a user gating more rows than
/// this in one week still gets a digest, counting the newest rows.
const DIGEST_SCAN_LIMIT: u32 = 500;

/// Outcome of a single scheduler tick — exposed for tests and metrics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PersonaDigestTickOutcome {
    /// Tenants examined this tick.
    pub tenants_scanned: usize,
    /// Active users examined across all tenants.
    pub users_scanned: usize,
    /// Users whose armed policy carries the weekly digest cadence.
    pub users_eligible: usize,
    /// Digest notifications dispatched (one per user with gated rows).
    pub digests_sent: usize,
    /// Per-user errors. Logged; the sweep continues.
    pub errors: usize,
}

/// Run a single digest sweep across all tenants.
///
/// Exposed (rather than only the loop) so integration tests can drive one
/// sweep without `tokio::sleep`. Production callers wrap this in
/// [`start_persona_digest_scheduler`].
///
/// # Errors
///
/// Returns the database error only if the initial tenant enumeration fails.
/// Per-tenant and per-user errors are counted in
/// [`PersonaDigestTickOutcome::errors`] but do not abort the sweep.
pub async fn tick(
    repos: &Arc<RepositoryRegistry>,
    gate: &Arc<dyn PersonaPolicyGate>,
    service: &NotificationService,
    strings: &MessagingStringsRegistry,
) -> AppResult<PersonaDigestTickOutcome> {
    let tenants = repos.tenants.get_all().await?;

    let mut outcome = PersonaDigestTickOutcome {
        tenants_scanned: tenants.len(),
        ..PersonaDigestTickOutcome::default()
    };

    for tenant in tenants {
        process_tenant(repos, gate, service, strings, tenant.id, &mut outcome).await;
    }

    if outcome.users_eligible > 0 {
        info!(
            tenants_scanned = outcome.tenants_scanned,
            users_scanned = outcome.users_scanned,
            users_eligible = outcome.users_eligible,
            digests_sent = outcome.digests_sent,
            errors = outcome.errors,
            "persona notification digest sweep complete"
        );
    } else {
        debug!(
            tenants_scanned = outcome.tenants_scanned,
            users_scanned = outcome.users_scanned,
            "persona notification digest sweep: no armed weekly-digest user"
        );
    }

    Ok(outcome)
}

/// Sweep one tenant's active users, digesting each armed weekly-digest user.
async fn process_tenant(
    repos: &Arc<RepositoryRegistry>,
    gate: &Arc<dyn PersonaPolicyGate>,
    service: &NotificationService,
    strings: &MessagingStringsRegistry,
    tenant_id: TenantId,
    outcome: &mut PersonaDigestTickOutcome,
) {
    let users = match repos.users.get_by_status("active", Some(tenant_id)).await {
        Ok(users) => users,
        Err(e) => {
            error!(tenant_id = %tenant_id, error = %e, "persona digest: user listing failed");
            outcome.errors += 1;
            return;
        }
    };
    for user in users {
        outcome.users_scanned += 1;
        let Some(push_policy) = gate
            .policy_for(user.id, CommTenantId(tenant_id.as_uuid()))
            .await
        else {
            continue;
        };
        if !push_policy.armed || push_policy.digest != Some(DigestCadence::Weekly) {
            continue;
        }
        outcome.users_eligible += 1;
        match send_user_digest(service, strings, &user, tenant_id).await {
            Ok(true) => outcome.digests_sent += 1,
            Ok(false) => {}
            Err(e) => {
                warn!(
                    user_id = %user.id,
                    tenant_id = %tenant_id,
                    error = %e,
                    "persona digest: dispatch failed (best-effort)"
                );
                outcome.errors += 1;
            }
        }
    }
}

/// Whether a persisted notification carries the persona-gated marker.
fn is_persona_gated(data: Option<&Value>) -> bool {
    data.and_then(|d| d.get(PERSONA_GATED_DATA_KEY))
        .and_then(Value::as_bool)
        == Some(true)
}

/// Send one digest to `user` when gated rows accumulated since their previous
/// digest. Returns `Ok(true)` when a digest was dispatched.
async fn send_user_digest(
    service: &NotificationService,
    strings: &MessagingStringsRegistry,
    user: &User,
    tenant_id: TenantId,
) -> AppResult<bool> {
    let tenant = CommTenantId(tenant_id.as_uuid());

    // The previous digest bounds the window; first-ever digest looks 7 days
    // back so boot-time backlog stays one week deep, matching the cadence.
    let (system_rows, _, _) = to_app_error_result(
        service
            .list_notifications(
                user.id,
                tenant,
                DIGEST_SCAN_LIMIT,
                0,
                Some(NotificationCategory::System.as_str()),
                false,
            )
            .await,
    )?;
    let since = system_rows
        .iter()
        .filter(|n| n.notification_type == PERSONA_DIGEST_TYPE)
        .map(|n| n.created_at)
        .max()
        .unwrap_or_else(|| Utc::now() - Duration::days(7));

    let (rows, _, _) = to_app_error_result(
        service
            .list_notifications(user.id, tenant, DIGEST_SCAN_LIMIT, 0, None, false)
            .await,
    )?;
    let item_count = rows
        .iter()
        .filter(|n| n.created_at > since && is_persona_gated(n.data.as_ref()))
        .count();
    if item_count == 0 {
        return Ok(false);
    }

    let params = json!({ "item_count": item_count });
    let renderer = NotificationTextRenderer::new(strings, &user.locale);
    // The digest is dispatched directly rather than through `dispatch_event`
    // because the sweep already holds the user row it would re-read; the text
    // comes from the same renderer either way, so both paths say the same
    // thing, and the stored parameters let the feed re-render it after a
    // language change.
    let empty = Map::new();
    let digest_params = params.as_object().unwrap_or(&empty);
    let request = DispatchRequest {
        user_id: user.id,
        tenant_id: tenant,
        category: NotificationCategory::System,
        notification_type: PERSONA_DIGEST_TYPE.to_owned(),
        title: renderer.title(NotificationEvent::PersonaDigest, digest_params),
        body: renderer.body(NotificationEvent::PersonaDigest, digest_params),
        data: Some(event_data(json!({}), params.clone())),
        image_url: None,
        actions: None,
        bypass_frequency_cap: false,
    };
    // P0: the digest is the delivery the persona floor promised in exchange
    // for the pushes it withheld, so it must never gate itself.
    to_app_error_result(service.dispatch_with_tier(&request, PushTier::P0).await)?;
    debug!(user_id = %user.id, item_count, "persona digest dispatched");
    Ok(true)
}

/// Map a commere result into the platform error type.
fn to_app_error_result<T>(result: Result<T, pierre_notifications::CommereError>) -> AppResult<T> {
    result.map_err(to_app_error)
}

/// Spawn the weekly persona-digest scheduler as a background tokio task.
///
/// Called once at server bootstrap beside the group digest spawn. The
/// [`AbortHandle`](tokio::task::AbortHandle) is discarded because the scheduler
/// is best-effort and a restart re-arms the timer; "already digested" is
/// derived from the persisted rows, so a retry or restart re-sends nothing.
pub fn start_persona_digest_scheduler(
    repos: Arc<RepositoryRegistry>,
    gate: Arc<dyn PersonaPolicyGate>,
    service: Arc<NotificationService>,
    strings: Arc<MessagingStringsRegistry>,
) {
    spawn_periodic(
        "persona notification digest scheduler",
        DEFAULT_TICK_INTERVAL,
        move || {
            let repos = Arc::clone(&repos);
            let gate = Arc::clone(&gate);
            let service = Arc::clone(&service);
            let strings = Arc::clone(&strings);
            async move {
                tick(&repos, &gate, &service, &strings).await?;
                Ok(())
            }
        },
    );
}
