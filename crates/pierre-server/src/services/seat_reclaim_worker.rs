// ABOUTME: Assembles the Strava seat reclaimer from the server context and starts it on the worker ledger
// ABOUTME: The binary decides that it starts; this knows which slices of ServerContext the sweeper reads and acts through

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Boot wiring for [`pierre_services::strava_seat_reclaim`].

use std::sync::Arc;

use pierre_runtime_context::AdminConfigLookup;
use pierre_services::oauth_flow::OAuthService;
use pierre_services::strava_seat_reclaim::{start_strava_seat_reclaimer, StravaSeatReclaimer};
use pierre_services::user_removal::ProviderDisconnector;
use tracing::warn;

use crate::mcp::resources::ServerContext;

/// Start the Strava seat reclaimer.
///
/// Its policy is runtime configuration, so without the admin config service
/// there is nothing to run it under and it is not started. It disconnects
/// through the same chokepoint as the athlete's own disconnect and warns
/// through the notification service the reconnect pushes use. Skipped for an
/// in-memory database, like the coaching workers, so a throwaway server never
/// runs a pass against a scratch database.
pub fn start_seat_reclaim_worker(resources: &Arc<ServerContext>) {
    if resources.common.config.database.url.is_memory() {
        return;
    }
    let Some(admin_config) = resources.agent.admin_config.as_ref() else {
        warn!("admin config service unavailable: the Strava seat reclaimer is not started, since its policy lives there");
        return;
    };
    let config: Arc<dyn AdminConfigLookup> = Arc::clone(admin_config) as Arc<dyn AdminConfigLookup>;
    let disconnector: Arc<dyn ProviderDisconnector> = Arc::new(OAuthService::new(
        resources.data(),
        Arc::clone(&resources.common.config),
    ));
    #[cfg(feature = "client-notifications")]
    let notifications = resources.common.notification_service.clone();
    #[cfg(not(feature = "client-notifications"))]
    let notifications = None;

    let reclaimer = Arc::new(StravaSeatReclaimer::new(
        Arc::clone(&resources.common.repos),
        config,
        disconnector,
        notifications,
    ));
    start_strava_seat_reclaimer(reclaimer, Arc::clone(&resources.common.repos.worker_runs));
}
