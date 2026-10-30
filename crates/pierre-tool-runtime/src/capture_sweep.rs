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

use std::collections::HashSet;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::{ReauthMark, TenantId};
use pierre_database::repositories::{ActivityFetchFailure, CaptureFreshness};
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::registry::ProviderRegistry;
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

/// Which live connections one sweep refreshes.
///
/// The daily sweep walks every live connection. Between two of them an
/// athlete's scraped capture moves only when they chat, and a chat's own read
/// outlives the turn's budget, so its result lands after the reply that
/// needed it (carnet#1206). A sweep run every few hours over the athletes who
/// chat keeps their head current without waking the scraper for everyone.
#[derive(Debug, Clone, Copy, Default)]
pub struct SweepSelection {
    /// Only the connections of athletes served — a chat turn or an app load,
    /// on any of their connections — within this long. `None` walks every
    /// live connection.
    pub active_within: Option<ChronoDuration>,
    /// Pass over connections whose last successful fetch is younger than
    /// this: a turn or an earlier sweep already refreshed them.
    pub skip_fetched_within: Option<ChronoDuration>,
}

impl SweepSelection {
    /// The athletes of `rows` this sweep treats as active: those with a serve
    /// of ANY of their connections within `active_within`. `None` when the
    /// sweep walks everyone.
    ///
    /// Activity is the athlete's, not the connection's. `last_used_at` is
    /// touched on the connection the election picked, so an athlete's other
    /// connections never carry it (11 of 24 on dev read `NULL` on
    /// 2026-10-30), and a connection-level test left every one of them to
    /// the daily sweep while their athlete chatted daily (carnet#1278).
    fn active_athletes(&self, rows: &[CaptureFreshness], now: DateTime<Utc>) -> ActiveAthletes {
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
        now: DateTime<Utc>,
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
    /// Live connections the sweep left out: a push-only provider's, which
    /// has nothing to fetch ([`capturable`]), and those its
    /// [`SweepSelection`] did not pick — not used recently enough, or
    /// fetched recently enough already.
    pub passed_over: usize,
    /// Whether the sweep reached every connection in the snapshot.
    pub completed: bool,
    /// Every connection walked, in the order it was walked.
    pub connections: Vec<ConnectionRefresh>,
}

/// Refresh the recent head of every live provider connection `selection`
/// picks, push-only providers' passed over ([`capturable`]), then retry the flagged scrape sessions whose retry is due.
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
    selection: SweepSelection,
) -> AppResult<RefreshReport> {
    let started_at = Utc::now();
    let deadline = Instant::now() + budget.total;
    let live = runtime
        .repos()
        .activity_cache
        .capture_freshness_snapshot(budget.connection_limit)
        .await?;
    let live_count = live.len();
    let (fetchable, _) = capturable(runtime.provider_registry(), live);
    let active = selection.active_athletes(&fetchable, started_at);
    let snapshot: Vec<CaptureFreshness> = fetchable
        .into_iter()
        .filter(|connection| selection.selects(connection, started_at, &active))
        .collect();
    let passed_over = live_count - snapshot.len();

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
        passed_over,
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
        passed_over,
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

#[cfg(test)]
mod tests {
    use std::slice;

    use chrono::{DateTime, Duration, Utc};
    use pierre_database::repositories::CaptureFreshness;

    use super::SweepSelection;

    fn connection(
        last_used_at: Option<DateTime<Utc>>,
        last_fetch_at: Option<DateTime<Utc>>,
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
        active_within: Some(Duration::days(7)),
        skip_fetched_within: Some(Duration::hours(2)),
    };

    /// Whether `selection` walks `row` when `row` is the athlete's only
    /// connection.
    fn selects_alone(selection: &SweepSelection, row: &CaptureFreshness) -> bool {
        let now = Utc::now();
        let active = selection.active_athletes(slice::from_ref(row), now);
        selection.selects(row, now, &active)
    }

    /// The 2026-10-27 athlete: chatted yesterday, last fetched this morning.
    #[test]
    fn an_athlete_who_chats_and_was_not_fetched_lately_is_refreshed() {
        let now = Utc::now();
        let row = connection(
            Some(now - Duration::days(1)),
            Some(now - Duration::hours(5)),
        );
        assert!(
            selects_alone(&ACTIVE_ONLY, &row),
            "bug: an active athlete's stale head must be refreshed"
        );
    }

    #[test]
    fn an_athlete_who_has_not_chatted_this_week_waits_for_the_daily_sweep() {
        let now = Utc::now();
        for used in [None, Some(now - Duration::days(8))] {
            let row = connection(used, None);
            assert!(
                !selects_alone(&ACTIVE_ONLY, &row),
                "bug: an idle athlete must not wake the scraper: {used:?}"
            );
            assert!(
                selects_alone(&SweepSelection::default(), &row),
                "bug: the daily sweep walks every live connection: {used:?}"
            );
        }
    }

    #[test]
    fn a_head_a_turn_just_refreshed_is_not_fetched_again() {
        let now = Utc::now();
        let row = connection(Some(now), Some(now - Duration::minutes(30)));
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
        let now = Utc::now();
        let elected = CaptureFreshness {
            provider: "strava".to_owned(),
            ..connection(
                Some(now - Duration::days(1)),
                Some(now - Duration::hours(1)),
            )
        };
        let untouched = CaptureFreshness {
            provider: "intervals_icu".to_owned(),
            ..connection(None, Some(now - Duration::hours(20)))
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
        let now = Utc::now();
        let chatty = connection(Some(now), Some(now - Duration::hours(5)));
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
}
