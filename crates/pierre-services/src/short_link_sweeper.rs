// ABOUTME: Background sweeper that deletes expired rows from the short_links table
// ABOUTME: The reconnect/connect shortener mints a row per link; resolution filters expiry, this reclaims storage
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Short-link table hygiene.
//!
//! [`shorten_url`](pierre_database::repositories::shorten_url) inserts one
//! `short_links` row per minted reconnect/connect link, and the chat reconnect
//! path mints on every expired-session turn by design (a broken first link must
//! not silence the user). `resolve_short_link` filters expired rows at read
//! time, so the only thing missing is reclamation — without it the table grows
//! unbounded. This periodic sweep deletes rows past their TTL, keeping only the
//! live (<= 24h) links the migration's index is sized for.

use std::sync::Arc;
use std::time::Duration;

use crate::periodic::spawn_periodic;
use pierre_database::repositories::{ShortLinkRepository, WorkerRunRepository};
use tracing::debug;

/// Sweep cadence. Short links live 24h; a periodic reclaim keeps the table
/// bounded without being chatty (the DELETE is a single indexed range scan).
const SWEEP_INTERVAL: Duration = Duration::from_hours(6);

/// Start the background short-link sweeper.
///
/// Spawns a [`spawn_periodic`] loop that deletes expired `short_links` rows
/// every [`SWEEP_INTERVAL`]. Fire-and-forget and best-effort: a failed sweep
/// is logged and retried on the next tick, never propagated. The `ledger`
/// keeps the schedule across restarts, so a fresh instance sweeps when the
/// interval since the last sweep has elapsed, not one interval after boot.
pub fn start_short_link_sweeper(
    short_links: Arc<dyn ShortLinkRepository>,
    ledger: Arc<dyn WorkerRunRepository>,
) {
    spawn_periodic("short-link sweeper", SWEEP_INTERVAL, ledger, move || {
        let short_links = Arc::clone(&short_links);
        async move {
            let removed = short_links.delete_expired_short_links().await?;
            if removed > 0 {
                debug!(removed, "short-link sweep reclaimed expired rows");
            }
            Ok(())
        }
    });
}
