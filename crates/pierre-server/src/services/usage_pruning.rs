// ABOUTME: Background task for periodic pruning of old usage counter records
// ABOUTME: Runs hourly via tokio interval, deleting counters older than 90 days
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Usage Counter Pruning Task
//!
//! Spawns a background tokio task that runs hourly to prune usage counter
//! records with periods older than 90 days. This prevents unbounded growth
//! of the `usage_counters` table.

use std::sync::Arc;

use tokio::task::AbortHandle;
use tokio::time::{interval_at, Duration, Instant};
use tracing::{debug, info, warn};

use crate::config::admin::service::AdminConfigService;
use crate::services::usage_counter::UsageCounterService;
use pierre_database::backends::UsageCounterRepository;

/// Number of seconds in one hour
const HOUR_SECONDS: u64 = 3_600;

/// Default retention period in days for old usage counters
const DEFAULT_RETENTION_DAYS: i64 = 90;

/// Start the periodic usage counter pruning task
///
/// Spawns a background tokio task that:
/// 1. Waits one hour before the first run (avoids startup load)
/// 2. Prunes counters older than 90 days (configurable via admin config)
/// 3. Repeats every hour
pub fn start_usage_pruning_task(
    usage_counters: Arc<dyn UsageCounterRepository>,
    admin_config: Arc<AdminConfigService>,
) -> AbortHandle {
    let hour = Duration::from_secs(HOUR_SECONDS);
    let handle = tokio::spawn(async move {
        // First tick fires after one hour (avoids startup load)
        let mut ticker = interval_at(Instant::now() + hour, hour);
        loop {
            ticker.tick().await;
            run_pruning_cycle(usage_counters.as_ref(), &admin_config).await;
        }
    });
    let abort_handle = handle.abort_handle();
    info!("Usage counter pruning task started (hourly, 90-day retention)");
    abort_handle
}

/// Execute a single pruning cycle: resolve retention config and prune old records
async fn run_pruning_cycle(
    usage_counters: &dyn UsageCounterRepository,
    admin_config: &AdminConfigService,
) {
    let retention_days = admin_config
        .get_value("usage_quotas.counter_retention_days", None)
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
