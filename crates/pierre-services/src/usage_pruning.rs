// ABOUTME: Background task for periodic pruning of old usage counter records
// ABOUTME: Runs hourly on spawn_periodic, deleting counters older than 90 days
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Usage Counter Pruning Task
//!
//! Spawns a periodic worker that runs hourly to prune usage counter records
//! with periods older than 90 days. This prevents unbounded growth of the
//! `usage_counters` table.

use std::sync::Arc;
use std::time::Duration;

use tokio::task::AbortHandle;
use tracing::{debug, info, warn};

use crate::periodic::spawn_periodic;
use crate::usage_counter::UsageCounterService;
use pierre_database::backends::UsageCounterRepository;
use pierre_database::repositories::WorkerRunRepository;
use pierre_runtime_context::{AdminConfigLookup, ConfigLookupScope};

/// Number of seconds in one hour
const HOUR_SECONDS: u64 = 3_600;

/// Default retention period in days for old usage counters
const DEFAULT_RETENTION_DAYS: i64 = 90;

/// Start the periodic usage counter pruning task
///
/// Runs on [`spawn_periodic`]: the `ledger` decides when the next hourly
/// pass is due, so a fresh instance prunes when an hour has elapsed since
/// the last pass anywhere, not one hour after its own boot. Each pass
/// prunes counters older than 90 days (configurable via admin config).
pub fn start_usage_pruning_task(
    usage_counters: Arc<dyn UsageCounterRepository>,
    admin_config: Arc<dyn AdminConfigLookup>,
    ledger: Arc<dyn WorkerRunRepository>,
) -> AbortHandle {
    let abort_handle = spawn_periodic(
        "usage counter pruning",
        Duration::from_secs(HOUR_SECONDS),
        ledger,
        move || {
            let usage_counters = Arc::clone(&usage_counters);
            let admin_config = Arc::clone(&admin_config);
            async move {
                run_pruning_cycle(usage_counters.as_ref(), admin_config.as_ref()).await;
                Ok(())
            }
        },
    );
    info!("Usage counter pruning task started (hourly, 90-day retention)");
    abort_handle
}

/// Execute a single pruning cycle: resolve retention config and prune old records
async fn run_pruning_cycle(
    usage_counters: &dyn UsageCounterRepository,
    admin_config: &dyn AdminConfigLookup,
) {
    let retention_days = admin_config
        .get_value(
            "usage_quotas.counter_retention_days",
            ConfigLookupScope::global(),
        )
        .await
        .ok()
        .flatten()
        .and_then(|v| v.as_i64())
        .unwrap_or(DEFAULT_RETENTION_DAYS);

    let usage_svc = UsageCounterService::new(usage_counters, admin_config);

    match usage_svc.prune_old_counters(retention_days).await {
        Ok(0) => debug!("Usage counter pruning: no records to delete"),
        Ok(deleted) => info!(
            deleted_count = deleted,
            retention_days, "Pruned old usage counter records"
        ),
        Err(e) => warn!("Usage counter pruning failed: {e}"),
    }
}
