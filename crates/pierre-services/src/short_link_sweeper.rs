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

use pierre_database::repositories::ShortLinkRepository;
use tokio::time::interval;
use tracing::{debug, error};

/// Sweep cadence. Short links live 24h; a periodic reclaim keeps the table
/// bounded without being chatty (the DELETE is a single indexed range scan).
const SWEEP_INTERVAL: Duration = Duration::from_hours(6);

/// Start the background short-link sweeper.
///
/// Spawns a `tokio::time::interval` loop that deletes expired `short_links`
/// rows every [`SWEEP_INTERVAL`]. Fire-and-forget and best-effort: a failed
/// sweep is logged and retried on the next tick, never propagated. The
/// immediate first tick is consumed so a restart doesn't sweep instantly.
pub fn start_short_link_sweeper(short_links: Arc<dyn ShortLinkRepository>) {
    tokio::spawn(async move {
        debug!("short-link sweeper started");
        let mut ticker = interval(SWEEP_INTERVAL);
        ticker.tick().await; // consume the immediate first tick
        loop {
            ticker.tick().await;
            match short_links.delete_expired_short_links().await {
                Ok(0) => {}
                Ok(removed) => debug!(removed, "short-link sweep reclaimed expired rows"),
                Err(e) => error!(error = %e, "short-link sweep failed"),
            }
        }
    });
}
