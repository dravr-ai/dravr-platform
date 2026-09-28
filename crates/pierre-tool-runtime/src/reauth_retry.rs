// ABOUTME: Throttled retry of a scrape session flagged needs_reauth — one attempt per connection per interval
// ABOUTME: OAuth connections are never retried; a served retry re-arms the connection through the live-read path

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Revisiting a flag one failed read set.
//!
//! A `needs_reauth` flag is a verdict one attempt reached. For an OAuth
//! connection it is final: the provider refused the refresh token, and only a
//! reconnect mints a new one. A scrape session is different — its credential
//! is a cookie jar the scraper service holds, and one refusal can be the
//! scraper rather than the session: on 2026-09-25 the capture sweep flagged a
//! connection over a single `401` from a scraper instance that had not seen
//! the session import, and the flag then stood while the session was fine,
//! because every refresh path acts on `active` connections only.
//!
//! So the capture sweep and the athlete's Home page also attempt a flagged
//! connection whose backend is a scrape mirror
//! ([`backend_resolver::is_mirror_backend`]), at most once per
//! [`SCRAPE_SESSION_RETRY_INTERVAL_HOURS`] per connection, across every caller
//! and replica ([`claim_scrape_session_retry`]). The attempt is an ordinary
//! live read through [`crate::activity_fetch::fetch_provider_head`]: a read
//! the session serves re-arms the connection there, and a refused one leaves
//! the flag standing without a second notice, since the notice is claimed once
//! per transition. The interval is what keeps a retry from hammering the
//! scraper, and the sweep never logs in, so no retry can prompt a 2FA tap.

use std::sync::Arc;

use chrono::{Duration, Utc};
use pierre_core::models::TenantId;
use pierre_providers::backend_resolver::is_mirror_backend;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::runtime::ToolRuntime;

/// Fewest hours between two attempts of one flagged scrape session.
///
/// The flag itself counts as an attempt, so the first retry comes this long
/// after the flag. Six hours lets a scraper-side blip clear within the day
/// while costing each flagged connection at most four headless reads a day.
pub const SCRAPE_SESSION_RETRY_INTERVAL_HOURS: i64 = 6;

/// Whether a flagged connection on `provider` is one the platform retries at
/// all: a scrape session, never an OAuth grant.
#[must_use]
pub fn retries_flagged_session(provider: &str) -> bool {
    is_mirror_backend(provider)
}

/// Claim this interval's one retry of the flagged scrape session
/// `(user_id, tenant_id, provider)`.
///
/// `false` — attempt nothing — for a provider that is not a scrape mirror, for
/// a connection that is not `needs_reauth`, and for one flagged or retried
/// within [`SCRAPE_SESSION_RETRY_INTERVAL_HOURS`]. A claim that cannot be
/// written reads as `false` too: an unthrottled attempt is the thing this
/// exists to prevent.
pub async fn claim_scrape_session_retry(
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> bool {
    if !retries_flagged_session(provider) {
        return false;
    }
    let due_before = Utc::now() - Duration::hours(SCRAPE_SESSION_RETRY_INTERVAL_HOURS);
    match runtime
        .repos()
        .provider_connections
        .claim_reauth_retry(user_id, tenant_id, provider, due_before)
        .await
    {
        Ok(claimed) => {
            debug!(
                user_id = %user_id,
                provider = %provider,
                claimed,
                "flagged scrape session retry claim"
            );
            claimed
        }
        Err(e) => {
            warn!(
                user_id = %user_id,
                provider = %provider,
                error = %e,
                "flagged scrape session retry claim failed; not retrying"
            );
            false
        }
    }
}
