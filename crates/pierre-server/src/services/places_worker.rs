// ABOUTME: The periodic worker that recomputes the places of athletes whose cached activities changed
// ABOUTME: A bounded pass on the worker ledger: up to twenty athletes every fifteen minutes, local CPU only

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Athlete places worker (carnet#1484, slice 6).
//!
//! Places are clustered from the trimmed starts of an athlete's stored tracks
//! ([`pierre_services::athlete_places`]). Nothing on the ingest, erasure,
//! expiry or purge paths calls the clustering: this worker finds the athletes
//! whose cached rows changed since their last clustering (or whose last one
//! is a day old) and recomputes them, so a first-time athlete has places
//! within a tick of their first sync. A pass reads only the platform's own
//! tables and asks nothing of any network.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use pierre_services::athlete_places::{places_pass, PlacesPassReport};
use pierre_services::periodic::spawn_periodic;
use tracing::{info, warn};

use crate::mcp::resources::ServerContext;

/// The name every log line and the worker ledger row carry.
pub const WORKER_NAME: &str = "athlete places";

/// Pass cadence: a first-time athlete's places appear within it.
pub const TICK_INTERVAL: Duration = Duration::from_mins(15);

/// The most athletes one pass recomputes; the next pass takes the rest.
pub const USERS_PER_PASS: u32 = 20;

/// Start the places worker on the worker ledger.
///
/// Skipped for an in-memory database, like the other workers, so a
/// throwaway server never writes behind a test's back.
pub fn start_places_worker(resources: &Arc<ServerContext>) {
    if resources.common.config.database.url.is_memory() {
        return;
    }
    let ledger = Arc::clone(&resources.common.repos.worker_runs);
    let repos = Arc::clone(&resources.common.repos);
    spawn_periodic(WORKER_NAME, TICK_INTERVAL, ledger, move || {
        let repos = Arc::clone(&repos);
        async move {
            let report = places_pass(&repos, Utc::now(), USERS_PER_PASS).await?;
            log_pass(report);
            Ok(())
        }
    });
}

/// Log a pass that did anything; one with any failed athlete at `WARN`.
fn log_pass(report: PlacesPassReport) {
    if report.failed > 0 {
        warn!(
            worker = WORKER_NAME,
            users = report.users,
            places = report.places,
            failed = report.failed,
            "athlete places pass finished with failed athletes"
        );
    } else if report != PlacesPassReport::default() {
        info!(
            worker = WORKER_NAME,
            users = report.users,
            places = report.places,
            "athlete places pass finished"
        );
    }
}
