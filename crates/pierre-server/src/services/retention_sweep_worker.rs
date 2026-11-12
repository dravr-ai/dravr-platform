// ABOUTME: Assembles the one retention sweep from the server context and the activity record store, then starts it
// ABOUTME: The full archive sink retracts what expires, so every expired activity is attested like the cache sweep's was

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Boot wiring for [`pierre_services::retention_sweep`].

use std::sync::Arc;

use dravr_barrage::store::ActivityRecordStore;
use pierre_runtime_context::AdminConfigLookup;
use pierre_services::retention_sweep::{start_retention_sweeper, RetentionSweeper};
use tracing::warn;

use crate::config::admin::AdminConfigService;
use crate::mcp::resources::ServerContext;
use crate::services::archive_import::sink::CachedActivitiesSink;

/// The retention sweep, or `None` without the admin config service.
///
/// It sweeps `records`, the activity record store the server keeps live
/// reads in, and `resources`' wellness tables, under the policy the admin
/// configuration holds, which is why it needs that service.
#[must_use]
pub fn retention_sweeper(
    resources: &Arc<ServerContext>,
    records: Arc<dyn ActivityRecordStore>,
) -> Option<RetentionSweeper> {
    let admin_config = resources.agent.admin_config.as_ref()?;
    let config: Arc<dyn AdminConfigLookup> = Arc::<AdminConfigService>::clone(admin_config);
    let wellness_providers = resources
        .data()
        .provider_registry()
        .supported_providers()
        .into_iter()
        .map(str::to_owned)
        .collect();
    Some(RetentionSweeper::new(
        records,
        Arc::new(CachedActivitiesSink::new(Arc::clone(resources))),
        Arc::clone(&resources.common.repos.provider_data),
        config,
        wellness_providers,
    ))
}

/// Start the retention sweep, hourly on the worker ledger.
///
/// Skipped for an in-memory database, like the other ledger workers, so a
/// throwaway server never runs a pass against a scratch database, and
/// without the admin config service, since the policy lives there.
pub fn start_retention_sweep_worker(
    resources: &Arc<ServerContext>,
    records: Arc<dyn ActivityRecordStore>,
) {
    if resources.common.config.database.url.is_memory() {
        return;
    }
    let Some(sweeper) = retention_sweeper(resources, records) else {
        warn!("admin config service unavailable: the retention sweep is not started, since its policy lives there");
        return;
    };
    start_retention_sweeper(
        Arc::new(sweeper),
        Arc::clone(&resources.common.repos.worker_runs),
    );
}
