// ABOUTME: Shared page-count policy every provider's activity fetch clamps to
// ABOUTME: One configurable ceiling, so no provider silently truncates a deep window
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Shared paging policy for provider activity fetches.
//!
//! Every upstream answers a bounded number of activities per request — 200 for
//! Strava and Intervals.icu, 100 for Garmin and Fitbit, 50 for COROS, 25 for
//! WHOOP. A caller asking for a season therefore needs several requests, and
//! the question "how many may I issue?" has to have the same answer for every
//! provider, or the shallow ones truncate while the deep ones do not.
//!
//! It did not. Strava and Garmin walked as many pages as the caller's limit
//! implied, with no ceiling at all; WHOOP, COROS, Fitbit and Intervals.icu
//! clamped the caller's limit to a single page and returned quietly. That
//! difference was not local. The historical gate asks every provider for
//! `DEFAULT_HISTORICAL_BACKFILL_FETCH_LIMIT` activities and then reads
//! `fetched_count < fetch_limit` as proof the window was exhausted — so a
//! provider that quietly answered 25 made the backfill record a depth it never
//! reached, and the gate served that shallow slice as a complete season from
//! then on.
//!
//! So the ceiling lives here, once, and every provider clamps to it.

use std::env;

use tracing::warn;

/// Environment override for [`max_activity_pages`].
pub const MAX_ACTIVITY_PAGES_ENV: &str = "PIERRE_PROVIDER_MAX_ACTIVITY_PAGES";

/// Default ceiling on the requests one activity fetch may issue.
///
/// Chosen so it does not bind at the default backfill depth on any provider:
/// two thousand activities is eighty pages at WHOOP's 25 per request, forty at
/// COROS's 50, twenty at Garmin's and Fitbit's 100, ten at Strava's and
/// Intervals.icu's 200. That makes this a runaway backstop rather than a policy
/// limit — the thing that stops a caller-supplied limit from turning into an
/// unbounded walk — which is the only role a shared ceiling can honestly play
/// while the count of returned rows is still what tells the backfill whether a
/// window was exhausted.
pub const DEFAULT_MAX_ACTIVITY_PAGES: usize = 100;

/// Ceiling on the requests a single activity fetch may issue, for every provider.
///
/// Lower it to trade depth for upstream calls on a rate-limited deployment;
/// raise it for an athlete with more history than the default depth reaches.
/// A zero or unparseable value falls back to [`DEFAULT_MAX_ACTIVITY_PAGES`] —
/// a ceiling of zero would fetch nothing at all.
#[must_use]
pub fn max_activity_pages() -> usize {
    env::var(MAX_ACTIVITY_PAGES_ENV)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_MAX_ACTIVITY_PAGES)
}

/// How many requests a fetch of `limit` activities may issue at `page_size` each.
///
/// Always at least one, so a fetch never resolves to "issue no requests", and
/// never more than [`max_activity_pages`]. `page_size` is floored at 1 because
/// it divides.
///
/// Logs when the ceiling binds: a truncated walk that says nothing reads
/// downstream as a window that was exhausted, which is the exact confusion this
/// module exists to end.
#[must_use]
pub fn pages_for(limit: usize, page_size: usize) -> usize {
    let wanted = limit.div_ceil(page_size.max(1)).max(1);
    let ceiling = max_activity_pages();
    if wanted > ceiling {
        warn!(
            requested_limit = limit,
            page_size,
            pages_wanted = wanted,
            pages_allowed = ceiling,
            "Activity fetch capped by {MAX_ACTIVITY_PAGES_ENV}: returning at most {} of {limit} \
             requested activities",
            ceiling.saturating_mul(page_size),
        );
        return ceiling;
    }
    wanted
}
