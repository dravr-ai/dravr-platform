// ABOUTME: The daily Garmin grant check — renews each live Garmin grant's token, flagging one Garmin refused to renew
// ABOUTME: So "Reconnect needed" shows for an athlete whose grant died even when they record nothing Garmin would push

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The daily Garmin grant check (carnet#1454).
//!
//! The grant is Garmin's one ongoing source, and the worker finds it dead
//! only on a call that spends its token: an activity file Garmin pushed. An
//! athlete who records nothing for the refresh token's lifetime would never
//! be told their connection died. Once a day this walks every tenant's
//! `active` Garmin connections and asks for each a valid token, which
//! renews one that is due through the same token path every Garmin read
//! uses. A grant whose path hands back no token (Garmin refused the renewal,
//! nothing is left to renew with) is flagged as the worker flags it, and
//! the athlete is told once. A failure that is not that verdict (a store
//! hiccup, a renewal that failed without a refusal, a connection whose token
//! row is gone) flags nothing.
//!
//! It renews an OAuth grant; it keeps no usr/pwd session alive.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use pierre_core::constants::oauth_providers::GARMIN;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_services::garmin_push::GarminOwner;
use pierre_services::periodic::spawn_periodic;
use pierre_tool_runtime::protocol::AuthService;
use pierre_tool_runtime::runtime::ToolRuntime;
use tracing::{info, warn};

use super::worker::{flag_garmin, GRANT_REFUSED};
use crate::mcp::resources::ServerContext;

/// The check's name on the worker ledger.
const WORKER_NAME: &str = "garmin grant refresh";

/// How often every live Garmin grant is checked.
const CHECK_INTERVAL: Duration = Duration::from_hours(24);

/// What one pass found.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GrantCheck {
    /// Grants whose token is valid, renewed where it was due.
    pub live: usize,
    /// Grants Garmin refused to renew, now flagged for reconnect.
    pub flagged: usize,
    /// Grants whose token path failed without a verdict, or whose token row
    /// is gone; left as they are.
    pub unread: usize,
}

/// Start the daily check. Skipped for an in-memory database, like the other
/// workers, so a throwaway server never runs a pass against a scratch one.
pub fn start_garmin_grant_refresh(resources: &Arc<ServerContext>) {
    if resources.common.config.database.url.is_memory() {
        return;
    }
    let ledger = Arc::clone(&resources.common.repos.worker_runs);
    let resources = Arc::clone(resources);
    spawn_periodic(WORKER_NAME, CHECK_INTERVAL, ledger, move || {
        let resources = Arc::clone(&resources);
        async move {
            let check = check_garmin_grants(&resources).await?;
            info!(
                live = check.live,
                flagged = check.flagged,
                unread = check.unread,
                "Garmin grant check ran"
            );
            Ok(())
        }
    });
}

/// One pass over every tenant's `active` Garmin connections.
///
/// # Errors
///
/// When the tenants, or one tenant's Garmin connections, cannot be listed.
pub async fn check_garmin_grants(resources: &Arc<ServerContext>) -> AppResult<GrantCheck> {
    let tenants: Vec<TenantId> = resources
        .common
        .repos
        .tenants
        .get_all()
        .await?
        .into_iter()
        .map(|tenant| tenant.id)
        .collect();
    check_garmin_grants_in(resources, &tenants).await
}

/// One pass over the `active` Garmin connections of `tenants`, each read in
/// its own tenant.
///
/// # Errors
///
/// When one tenant's Garmin connections cannot be listed.
pub async fn check_garmin_grants_in(
    resources: &Arc<ServerContext>,
    tenants: &[TenantId],
) -> AppResult<GrantCheck> {
    let repos = &resources.common.repos;
    let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(resources);
    let auth = AuthService::new(Arc::clone(&runtime));
    let mut check = GrantCheck::default();
    for &tenant_id in tenants {
        let tenant_str = tenant_id.to_string();
        let users = repos
            .provider_connections
            .list_active_connection_users(tenant_id, GARMIN)
            .await?;
        for user_id in users {
            let attempt_started_at = Utc::now();
            match auth
                .get_valid_token(user_id, GARMIN, Some(&tenant_str))
                .await
            {
                Ok(Some(_)) => check.live += 1,
                Ok(None) => {
                    if flag_if_refused(
                        &runtime,
                        GarminOwner { user_id, tenant_id },
                        attempt_started_at,
                    )
                    .await
                    {
                        check.flagged += 1;
                    } else {
                        check.unread += 1;
                    }
                }
                Err(e) => {
                    warn!(user_id = %user_id, error = %e, "Garmin grant check: the token path failed; nothing flagged");
                    check.unread += 1;
                }
            }
        }
    }
    Ok(check)
}

/// Flag `owner`'s Garmin grant, whose token path handed back no token, and
/// report whether it was flagged.
///
/// No token is Garmin's refusal only over a token row still there: a row gone
/// (a disconnect since the list was read, a connection whose token was never
/// stored) is nothing to reconnect, and a row that cannot be re-read is no
/// verdict. Neither is flagged.
async fn flag_if_refused(
    runtime: &Arc<dyn ToolRuntime>,
    owner: GarminOwner,
    attempt_started_at: DateTime<Utc>,
) -> bool {
    let GarminOwner { user_id, tenant_id } = owner;
    match runtime
        .repos()
        .oauth_tokens
        .get_token(user_id, tenant_id, GARMIN)
        .await
    {
        Ok(Some(_)) => {
            flag_garmin(runtime, owner, GRANT_REFUSED, attempt_started_at).await;
            true
        }
        Ok(None) => {
            info!(user_id = %user_id, "Garmin grant check: no token row behind the connection; skipped");
            false
        }
        Err(e) => {
            warn!(user_id = %user_id, error = %e, "Garmin grant check: the token row could not be re-read; nothing flagged");
            false
        }
    }
}
