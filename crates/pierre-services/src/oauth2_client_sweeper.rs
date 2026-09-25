// ABOUTME: Background sweeper that deletes RFC 7591 client registrations retention no longer keeps
// ABOUTME: Reclaims expired registrations past their grace and the ones no user ever authorized
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! OAuth 2.0 client registration hygiene.
//!
//! `POST /oauth2/register` is anonymous by design (RFC 7591), and every row it
//! wrote used to stay forever: `expires_at` stopped an expired client from
//! being used, but nothing deleted it, so the table grew with every
//! registration anyone ever made (carnet#483). This sweep deletes what the
//! [`ClientRetentionConfig`] no longer keeps — registrations past their expiry
//! grace, and registrations no user authorized within a day — and the pending
//! ceiling enforced at registration holds the second kind to a fixed count
//! between sweeps.
//!
//! Mirrors [`short_link_sweeper`](crate::short_link_sweeper) and
//! [`mcp_task_sweeper`](crate::mcp_task_sweeper): it reclaims rows whose
//! expiry the read path already honours, on [`spawn_periodic`], whose ledger
//! runs the first pass within seconds of boot whenever one is due. Unlike them
//! it logs a non-empty pass at `info!`: how many registrations were abandoned
//! is how an operator sees the anonymous endpoint being filled.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use pierre_auth::config::ClientRetentionConfig;
use pierre_auth::oauth2_server::ClientRegistrationManager;
use pierre_core::models::OAuth2ClientSweep;
use pierre_database::repositories::{OAuth2ServerRepository, WorkerRunRepository};
use tracing::info;

use crate::periodic::spawn_periodic;

/// Start the background OAuth 2.0 client registration sweeper.
///
/// Every `retention.sweep_interval_secs` it deletes the registrations the
/// policy no longer keeps (see
/// [`ClientRegistrationManager::sweep_stale_clients`]). Fire-and-forget and
/// best-effort: a failed pass is logged and retried by the tick loop, never
/// propagated, because a missed reclamation is survivable and the pending
/// ceiling keeps refusing new registrations meanwhile.
pub fn start_oauth2_client_sweeper(
    oauth2: Arc<dyn OAuth2ServerRepository>,
    ledger: Arc<dyn WorkerRunRepository>,
    retention: ClientRetentionConfig,
) {
    let period = Duration::from_secs(retention.sweep_interval_secs);
    spawn_periodic("oauth2 client sweeper", period, ledger, move || {
        let clients = ClientRegistrationManager::new(Arc::clone(&oauth2));
        async move {
            let sweep = clients.sweep_stale_clients(&retention, Utc::now()).await?;
            if sweep != OAuth2ClientSweep::default() {
                info!(
                    expired = sweep.expired,
                    abandoned = sweep.abandoned,
                    orphaned_grants = sweep.orphaned_grants,
                    "OAuth2 client sweep deleted stale registrations"
                );
            }
            Ok(())
        }
    });
}
