// ABOUTME: Background sweeper that deletes expired rows from the mcp_tasks table
// ABOUTME: Every task handle carries a TTL the client is told to honour; this is what makes the TTL real
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! MCP task table hygiene.
//!
//! The tasks extension mints one `mcp_tasks` row per long-running tool call and
//! stamps it with `expires_at_ms`. Reads already filter on that column, so an
//! expired task correctly answers as absent — but nothing deleted the row, so
//! the table grew without bound and the `ttlMs` advertised in every task frame
//! was a promise no code kept. This periodic sweep is what makes the TTL real.
//!
//! Deliberately mirrors [`start_short_link_sweeper`](crate::short_link_sweeper)
//! rather than inventing a second cadence shape: both reclaim rows whose expiry
//! the read path already honours.

use std::sync::Arc;
use std::time::Duration;

use pierre_database::repositories::McpTaskRepository;
use tokio::time::interval;
use tracing::{debug, error};

/// Sweep cadence. Task TTL is 30 minutes, so an hourly reclaim keeps the table
/// bounded well inside an order of magnitude of the lifetime it enforces
/// without polling a table that is empty most of the time.
const SWEEP_INTERVAL: Duration = Duration::from_hours(1);

/// Start the background MCP task sweeper.
///
/// Spawns a `tokio::time::interval` loop that deletes expired `mcp_tasks` rows
/// every [`SWEEP_INTERVAL`]. Fire-and-forget and best-effort: a failed sweep is
/// logged and retried on the next tick, never propagated, because losing a
/// reclamation pass is survivable and taking the server down for it is not.
/// The immediate first tick is consumed so a restart doesn't sweep instantly.
pub fn start_mcp_task_sweeper(tasks: Arc<dyn McpTaskRepository>) {
    tokio::spawn(async move {
        debug!("MCP task sweeper started");
        let mut ticker = interval(SWEEP_INTERVAL);
        ticker.tick().await; // consume the immediate first tick
        loop {
            ticker.tick().await;
            match tasks.delete_expired_tasks(now_ms()).await {
                Ok(0) => {}
                Ok(removed) => debug!(removed, "MCP task sweep reclaimed expired rows"),
                Err(e) => error!(error = %e, "MCP task sweep failed"),
            }
        }
    });
}

/// Current unix time in milliseconds, the unit `mcp_tasks.expires_at_ms` uses.
fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}
