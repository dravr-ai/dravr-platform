// ABOUTME: Nightly capture sweep — refreshes every live connection's head and flags the dead ones
// ABOUTME: Never attempts a login; an auth-shaped failure flags the connection and waits for the athlete
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The half that restarts a stopped capture.
//!
//! `ActivityCacheRepository::capture_freshness_snapshot` and
//! `/admin/diagnostics/capture-staleness` gave a frozen capture a reader: an
//! athlete being served from cache by a provider that stopped answering is now
//! visible. Being visible is not being fixed — nothing re-fetches, and nothing
//! marks the connection so the athlete is asked to reconnect. This is that
//! actor, driven off the same snapshot the reader judges, so the two can never
//! disagree about which connections are live.
//!
//! # The shape that keeps 2FA out of the loop
//!
//! [`refresh_captures`] **never attempts a login.** It refreshes only what is
//! already authenticated, and when a capture fails in an auth-shaped way it
//! flags the connection and moves on. The athlete's next turn already consults
//! that flag and hands back a reconnect link.
//!
//! That constraint is not fastidiousness. A fresh scraper login can demand a 2FA
//! phone tap within a four-minute window, and the scraper service scales to zero
//! holding no durable session, so there is nothing to resume from after an idle
//! period — an unattended re-login is not something that can be made to work at
//! 04:00. What this sweep buys is the conversion of *"silently captures nothing
//! for days"* into *"tells you it needs a reconnect"*, which is precisely the
//! incident that prompted it.
//!
//! # Why the sweep does not talk to providers itself
//!
//! Every fetch goes through [`fetch_provider_head`], the same write-through the
//! read path uses. A second writer to the activity cache would duplicate the
//! upsert, dedup, prune and freshness-mark logic — and `fetched_at`, the mark
//! that write-through advances, is the exact signal the staleness reader trusts.

use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_providers::core::ActivityQueryParams;
use serde::Serialize;
use tokio::time::timeout;
use tracing::{info, warn};
use uuid::Uuid;

use crate::activity_fetch::fetch_provider_head;
use crate::runtime::ToolRuntime;

/// How far back one sweep fetch reaches. A nightly cadence only needs to cover
/// the day that passed; a week absorbs several consecutive failures without
/// turning the refresh into a backfill, which has its own bounded path.
const HEAD_WINDOW_DAYS: i64 = 7;

/// Cap on activities pulled per connection. A week of one athlete's training is
/// far under this, so the bound only ever truncates a pathological feed.
const HEAD_FETCH_LIMIT: usize = 50;

/// Upper bound on connections one sweep walks.
pub const DEFAULT_CONNECTION_LIMIT: i64 = 5_000;

/// Per-connection fetch bound.
///
/// A scrape-backed fetch drives a headless browser and is measured in tens of
/// seconds. This bounds the *fetch*, never a login — the sweep does not log in,
/// so the scraper's four-minute phone-tap window is not in play here.
pub const DEFAULT_CONNECTION_TIMEOUT_SECS: u64 = 90;

/// Whole-sweep bound.
///
/// The deployed API's request timeout is 600 s and this runs inline on a request
/// (the service scales to zero, so a detached task has no CPU to run on once the
/// response is sent). Finishing under the budget with an honest "did not reach
/// these" beats a 504 that reports nothing at all.
pub const DEFAULT_SWEEP_BUDGET_SECS: u64 = 480;

/// Reason recorded on a connection this sweep flags.
///
/// Matches the vocabulary the backfill notifier already writes, so a reader of
/// `provider_connections.last_error` sees one word for one condition regardless
/// of which path noticed it.
const FLAG_REASON: &str = "session_expired";

/// Bounds for one refresh sweep.
#[derive(Debug, Clone, Copy)]
pub struct SweepBudget {
    /// Longest one connection's fetch may take.
    pub per_connection: Duration,
    /// Longest the whole sweep may take before it stops starting new fetches.
    pub total: Duration,
    /// Most connections to walk.
    pub connection_limit: i64,
}

impl Default for SweepBudget {
    fn default() -> Self {
        Self {
            per_connection: Duration::from_secs(DEFAULT_CONNECTION_TIMEOUT_SECS),
            total: Duration::from_secs(DEFAULT_SWEEP_BUDGET_SECS),
            connection_limit: DEFAULT_CONNECTION_LIMIT,
        }
    }
}

/// What one connection's refresh attempt did.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum RefreshOutcome {
    /// The provider answered and the result was written through, advancing the
    /// `fetched_at` the staleness reader trusts.
    Refreshed {
        /// Activities the provider returned for the head window.
        activities: usize,
    },
    /// The provider rejected the stored credential or session, so the connection
    /// was flipped to `needs_reauth` and the athlete's next turn will offer a
    /// reconnect.
    Flagged {
        /// Reason recorded on the connection.
        reason: String,
    },
    /// The attempt failed in a way that is not the athlete's session dying — a
    /// timeout, a 5xx, an unsupported provider, a malformed stored id. Recorded,
    /// never flagged: a flake is not a disconnect.
    Failed {
        /// Error text, for the operator reading the report.
        error: String,
    },
    /// The sweep ran out of its time budget before reaching this connection.
    SkippedBudgetExhausted,
}

/// One connection's line in the refresh report.
#[derive(Debug, Clone, Serialize)]
pub struct ConnectionRefresh {
    /// Tenant the connection belongs to.
    pub tenant_id: String,
    /// Athlete the connection belongs to, in the stored string form — a
    /// malformed id is reported here rather than dropped by a failed parse.
    pub user_id: String,
    /// Provider slug.
    pub provider: String,
    /// What happened.
    #[serde(flatten)]
    pub outcome: RefreshOutcome,
}

/// The result of one sweep.
#[derive(Debug, Clone, Serialize)]
pub struct RefreshReport {
    /// When the sweep started.
    pub started_at: DateTime<Utc>,
    /// When it finished.
    pub finished_at: DateTime<Utc>,
    /// Connections the sweep attempted a fetch for.
    pub attempted: usize,
    /// Attempts whose provider answered.
    pub refreshed: usize,
    /// Attempts that flipped a connection to `needs_reauth`.
    pub flagged: usize,
    /// Attempts that failed transiently.
    pub failed: usize,
    /// Connections the time budget left unreached.
    pub skipped: usize,
    /// Whether the sweep reached every connection in the snapshot.
    pub completed: bool,
    /// Every connection walked, in the order it was walked.
    pub connections: Vec<ConnectionRefresh>,
}

/// Refresh the recent head of every live provider connection.
///
/// Walks the same snapshot the staleness reader judges — which already excludes
/// connections needing re-auth, since a known-dead one has nothing to refresh
/// and re-attempting it would spend a scrape per night for nothing.
///
/// Never attempts a login. Each connection is fetched through the shared
/// write-through path; an auth-shaped failure flags the connection with
/// [`FLAG_REASON`] and the sweep moves on.
///
/// # Errors
///
/// Returns an error only when the connection snapshot itself cannot be read. A
/// per-connection failure is reported in [`RefreshReport::connections`], never
/// propagated — one dead provider must not stop the sweep of every other.
pub async fn refresh_captures(
    runtime: &Arc<dyn ToolRuntime>,
    budget: SweepBudget,
) -> AppResult<RefreshReport> {
    let started_at = Utc::now();
    let deadline = Instant::now() + budget.total;
    let snapshot = runtime
        .repos()
        .activity_cache
        .capture_freshness_snapshot(budget.connection_limit)
        .await?;

    let after_ts = (started_at - ChronoDuration::days(HEAD_WINDOW_DAYS)).timestamp();
    let params = ActivityQueryParams {
        limit: Some(HEAD_FETCH_LIMIT),
        offset: None,
        before: None,
        after: Some(after_ts),
    };

    let mut report = Vec::with_capacity(snapshot.len());
    let (mut attempted, mut refreshed, mut flagged, mut failed, mut skipped) = (0, 0, 0, 0, 0);

    for connection in snapshot {
        let outcome = if Instant::now() >= deadline {
            skipped += 1;
            RefreshOutcome::SkippedBudgetExhausted
        } else {
            attempted += 1;
            let outcome = refresh_one(
                runtime,
                &connection.tenant_id,
                &connection.user_id,
                &connection.provider,
                &params,
                budget.per_connection,
            )
            .await;
            match outcome {
                RefreshOutcome::Refreshed { .. } => refreshed += 1,
                RefreshOutcome::Flagged { .. } => flagged += 1,
                _ => failed += 1,
            }
            outcome
        };

        report.push(ConnectionRefresh {
            tenant_id: connection.tenant_id,
            user_id: connection.user_id,
            provider: connection.provider,
            outcome,
        });
    }

    let completed = skipped == 0;

    info!(
        attempted,
        refreshed, flagged, failed, skipped, completed, "Capture sweep finished"
    );

    Ok(RefreshReport {
        started_at,
        finished_at: Utc::now(),
        attempted,
        refreshed,
        flagged,
        failed,
        skipped,
        completed,
        connections: report,
    })
}

/// Refresh one connection's head, flagging it when the failure is auth-shaped.
async fn refresh_one(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: &str,
    user_id: &str,
    provider: &str,
    params: &ActivityQueryParams,
    per_connection: Duration,
) -> RefreshOutcome {
    // The snapshot carries ids in their stored string form so a malformed one is
    // reported rather than silently dropped. The fetch needs them typed, so the
    // parse happens here and its failure is an outcome like any other.
    let Ok(parsed_user) = Uuid::from_str(user_id) else {
        warn!(
            user_id = %user_id,
            provider = %provider,
            "Capture sweep: connection carries an unparseable user_id"
        );
        return RefreshOutcome::Failed {
            error: "unparseable user_id on connection".to_owned(),
        };
    };

    let fetch = fetch_provider_head(runtime, provider, parsed_user, tenant_id, params);
    let Ok(result) = timeout(per_connection, fetch).await else {
        // A bound the sweep imposed, not a verdict on the connection — a slow
        // scrape is not a dead session and must never flag one.
        warn!(
            user_id = %user_id,
            provider = %provider,
            timeout_secs = per_connection.as_secs(),
            "Capture sweep: fetch exceeded its bound"
        );
        return RefreshOutcome::Failed {
            error: format!("fetch exceeded {}s", per_connection.as_secs()),
        };
    };

    match result {
        Ok(activities) => RefreshOutcome::Refreshed {
            activities: activities.len(),
        },
        Err(e) if e.provider_auth_required_provider().is_some() => {
            flag_connection(runtime, tenant_id, parsed_user, provider).await
        }
        Err(e) => {
            warn!(
                user_id = %user_id,
                provider = %provider,
                error = %e,
                "Capture sweep: fetch failed transiently"
            );
            RefreshOutcome::Failed {
                error: e.to_string(),
            }
        }
    }
}

/// Flip a connection to `needs_reauth` so the athlete's next turn offers a
/// reconnect instead of silence.
///
/// No notification is sent from here. The athlete's next turn already consults
/// this flag and renders the reconnect link, which is the point — waking someone
/// at 4am to say a scrape session lapsed is the opposite of the quietness this
/// design buys. Flipping the status also drops the connection out of the
/// staleness snapshot, so the reader stops counting a failure that now has a
/// known reason and a stated remedy.
async fn flag_connection(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: &str,
    user_id: Uuid,
    provider: &str,
) -> RefreshOutcome {
    let Ok(tenant) = TenantId::parse_str(tenant_id) else {
        warn!(
            user_id = %user_id,
            provider = %provider,
            "Capture sweep: connection carries an unparseable tenant_id; cannot flag"
        );
        return RefreshOutcome::Failed {
            error: "unparseable tenant_id on connection".to_owned(),
        };
    };

    match runtime
        .repos()
        .provider_connections
        .mark_needs_reauth(user_id, tenant, provider, Some(FLAG_REASON))
        .await
    {
        Ok(()) => {
            info!(
                user_id = %user_id,
                provider = %provider,
                reason = FLAG_REASON,
                "Capture sweep: connection flipped to needs_reauth"
            );
            RefreshOutcome::Flagged {
                reason: FLAG_REASON.to_owned(),
            }
        }
        Err(e) => {
            warn!(
                user_id = %user_id,
                provider = %provider,
                error = %e,
                "Capture sweep: failed to flag connection"
            );
            RefreshOutcome::Failed {
                error: format!("flag failed: {e}"),
            }
        }
    }
}
