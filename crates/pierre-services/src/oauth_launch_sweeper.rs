// ABOUTME: Background sweeper that reports OAuth launches which expired without ever completing
// ABOUTME: Reclaims the dead state rows and makes an unfinished connect flow visible instead of silent

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Detection for a connect flow that starts and never returns.
//!
//! Every OAuth launch stores an `oauth_client_state` row for CSRF validation
//! and consumes it at the callback. A flow the user completes therefore leaves
//! a consumed row; a flow that dies between the authorize redirect and the
//! callback leaves one that simply expires unconsumed. Nothing read those, so
//! the difference between "nobody connected today" and "connecting is broken
//! for everyone on iPhone" was not recorded anywhere.
//!
//! That is not hypothetical. On 2026-09-10 the launch popup opened blank and
//! never navigated; the server issued a valid authorize URL, returned 200, and
//! logged nothing further. Four failures across two people were invisible to
//! every signal the platform keeps, and were found only by correlating nginx
//! access logs by hand — init-200s with no callback after them — a day later.
//! This sweep is that correlation, done continuously and cheaply.
//!
//! Abandonment is normal: a user opens the consent screen, changes their mind,
//! and closes the window. So a single reaped row means nothing and this emits a
//! *count*, leaving the judgement of "how many is too many" to the alert
//! threshold rather than hard-coding a verdict here.
//!
//! Mirrors [`short_link_sweeper`](crate::short_link_sweeper) and
//! [`mcp_task_sweeper`](crate::mcp_task_sweeper) rather than inventing a third
//! cadence shape. It differs from both in one way: they reclaim rows whose
//! expiry the read path already honours and log at `debug!` because the count
//! is routine hygiene. Here the count IS the signal, so it goes out at `warn!`
//! with a stable marker string an alert can match on.

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use pierre_database::repositories::OAuthClientStateRepository;
use tracing::warn;

use crate::periodic::spawn_periodic;

/// Sweep cadence. OAuth states live 10 minutes, so a 15-minute pass reports a
/// dead launch while the user who hit it is plausibly still at their desk, and
/// keeps the alert's window meaningful without polling a table that is empty
/// whenever connecting works.
const SWEEP_INTERVAL: Duration = Duration::from_mins(15);

/// The marker an alert matches on. Kept as one literal so the string a log
/// filter greps for and the string this emits cannot drift apart; the
/// monitoring filter in `infra/environments/dev/monitoring.tf` matches it.
const EXPIRED_LAUNCH_MARKER: &str = "oauth launches expired without completing";

/// Start the background OAuth launch sweeper.
///
/// Reclaims `oauth_client_state` rows that expired unconsumed every
/// [`SWEEP_INTERVAL`] and reports how many, per provider. Fire-and-forget and
/// best-effort: a failed pass is logged and retried on the next tick, never
/// propagated, because losing one detection pass is survivable and taking the
/// server down for it is not.
pub fn start_oauth_launch_sweeper(states: Arc<dyn OAuthClientStateRepository>) {
    spawn_periodic("oauth launch sweeper", SWEEP_INTERVAL, move || {
        let states = Arc::clone(&states);
        async move {
            let reaped = states.reap_expired_oauth_client_states(Utc::now()).await?;
            report_reaped(&reaped);
            Ok(())
        }
    });
}

/// Emit the sweep result, or nothing at all when every launch completed.
///
/// Split out so the reporting rule — silence on a clean sweep, one `warn!`
/// carrying the total and the per-provider breakdown otherwise — is testable
/// without a database or a running tick loop.
fn report_reaped(reaped: &[(String, u64)]) {
    let total: u64 = reaped.iter().map(|(_, count)| *count).sum();
    if total == 0 {
        return;
    }
    warn!(
        reaped = total,
        providers = %format_breakdown(reaped),
        "{EXPIRED_LAUNCH_MARKER}"
    );
}

/// Render the per-provider counts as `strava=3 whoop=1`.
///
/// A bare total says connecting is failing; the breakdown says which provider,
/// which is the first thing an operator needs and the difference between one
/// provider's outage and a platform-wide one.
fn format_breakdown(reaped: &[(String, u64)]) -> String {
    let mut rendered = String::new();
    for (provider, count) in reaped {
        if !rendered.is_empty() {
            rendered.push(' ');
        }
        // Writing to a String is infallible; the `_ = ` discards the Result
        // rather than unwrapping it, which is banned in src.
        _ = write!(rendered, "{provider}={count}");
    }
    rendered
}
