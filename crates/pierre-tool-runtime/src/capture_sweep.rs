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
//! A flag on a signed-in session is one read's verdict, and the service can be
//! wrong about a session that is fine, so the sweep also retries flagged signed-in
//! sessions on a throttle ([`crate::reauth_retry`]) — the same headless read,
//! never a login. One the session serves is re-armed.
//!
//! That constraint is not fastidiousness. A fresh service login can demand a 2FA
//! phone tap within a four-minute window, and the service scales to zero
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
//!
//! # Grants beside a session, and the grants themselves
//!
//! A usr/pwd session is its provider's primary source (carnet#1494): the
//! sweep refreshes it, and passes over the OAuth grant standing beside it
//! ([`grant_beside_session`]), which only answers when the session cannot,
//! so the API is never read beside the session. A push-only grant has no head
//! to refresh, so the daily sweep walks it through the token path instead
//! ([`grant_check`]): a refresh the provider refuses flags the grant even for
//! an athlete who records nothing, and a live grant keeps refreshing.

mod grant_check;

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{ReauthMark, TenantId};
use pierre_database::repositories::{ActivityFetchFailure, CaptureFreshness};
use pierre_providers::backend_resolver::grant_beside_session;
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::request_budget::{in_background, refused_before_send};
use pierre_providers::sciotte_error::is_coach_account_refusal;
use serde::Serialize;
use tokio::time::timeout;
use tracing::{info, warn};
use uuid::Uuid;

#[cfg(feature = "tools-data")]
use crate::activity_backfill::after_refresh;
use crate::activity_fetch::fetch_provider_head;
use crate::activity_fetch::sync_verdict::{
    covers_list_head, held_to_background_share, record_sync_failure,
};
use crate::protocol::reauth_notice::flag_needs_reauth;
use crate::reauth_retry::{claim_flagged_session_retry, FLAGGED_SESSION_RETRY_INTERVAL_HOURS};
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
/// A session-read fetch drives a headless browser and is measured in tens of
/// seconds. This bounds the *fetch*, never a login — the sweep does not log in,
/// so the service's four-minute phone-tap window is not in play here.
pub const DEFAULT_CONNECTION_TIMEOUT_SECS: u64 = 90;

/// Whole-sweep bound.
///
/// The deployed API's request timeout is 600 s and this runs inline on a request
/// (the service scales to zero, so a detached task has no CPU to run on once the
/// response is sent). Finishing under the budget with an honest "did not reach
/// these" beats a 504 that reports nothing at all.
pub const DEFAULT_SWEEP_BUDGET_SECS: u64 = 480;

/// Reason recorded on a connection this sweep flags, and on one a Home
/// refresh found refused ([`fetch_provider_activities`](crate::activity_fetch::fetch_provider_activities)),
/// or the health sync found dead.
///
/// Matches the vocabulary the backfill notifier already writes, so a reader of
/// `provider_connections.last_error` sees one word for one condition regardless
/// of which path noticed it.
pub const FLAG_REASON: &str = "session_expired";

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

/// Which live connections one sweep refreshes.
///
/// The daily sweep walks every live connection. Between two of them an
/// athlete's session-read capture moves only when they chat, and a chat's own read
/// outlives the turn's budget, so its result lands after the reply that
/// needed it (carnet#1206). A sweep run every few hours over the athletes who
/// chat keeps their head current without waking the service for everyone.
#[derive(Debug, Clone, Copy, Default)]
pub struct SweepSelection {
    /// Only the connections of athletes served — a chat turn or an app load,
    /// on any of their connections — within this long. `None` walks every
    /// live connection.
    pub active_within: Option<Elapsed>,
    /// Pass over connections whose last successful fetch is younger than
    /// this: a turn or an earlier sweep already refreshed them.
    pub skip_fetched_within: Option<Elapsed>,
}

/// The widest window, in hours, either bound of a [`SweepSelection`] takes:
/// past it the sweep's `now - window` would leave the range the date types hold.
const MAX_SELECTION_HOURS: i64 = 720;

impl SweepSelection {
    /// The selection the hour bounds a caller names, each `None` when not
    /// given. Either one given is clamped to at most `MAX_SELECTION_HOURS`.
    ///
    /// An activity window of zero hours would select nobody, so it is clamped
    /// to one. A skip window of zero passes over nothing: every connection of
    /// an active athlete is due, a head their own turn read minutes ago
    /// included, which is what an admin asking the lane to skip nothing
    /// expects (carnet#1692).
    #[must_use]
    pub fn from_hours(
        active_within_hours: Option<u32>,
        skip_fetched_within_hours: Option<u32>,
    ) -> Self {
        let hours = |h: u32| Elapsed::hours(i64::from(h).clamp(1, MAX_SELECTION_HOURS));
        Self {
            active_within: active_within_hours.map(hours),
            skip_fetched_within: skip_fetched_within_hours.filter(|&h| h > 0).map(hours),
        }
    }

    /// The athletes of `rows` this sweep treats as active: those with a serve
    /// of ANY of their connections within `active_within`. `None` when the
    /// sweep walks everyone.
    ///
    /// Activity is the athlete's, not the connection's. `last_used_at` is
    /// touched on the connection the election picked, so an athlete's other
    /// connections never carry it (11 of 24 on dev read `NULL` on
    /// 2026-10-30), and a connection-level test left every one of them to
    /// the daily sweep while their athlete chatted daily (carnet#1278).
    fn active_athletes(&self, rows: &[CaptureFreshness], now: Timestamp) -> ActiveAthletes {
        let Some(window) = self.active_within else {
            return ActiveAthletes(None);
        };
        let since = now - window;
        ActiveAthletes(Some(
            rows.iter()
                .filter(|row| row.last_used_at.is_some_and(|used| used >= since))
                .map(athlete_key)
                .collect(),
        ))
    }

    /// Whether `connection` is one this sweep refreshes at `now`: its athlete
    /// is active and no fetch reached it within `skip_fetched_within`.
    fn selects(
        &self,
        connection: &CaptureFreshness,
        now: Timestamp,
        active: &ActiveAthletes,
    ) -> bool {
        let due = self.skip_fetched_within.is_none_or(|window| {
            connection
                .last_fetch_at
                .is_none_or(|fetched| fetched < now - window)
        });
        active.covers(connection) && due
    }
}

/// The athletes a sweep walks, keyed by tenant and user; `None` is everyone.
struct ActiveAthletes(Option<HashSet<(String, String)>>);

impl ActiveAthletes {
    fn covers(&self, connection: &CaptureFreshness) -> bool {
        self.0
            .as_ref()
            .is_none_or(|active| active.contains(&athlete_key(connection)))
    }
}

/// One athlete across their connections: the tenant and user a row belongs to.
fn athlete_key(row: &CaptureFreshness) -> (String, String) {
    (row.tenant_id.clone(), row.user_id.clone())
}

/// The connections of `snapshot` a capture can be made of, and how many
/// were left out.
///
/// A push-only provider ([`ProviderRegistry::is_push_only`]) has no list to
/// read: its data arrives when the provider pushes it, and its fetch mark
/// moves only then. The sweep has nothing to refresh for it, and the
/// staleness reader would read an athlete who simply recorded nothing as a
/// frozen capture, so both leave such a connection out of what they judge.
#[must_use]
pub fn capturable(
    registry: &ProviderRegistry,
    snapshot: Vec<CaptureFreshness>,
) -> (Vec<CaptureFreshness>, usize) {
    let total = snapshot.len();
    let kept: Vec<CaptureFreshness> = snapshot
        .into_iter()
        .filter(|connection| !registry.is_push_only(&connection.provider))
        .collect();
    let left_out = total - kept.len();
    (kept, left_out)
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
        /// Error text, for the admin reading the report.
        error: String,
    },
    /// The sweep's background share of the provider's request budget was
    /// spent, so the request limiter held the read back before anything was
    /// sent (carnet#1488). That is the share doing its job (carnet#1491), not a
    /// failed read: the athlete's own read still has the rest of the budget,
    /// and a later sweep tries again. Whether the capture still goes stale is
    /// the staleness monitor's to judge. A `429` the provider itself answered
    /// was sent, and is [`RefreshOutcome::Failed`].
    Deferred {
        /// The refusal, for the admin reading the report.
        error: String,
    },
    /// The service refused the read because the account is a coach's, which
    /// keeps no calendar of its own (a TrainingPeaks coach account): the
    /// account's standing state, not a failed read, so it is never flagged.
    /// The head read still records it, so the backoff spaces the reads it
    /// would otherwise repeat. Its athletes are read through the groups it
    /// coaches.
    CoachAccount,
    /// The sweep ran out of its time budget before reaching this connection.
    SkippedBudgetExhausted,
    /// A push-only grant the daily sweep walked through the token path
    /// ([`grant_check`]) still holds a token its provider honours.
    GrantValid,
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
    pub started_at: Timestamp,
    /// When it finished.
    pub finished_at: Timestamp,
    /// Connections the sweep attempted a fetch for.
    pub attempted: usize,
    /// Attempts whose provider answered.
    pub refreshed: usize,
    /// Attempts that left their connection `needs_reauth`.
    pub flagged: usize,
    /// Attempts that failed transiently.
    pub failed: usize,
    /// Attempts held back by the sweep's background share of the provider's
    /// request budget ([`RefreshOutcome::Deferred`]), never counted as failed.
    pub deferred: usize,
    /// Attempts the service refused because the account is a coach's
    /// ([`RefreshOutcome::CoachAccount`]), never counted as failed.
    pub coach_accounts: usize,
    /// Connections the time budget left unreached.
    pub skipped: usize,
    /// Flagged signed-in sessions the sweep retried, each also counted in
    /// `attempted` and in the outcome it reached.
    pub retried: usize,
    /// Push-only grants the daily sweep walked through the token path
    /// ([`grant_check`]), each also counted in `flagged`, `failed` or
    /// `skipped` when it reached that outcome.
    pub grants_checked: usize,
    /// Live connections the sweep left out: a push-only provider's, which
    /// has nothing to fetch ([`capturable`]), an OAuth grant beside a
    /// usr/pwd session of its provider ([`grant_beside_session`]), and those its
    /// [`SweepSelection`] did not pick — not used recently enough, or
    /// fetched recently enough already.
    pub passed_over: usize,
    /// Whether the sweep reached every connection in the snapshot.
    pub completed: bool,
    /// Every connection walked, in the order it was walked.
    pub connections: Vec<ConnectionRefresh>,
}

/// Refresh the recent head of every live provider connection `selection`
/// picks, then retry the flagged signed-in sessions whose retry is due.
///
/// Push-only providers' connections and grants beside a session of their
/// provider are passed over ([`capturable`], [`grant_beside_session`]). The
/// daily sweep (no `active_within`) then walks every live push-only grant
/// through the token path ([`grant_check`]).
///
/// Walks the same snapshot the staleness reader judges — which already excludes
/// connections needing re-auth, since a known-dead one has nothing to refresh.
/// A flagged signed-in session is the exception, walked after the live ones by
/// [`retry_flagged_sessions`] on its own throttle; a flagged OAuth connection
/// is never re-attempted.
///
/// Never attempts a login. Each connection is fetched through the shared
/// write-through path; an auth-shaped failure flags the connection with
/// [`FLAG_REASON`] and the sweep moves on.
///
/// Nobody waits on the sweep, so it runs [`in_background`]: each provider
/// request it makes is admitted only inside the background share of the
/// signing app's windows, and the rest stays with the reads an athlete is
/// waiting on. On 2026-11-10 it read every Strava head at the full share, and
/// once a runaway backfill had spent the app's day each read was refused as a
/// spent foreground budget (carnet#1491).
///
/// # Errors
///
/// Returns an error only when the connection snapshot itself cannot be read. A
/// per-connection failure is reported in [`RefreshReport::connections`], never
/// propagated — one dead provider must not stop the sweep of every other.
pub async fn refresh_captures(
    runtime: &Arc<dyn ToolRuntime>,
    budget: SweepBudget,
    selection: SweepSelection,
) -> AppResult<RefreshReport> {
    in_background(sweep(runtime, budget, selection)).await
}

/// [`refresh_captures`]' walk, run in whatever share of the budget the caller
/// gives it.
async fn sweep(
    runtime: &Arc<dyn ToolRuntime>,
    budget: SweepBudget,
    selection: SweepSelection,
) -> AppResult<RefreshReport> {
    let started_at = Timestamp::now();
    let deadline = Instant::now() + budget.total;
    let live = runtime
        .repos()
        .activity_cache
        .capture_freshness_snapshot(budget.connection_limit)
        .await?;
    let live_count = live.len();
    let grants = grant_check::pushed_grants(runtime.provider_registry(), &live);
    let (fetchable, _) = capturable(runtime.provider_registry(), live);
    let active = selection.active_athletes(&fetchable, started_at);
    let mut snapshot = Vec::with_capacity(fetchable.len());
    for connection in fetchable {
        if selection.selects(&connection, started_at, &active)
            && !beside_session(runtime, &connection).await
        {
            snapshot.push(connection);
        }
    }
    let passed_over = live_count - snapshot.len();

    let after_ts = (started_at - Elapsed::days(HEAD_WINDOW_DAYS)).epoch_seconds();
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
    let grants_checked = if selection.active_within.is_none() {
        grant_check::check_grants(&mut walk, grants).await
    } else {
        0
    };

    let Walk {
        tally, connections, ..
    } = walk;
    let completed = tally.skipped == 0;

    info!(
        attempted = tally.attempted,
        refreshed = tally.refreshed,
        flagged = tally.flagged,
        failed = tally.failed,
        deferred = tally.deferred,
        coach_accounts = tally.coach_accounts,
        skipped = tally.skipped,
        retried,
        grants_checked,
        passed_over,
        completed,
        "Capture sweep finished"
    );

    Ok(RefreshReport {
        started_at,
        finished_at: Timestamp::now(),
        attempted: tally.attempted,
        refreshed: tally.refreshed,
        flagged: tally.flagged,
        failed: tally.failed,
        deferred: tally.deferred,
        coach_accounts: tally.coach_accounts,
        skipped: tally.skipped,
        retried,
        grants_checked,
        passed_over,
        completed,
        connections,
    })
}

/// Whether `connection` is an OAuth grant beside a usr/pwd session of its
/// provider ([`grant_beside_session`]), which the sweep never refreshes: the
/// session is read in its place. A row whose ids do not parse is not one: it
/// is walked, and reported as it fails.
async fn beside_session(runtime: &Arc<dyn ToolRuntime>, connection: &CaptureFreshness) -> bool {
    let (Ok(user_id), Ok(tenant)) = (
        Uuid::from_str(&connection.user_id),
        TenantId::parse_str(&connection.tenant_id),
    ) else {
        return false;
    };
    grant_beside_session(runtime.repos(), user_id, tenant, &connection.provider).await
}

/// The report's counters, as one sweep accumulates them.
#[derive(Debug, Default)]
struct Tally {
    attempted: usize,
    refreshed: usize,
    flagged: usize,
    failed: usize,
    deferred: usize,
    coach_accounts: usize,
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
                RefreshOutcome::Deferred { .. } => self.tally.deferred += 1,
                RefreshOutcome::CoachAccount => self.tally.coach_accounts += 1,
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

/// Retry the flagged signed-in sessions whose retry is due, and report how many
/// were attempted.
///
/// Each is claimed through [`claim_flagged_session_retry`] before it is
/// fetched, so a connection Home or another replica retried within the
/// interval is left alone and does not appear in the report. The fetch is the
/// sweep's ordinary one: a session that serves it is re-armed by the live-read
/// path, and a refused one stays flagged, reported `flagged`, and is not told
/// again. Stops claiming once the time budget is spent, so an unreached
/// session keeps its claim for the next sweep. A listing that cannot be read
/// is logged and retries nothing: the live half of the sweep already ran.
async fn retry_flagged_sessions(walk: &mut Walk<'_>, limit: i64) -> usize {
    let due_before = Timestamp::now() - Elapsed::hours(FLAGGED_SESSION_RETRY_INTERVAL_HOURS);
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
        if !claim_flagged_session_retry(
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
    let attempt_started_at = Timestamp::now();
    let fetch = fetch_provider_head(runtime, provider, parsed_user, tenant_id, params);
    let Ok(result) = timeout(per_connection, fetch).await else {
        // A bound the sweep imposed, not a verdict on the connection — a slow
        // session read is not a dead session and must never flag one.
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
        Ok(activities) => {
            #[cfg(feature = "tools-data")]
            if let Ok(tenant) = TenantId::parse_str(tenant_id) {
                after_refresh::ask_walks_after_refresh(runtime, parsed_user, tenant, provider)
                    .await;
            }
            RefreshOutcome::Refreshed {
                activities: activities.len(),
            }
        }
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
        Err(e) => unflagged_outcome(&e, user_id, provider),
    }
}

/// What the sweep reports for a fetch error that leaves the connection
/// unflagged: a coach account's refusal and a read the limiter held to the
/// background share before sending it ([`refused_before_send`]) are not
/// failures (carnet#1488), and anything else, a provider's own `429`
/// included, failed transiently.
fn unflagged_outcome(error: &AppError, user_id: &str, provider: &str) -> RefreshOutcome {
    if is_coach_account_refusal(error) {
        info!(
            user_id = %user_id,
            provider = %provider,
            "Capture sweep: a coach account keeps no calendar of its own"
        );
        return RefreshOutcome::CoachAccount;
    }
    if held_to_background_share(error) && refused_before_send(error) {
        info!(
            user_id = %user_id,
            provider = %provider,
            error = %error,
            "Capture sweep: read held to the background share of the request budget"
        );
        return RefreshOutcome::Deferred {
            error: error.to_string(),
        };
    }
    warn!(
        user_id = %user_id,
        provider = %provider,
        error = %error,
        "Capture sweep: fetch failed transiently"
    );
    RefreshOutcome::Failed {
        error: error.to_string(),
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
        covers_list_head(params, Timestamp::now().epoch_seconds()),
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
/// reason and a stated remedy; a signed-in session is then revisited by the
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
    attempt_started_at: Timestamp,
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

#[cfg(test)]
mod tests {
    use std::slice;

    use dravr_tronc::dates::{Elapsed, Timestamp};
    use pierre_core::errors::{AppError, ErrorCode};
    use pierre_database::repositories::CaptureFreshness;
    use pierre_providers::request_budget::in_background;
    use pierre_providers::sciotte_error::coach_account_error;
    use serde_json::json;

    use super::{unflagged_outcome, RefreshOutcome, SweepSelection, MAX_SELECTION_HOURS};

    fn connection(
        last_used_at: Option<Timestamp>,
        last_fetch_at: Option<Timestamp>,
    ) -> CaptureFreshness {
        CaptureFreshness {
            tenant_id: "tenant".to_owned(),
            user_id: "athlete".to_owned(),
            provider: "sciotte_strava".to_owned(),
            last_used_at,
            last_fetch_at,
        }
    }

    const ACTIVE_ONLY: SweepSelection = SweepSelection {
        active_within: Some(Elapsed::days(7)),
        skip_fetched_within: Some(Elapsed::hours(2)),
    };

    /// Whether `selection` walks `row` when `row` is the athlete's only
    /// connection.
    fn selects_alone(selection: &SweepSelection, row: &CaptureFreshness) -> bool {
        let now = Timestamp::now();
        let active = selection.active_athletes(slice::from_ref(row), now);
        selection.selects(row, now, &active)
    }

    /// A window past the bound is clamped to it, so the sweep's `now - window`
    /// stays in range instead of panicking the request, and an activity window
    /// of zero hours, which would select nobody, is clamped to one.
    #[test]
    fn an_out_of_range_window_is_clamped() {
        let oversized = SweepSelection::from_hours(Some(u32::MAX), Some(u32::MAX));
        assert_eq!(
            oversized.active_within,
            Some(Elapsed::hours(MAX_SELECTION_HOURS)),
            "bug: an oversized activity window must be clamped to the bound"
        );
        assert_eq!(
            oversized.skip_fetched_within,
            Some(Elapsed::hours(MAX_SELECTION_HOURS)),
            "bug: an oversized skip window must be clamped to the bound"
        );
        assert_eq!(
            SweepSelection::from_hours(Some(0), None).active_within,
            Some(Elapsed::hours(1)),
            "bug: a zero activity window must be clamped to one hour"
        );
    }

    /// carnet#1692: a skip window of zero hours passes over nothing, so a
    /// head the athlete's own turn read 26 minutes ago is refreshed.
    #[test]
    fn a_zero_skip_window_makes_every_connection_due() {
        let selection = SweepSelection::from_hours(Some(168), Some(0));
        assert_eq!(
            selection.skip_fetched_within, None,
            "bug: a zero skip window must be no window"
        );
        let now = Timestamp::now();
        let row = connection(Some(now), Some(now - Elapsed::minutes(26)));
        assert!(
            selects_alone(&selection, &row),
            "bug: an active athlete's head fetched 26 minutes ago must be due"
        );
    }

    /// The 2026-10-27 athlete: chatted yesterday, last fetched this morning.
    #[test]
    fn an_athlete_who_chats_and_was_not_fetched_lately_is_refreshed() {
        let now = Timestamp::now();
        let row = connection(Some(now - Elapsed::days(1)), Some(now - Elapsed::hours(5)));
        assert!(
            selects_alone(&ACTIVE_ONLY, &row),
            "bug: an active athlete's stale head must be refreshed"
        );
    }

    #[test]
    fn an_athlete_who_has_not_chatted_this_week_waits_for_the_daily_sweep() {
        let now = Timestamp::now();
        for used in [None, Some(now - Elapsed::days(8))] {
            let row = connection(used, None);
            assert!(
                !selects_alone(&ACTIVE_ONLY, &row),
                "bug: an idle athlete must not wake the service: {used:?}"
            );
            assert!(
                selects_alone(&SweepSelection::default(), &row),
                "bug: the daily sweep walks every live connection: {used:?}"
            );
        }
    }

    #[test]
    fn a_head_a_turn_just_refreshed_is_not_fetched_again() {
        let now = Timestamp::now();
        let row = connection(Some(now), Some(now - Elapsed::minutes(30)));
        assert!(
            !selects_alone(&ACTIVE_ONLY, &row),
            "bug: a head fetched 30 minutes ago must be passed over"
        );
        let never_fetched = connection(Some(now), None);
        assert!(
            selects_alone(&ACTIVE_ONLY, &never_fetched),
            "bug: a head never fetched is always due"
        );
    }

    /// The 2026-10-30 athlete: Strava elected and served yesterday, an
    /// intervals.icu connection never elected, so never touched, and fetched
    /// only by the daily sweep. Activity is theirs, not the elected row's.
    #[test]
    fn an_active_athletes_untouched_connection_is_refreshed_too() {
        let now = Timestamp::now();
        let elected = CaptureFreshness {
            provider: "strava".to_owned(),
            ..connection(Some(now - Elapsed::days(1)), Some(now - Elapsed::hours(1)))
        };
        let untouched = CaptureFreshness {
            provider: "intervals_icu".to_owned(),
            ..connection(None, Some(now - Elapsed::hours(20)))
        };
        let rows = [elected.clone(), untouched.clone()];
        let active = ACTIVE_ONLY.active_athletes(&rows, now);
        assert!(
            !ACTIVE_ONLY.selects(&elected, now, &active),
            "bug: the elected head fetched an hour ago is passed over"
        );
        assert!(
            ACTIVE_ONLY.selects(&untouched, now, &active),
            "bug: an active athlete's untouched connection must be refreshed"
        );
    }

    /// A different athlete's recent serve lends nobody else activity.
    #[test]
    fn activity_does_not_cross_athletes_or_tenants() {
        let now = Timestamp::now();
        let chatty = connection(Some(now), Some(now - Elapsed::hours(5)));
        let other_user = CaptureFreshness {
            user_id: "someone-else".to_owned(),
            ..connection(None, None)
        };
        let other_tenant = CaptureFreshness {
            tenant_id: "another-tenant".to_owned(),
            ..connection(None, None)
        };
        let rows = [chatty, other_user.clone(), other_tenant.clone()];
        let active = ACTIVE_ONLY.active_athletes(&rows, now);
        assert!(
            !ACTIVE_ONLY.selects(&other_user, now, &active),
            "bug: another athlete's serve must not make this one active"
        );
        assert!(
            !ACTIVE_ONLY.selects(&other_tenant, now, &active),
            "bug: a serve in another tenant must not make this athlete active"
        );
    }

    /// carnet#1488: a refusal by the sweep's own background share and a coach
    /// account's refusal are not failed reads, and the lane gates on `failed`.
    #[tokio::test]
    async fn a_share_refusal_and_a_coach_account_are_never_failed() {
        let coach = coach_account_error("a coach account keeps no calendar");
        assert!(
            matches!(
                unflagged_outcome(&coach, "u", "sciotte_trainingpeaks"),
                RefreshOutcome::CoachAccount
            ),
            "bug: a coach account's refusal must not count as a failed read"
        );

        // The limiter's refusal, as `RequestBudget::admit` marks it.
        let mut spent = AppError::new(
            ErrorCode::ExternalRateLimited,
            "strava request budget is spent for now",
        );
        spent.details = Some(Box::new(json!({ "refused_before_send": true })));
        let in_share = in_background(async { unflagged_outcome(&spent, "u", "strava") }).await;
        assert!(
            matches!(in_share, RefreshOutcome::Deferred { .. }),
            "bug: a read the background share held back must be deferred, not failed: {in_share:?}"
        );

        let outside = unflagged_outcome(&spent, "u", "strava");
        assert!(
            matches!(outside, RefreshOutcome::Failed { .. }),
            "bug: a rate limit outside the background share is a failed read: {outside:?}"
        );
    }

    /// A `429` the provider itself answered was sent, so it is a failed read
    /// even inside the sweep's background share: the lane is the one place a
    /// throttling provider during a sweep is seen.
    #[tokio::test]
    async fn a_provider_429_in_the_sweep_is_a_failed_read() {
        let throttled = AppError::new(
            ErrorCode::ExternalRateLimited,
            "Rate limit exceeded for strava",
        );
        let outcome = in_background(async { unflagged_outcome(&throttled, "u", "strava") }).await;
        assert!(
            matches!(outcome, RefreshOutcome::Failed { .. }),
            "bug: a provider's own 429 was sent and must count as failed, not deferred: {outcome:?}"
        );
    }
}
