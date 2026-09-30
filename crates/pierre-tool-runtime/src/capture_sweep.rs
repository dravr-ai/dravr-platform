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
//! flags the connection and moves on. The flag sends the athlete the reconnect
//! notice once ([`flag_needs_reauth`]), in the app and on every chat they
//! linked, and their next turn consults it and hands back a reconnect link.
//!
//! A flag on a scrape session is one read's verdict, and the scraper can be
//! wrong about a session that is fine, so the sweep also retries flagged scrape
//! sessions on a throttle ([`crate::reauth_retry`]) — the same headless read,
//! never a login. One the session serves is re-armed.
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
use pierre_core::models::{ReauthMark, TenantId};
use pierre_database::repositories::ActivityFetchFailure;
use pierre_providers::core::ActivityQueryParams;
use serde::Serialize;
use tokio::time::timeout;
use tracing::{info, warn};
use uuid::Uuid;

use crate::activity_fetch::fetch_provider_head;
use crate::activity_fetch::sync_verdict::{covers_list_head, record_sync_failure};
use crate::protocol::reauth_notice::flag_needs_reauth;
use crate::reauth_retry::{claim_scrape_session_retry, SCRAPE_SESSION_RETRY_INTERVAL_HOURS};
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

/// Reason recorded on a connection this sweep flags, and on one a Home
/// refresh found refused ([`fetch_provider_activities`](crate::activity_fetch::fetch_provider_activities)).
///
/// Matches the vocabulary the backfill notifier already writes, so a reader of
/// `provider_connections.last_error` sees one word for one condition regardless
/// of which path noticed it.
pub(crate) const FLAG_REASON: &str = "session_expired";

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
    /// is `needs_reauth` (flipped by this attempt, or already by another path)
    /// and the athlete's next turn will offer a reconnect.
    Flagged {
        /// Reason recorded on the connection.
        reason: String,
    },
    /// The attempt failed in a way that is not the athlete's session dying — a
    /// timeout, a 5xx, an unsupported provider, a malformed stored id, or a
    /// session a reconnect replaced while the fetch ran. Recorded, never
    /// flagged: a flake is not a disconnect.
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
    /// Attempts that left their connection `needs_reauth`.
    pub flagged: usize,
    /// Attempts that failed transiently.
    pub failed: usize,
    /// Connections the time budget left unreached.
    pub skipped: usize,
    /// Flagged scrape sessions the sweep retried, each also counted in
    /// `attempted` and in the outcome it reached.
    pub retried: usize,
    /// Whether the sweep reached every connection in the snapshot.
    pub completed: bool,
    /// Every connection walked, in the order it was walked.
    pub connections: Vec<ConnectionRefresh>,
}

/// Refresh the recent head of every live provider connection, then retry the
/// flagged scrape sessions whose retry is due.
///
/// Walks the same snapshot the staleness reader judges — which already excludes
/// connections needing re-auth, since a known-dead one has nothing to refresh.
/// A flagged scrape session is the exception, walked after the live ones by
/// [`retry_flagged_sessions`] on its own throttle; a flagged OAuth connection
/// is never re-attempted.
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

    let mut walk = Walk {
        runtime,
        params: &params,
        per_connection: budget.per_connection,
        deadline,
        tally: Tally::default(),
        connections: Vec::with_capacity(snapshot.len()),
    };
    for connection in snapshot {
        walk.visit(
            connection.tenant_id,
            connection.user_id,
            connection.provider,
        )
        .await;
    }
    let retried = retry_flagged_sessions(&mut walk, budget.connection_limit).await;

    let Walk {
        tally, connections, ..
    } = walk;
    let completed = tally.skipped == 0;

    info!(
        attempted = tally.attempted,
        refreshed = tally.refreshed,
        flagged = tally.flagged,
        failed = tally.failed,
        skipped = tally.skipped,
        retried,
        completed,
        "Capture sweep finished"
    );

    Ok(RefreshReport {
        started_at,
        finished_at: Utc::now(),
        attempted: tally.attempted,
        refreshed: tally.refreshed,
        flagged: tally.flagged,
        failed: tally.failed,
        skipped: tally.skipped,
        retried,
        completed,
        connections,
    })
}

/// The report's counters, as one sweep accumulates them.
#[derive(Debug, Default)]
struct Tally {
    attempted: usize,
    refreshed: usize,
    flagged: usize,
    failed: usize,
    skipped: usize,
}

/// One sweep in progress: what every connection's fetch shares, and what the
/// walk has reported so far.
struct Walk<'a> {
    runtime: &'a Arc<dyn ToolRuntime>,
    params: &'a ActivityQueryParams,
    per_connection: Duration,
    deadline: Instant,
    tally: Tally,
    connections: Vec<ConnectionRefresh>,
}

impl Walk<'_> {
    /// Whether the time budget is spent, so no further fetch may start.
    fn exhausted(&self) -> bool {
        Instant::now() >= self.deadline
    }

    /// Fetch one connection's head (or record that the budget left it
    /// unreached) and add its line to the report.
    async fn visit(&mut self, tenant_id: String, user_id: String, provider: String) {
        let outcome = if self.exhausted() {
            self.tally.skipped += 1;
            RefreshOutcome::SkippedBudgetExhausted
        } else {
            self.tally.attempted += 1;
            let outcome = refresh_one(
                self.runtime,
                &tenant_id,
                &user_id,
                &provider,
                self.params,
                self.per_connection,
            )
            .await;
            match outcome {
                RefreshOutcome::Refreshed { .. } => self.tally.refreshed += 1,
                RefreshOutcome::Flagged { .. } => self.tally.flagged += 1,
                _ => self.tally.failed += 1,
            }
            outcome
        };
        self.connections.push(ConnectionRefresh {
            tenant_id,
            user_id,
            provider,
            outcome,
        });
    }
}

/// Retry the flagged scrape sessions whose retry is due, and report how many
/// were attempted.
///
/// Each is claimed through [`claim_scrape_session_retry`] before it is
/// fetched, so a connection Home or another replica retried within the
/// interval is left alone and does not appear in the report. The fetch is the
/// sweep's ordinary one: a session that serves it is re-armed by the live-read
/// path, and a refused one stays flagged, reported `flagged`, and is not told
/// again. Stops claiming once the time budget is spent, so an unreached
/// session keeps its claim for the next sweep. A listing that cannot be read
/// is logged and retries nothing: the live half of the sweep already ran.
async fn retry_flagged_sessions(walk: &mut Walk<'_>, limit: i64) -> usize {
    let due_before = Utc::now() - ChronoDuration::hours(SCRAPE_SESSION_RETRY_INTERVAL_HOURS);
    let due = match walk
        .runtime
        .repos()
        .provider_connections
        .list_reauth_retry_due(due_before, limit)
        .await
    {
        Ok(due) => due,
        Err(e) => {
            warn!(error = %e, "Capture sweep: could not list flagged sessions due a retry");
            return 0;
        }
    };

    let mut retried = 0;
    for connection in due {
        if walk.exhausted() {
            break;
        }
        let Ok(tenant) = TenantId::parse_str(&connection.tenant_id) else {
            continue;
        };
        if !claim_scrape_session_retry(
            walk.runtime,
            connection.user_id,
            tenant,
            &connection.provider,
        )
        .await
        {
            continue;
        }
        retried += 1;
        walk.visit(
            connection.tenant_id,
            connection.user_id.to_string(),
            connection.provider,
        )
        .await;
    }
    retried
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

    // Taken before the fetch reads the credential, so a reconnect that lands
    // while the fetch is in flight is newer than the failure it reports.
    let attempt_started_at = Utc::now();
    let fetch = fetch_provider_head(runtime, provider, parsed_user, tenant_id, params);
    let Ok(result) = timeout(per_connection, fetch).await else {
        // A bound the sweep imposed, not a verdict on the connection — a slow
        // scrape is not a dead session and must never flag one.
        return fetch_dropped(
            runtime,
            tenant_id,
            parsed_user,
            provider,
            params,
            per_connection,
        )
        .await;
    };

    match result {
        Ok(activities) => RefreshOutcome::Refreshed {
            activities: activities.len(),
        },
        Err(e) if e.provider_auth_required_provider().is_some() => {
            flag_connection(
                runtime,
                tenant_id,
                parsed_user,
                provider,
                attempt_started_at,
            )
            .await
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

/// What the sweep reports for a head fetch its bound dropped, recorded as a
/// failed sync.
///
/// The fetch was cut off before it could record anything itself, so it is
/// recorded here — when it read the list head — and Home opens on "sync
/// failed" rather than on rows that silently missed their sync.
async fn fetch_dropped(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: &str,
    user_id: Uuid,
    provider: &str,
    params: &ActivityQueryParams,
    per_connection: Duration,
) -> RefreshOutcome {
    warn!(
        user_id = %user_id,
        provider = %provider,
        timeout_secs = per_connection.as_secs(),
        "Capture sweep: fetch exceeded its bound"
    );
    if let (Ok(tenant), true) = (
        TenantId::parse_str(tenant_id),
        covers_list_head(params, Utc::now().timestamp()),
    ) {
        record_sync_failure(
            runtime,
            user_id,
            tenant,
            provider,
            ActivityFetchFailure::FetchError,
        )
        .await;
    }
    RefreshOutcome::Failed {
        error: format!("fetch exceeded {}s", per_connection.as_secs()),
    }
}

/// What the sweep reports for an auth-shaped failure, given what flagging its
/// connection found. Only a connection that now needs re-authorizing is a flag.
fn flag_outcome(user_id: Uuid, provider: &str, mark: ReauthMark) -> RefreshOutcome {
    match mark {
        ReauthMark::Flagged | ReauthMark::AlreadyFlagged => {
            info!(
                user_id = %user_id,
                provider = %provider,
                reason = FLAG_REASON,
                already_flagged = mark == ReauthMark::AlreadyFlagged,
                "Capture sweep: connection needs re-authorizing"
            );
            RefreshOutcome::Flagged {
                reason: FLAG_REASON.to_owned(),
            }
        }
        ReauthMark::ReconnectedSince => {
            info!(
                user_id = %user_id,
                provider = %provider,
                "Capture sweep: connection reconnected while its fetch ran; left active"
            );
            RefreshOutcome::Failed {
                error: "the fetch failed on a credential a reconnect replaced while it ran"
                    .to_owned(),
            }
        }
        ReauthMark::NoConnection => RefreshOutcome::Failed {
            error: "the connection was removed while its fetch ran".to_owned(),
        },
    }
}

/// Flip a connection to `needs_reauth` and tell the athlete once.
///
/// The flag and its notice go through [`flag_needs_reauth`], the path every
/// flagging site shares: the athlete hears it once per transition, in the app
/// and on every chat they linked, with a sign-in link where the provider has
/// one. Flipping the status also drops the connection out of the staleness
/// snapshot, so the reader stops counting a failure that now has a known
/// reason and a stated remedy; a scrape session is then revisited by the
/// throttled retry ([`retry_flagged_sessions`]), which can re-arm it.
///
/// A connection the athlete reconnected after `attempt_started_at` is left
/// active: the failure was the credential the fetch read, not the new one. It
/// is reported as a failed attempt rather than a flag, as is a connection
/// removed while the fetch ran, since neither was flipped.
async fn flag_connection(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: &str,
    user_id: Uuid,
    provider: &str,
    attempt_started_at: DateTime<Utc>,
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

    match flag_needs_reauth(
        runtime,
        user_id,
        tenant,
        provider,
        FLAG_REASON,
        attempt_started_at,
    )
    .await
    {
        Ok((mark, _)) => flag_outcome(user_id, provider, mark),
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
