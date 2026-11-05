// ABOUTME: Per-instance poll that re-reads the runtime policy rows, so an admin save reaches every instance
// ABOUTME: Guardian and harness registries converge on system_settings; a failed tick keeps the policy in force

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! How an admin save reaches every instance.
//!
//! A save writes the row and installs the new snapshot on the one instance the
//! request landed on. The deployment runs several with no session affinity, so
//! this poll is how the others hear about it, and how an instance that booted on
//! defaults after a read failure recovers.
//!
//! It runs on every instance by design, so it is not a `spawn_periodic` worker:
//! those claim a lease to tick once across the fleet, which is the opposite. The
//! API runs with `cpu-throttling=false`, so a tick also runs on an idle instance.

use std::sync::Arc;
use std::time::Duration;

use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_database::backends::factory::Database;
use pierre_tool_runtime::guardian::GuardianConfigRegistry;
use tokio::time;
use tracing::{error, info};

/// How often an instance re-reads the rows, unless
/// `PIERRE_SETTINGS_POLL_INTERVAL_SECS` says otherwise.
const DEFAULT_POLL_INTERVAL_SECS: u64 = 30;

/// The poll interval for the deployment's setting, or the default.
#[must_use]
pub fn settings_poll_interval(configured_secs: Option<u64>) -> Duration {
    Duration::from_secs(configured_secs.unwrap_or(DEFAULT_POLL_INTERVAL_SECS))
}

/// The registries one instance keeps converged on the database.
#[derive(Debug, Clone)]
pub struct SettingsPollTargets {
    /// Where the rows live.
    pub database: Arc<Database>,
    /// The Guardian policy this instance enforces.
    pub guardian: Arc<GuardianConfigRegistry>,
    /// The chat harness tunables this instance runs under.
    pub harness: Arc<HarnessConfigRegistry>,
}

/// Spawn the supervised poll.
///
/// The first read comes one interval after boot, since the registries have just
/// read the rows. Supervisor: the loop runs in its own task whose `JoinHandle` is
/// awaited here, so a panicking tick is logged loudly and the loop is respawned
/// after one interval. A dead poll would otherwise freeze this instance on its
/// last policy, silently, while the admin surface reports saves as successful.
pub fn start_settings_poll(targets: SettingsPollTargets, interval: Duration) {
    info!(interval_secs = interval.as_secs(), "settings poll started");
    tokio::spawn(async move {
        loop {
            let poll_targets = targets.clone();
            let poll = tokio::spawn(async move {
                loop {
                    time::sleep(interval).await;
                    poll_targets
                        .guardian
                        .reload_from_database(&poll_targets.database)
                        .await;
                    poll_targets
                        .harness
                        .reload_from_database(&poll_targets.database)
                        .await;
                }
            });
            // The inner loop never returns normally; reaching here means the
            // task died (panic or runtime-cancelled join).
            if let Err(e) = poll.await {
                error!(
                    error = %e,
                    is_panic = e.is_panic(),
                    "settings poll died; respawning after one interval"
                );
            }
            time::sleep(interval).await;
        }
    });
}
