// ABOUTME: Frees Strava OAuth seats held by athletes who stopped using Dravr, under the strava_seat_reclaim.* policy
// ABOUTME: One hourly tick on spawn_periodic: observe reports who it would act on; enforce warns, then disconnects

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Strava seat reclaim
//!
//! Strava caps how many athletes one OAuth application may connect, and an
//! athlete who stops using Dravr keeps their seat until they disconnect. Once
//! the seats run out, the next athlete is sent to the Sciotte mirror instead of
//! OAuth. This sweeper gives idle seats back, under a policy an operator sets
//! with `pierre-cli config set strava_seat_reclaim.<key> <value>` (keys, defaults
//! and bounds in [`pierre_config::constants::strava_seat_reclaim`]). The policy
//! is read from the system-wide scope at the start of every pass, so a change
//! applies on the next pass without a restart.
//!
//! ## One pass
//!
//! 1. **Candidates.** Every Strava token that holds a seat (`counts_as_seat`:
//!    never a BYO-credentials athlete, never a dead grant) whose holder has been
//!    idle at least `idle_days - warn_lead_days`, least recently active first.
//!    Idle time runs from `users.last_active`, which every login and session
//!    refresh writes, every request the auth middleware authenticates (a
//!    session or `OAuth2`-connector JWT, or an API key, so an athlete who only
//!    uses Dravr from an MCP client counts as active) and every messaging turn.
//! 2. **Seats the count sees.** Pressure is measured over the env app and the
//!    enabled pool apps, each capped at its seat cap. A candidate is only ever
//!    warned or reclaimed when disconnecting them frees one of those seats: never
//!    a holder on a disabled pool app, on an app holding more athletes than its
//!    cap, or who holds the same app's seat through a token in another tenant.
//!    Such a candidate is reported as `frees_no_seat`.
//! 3. **Standing warnings.** A stored warning stands while the athlete has not
//!    been active since it, has not reconnected since it, is still a
//!    candidate, and the warning is younger than `idle_days`. In `enforce` any
//!    other is deleted, so an athlete who came back or reconnected is warned
//!    afresh if they go idle again, and a warning left over from a quiet
//!    stretch (observe, or no pressure) is sent again before it can be acted on.
//! 4. **Pressure.** Nothing is warned or reclaimed while at least
//!    `min_free_seats` seats are free, and never more than it takes to get back
//!    there, at most `max_per_tick` of each per pass:
//!    - a candidate idle `idle_days` whose warning reached them and is at
//!      least `warn_lead_days` old is disconnected through
//!      [`ProviderDisconnector`], the same chokepoint the athlete's own
//!      disconnect runs, so the grant is revoked at Strava and the seat freed
//!      on both sides; its `provider.disconnected` event carries
//!      `reason = seat_reclaim`. Their activity and warning are read again
//!      just before, so an athlete who came back during the pass keeps the
//!      seat;
//!    - candidates not yet warned are warned, as many as the shortfall leaves
//!      after the reached warnings already out, through the athlete
//!      notification path the reconnect pushes use (in-app, push, linked chat
//!      channels), in the athlete's own language, at the break-glass tier so no
//!      persona floor withholds it. A warning the pipeline suppressed is not
//!      recorded and is retried. One that reached no push device and no chat
//!      channel is recorded as unreached, so it is not sent again every pass,
//!      and it never justifies a disconnect: an idle athlete does not open the
//!      in-app list, and opening it would cancel the reclaim anyway.
//! 5. **`observe`** computes the same plan, logs one line per candidate with
//!    the action `enforce` would take, and writes nothing.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use pierre_auth::strava_pool::{strava_seat_apps, summarize_seats, StravaAppSeats};
use pierre_config::constants::strava_seat_reclaim as keys;
use pierre_core::constants::oauth_providers;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    NotificationScreen, StravaSeatHolder, StravaSeatReclaimWarning, TenantId,
};
use pierre_database::repositories::WorkerRunRepository;
use pierre_database::RepositoryRegistry;
use pierre_notifications::models::NotificationCategory;
use pierre_notifications::{
    Delivery, DispatchOutcome, EventDispatch, NotificationEvent, NotificationService, PushTier,
    TenantId as CommTenantId,
};
use pierre_runtime_context::{AdminConfigLookup, ConfigLookupScope};
use serde_json::{json, Value};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::periodic::spawn_periodic;
use crate::provider_revocation::{DisconnectReason, RevocationOutcome};
use crate::user_removal::ProviderDisconnector;

/// The name every log line and the worker ledger row carry.
pub const WORKER_NAME: &str = "strava seat reclaim";

/// Pass cadence: hourly.
///
/// The policy counts in days, so an hour is fine-grained enough that a
/// warning's lead ends within the hour it is due, and a config change applies
/// within the hour it is made.
pub const TICK_INTERVAL: StdDuration = StdDuration::from_hours(1);

/// The provider as the athlete reads it in the warning.
const PROVIDER_DISPLAY_NAME: &str = "Strava";

/// What the sweeper does with a candidate, from `strava_seat_reclaim.mode`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReclaimMode {
    /// Read the policy and stop.
    Off,
    /// Log the plan, change nothing.
    Observe,
    /// Warn, then disconnect.
    Enforce,
}

impl ReclaimMode {
    /// The mode a stored value names, or `None` for a value outside the three.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            keys::MODE_OFF => Some(Self::Off),
            keys::MODE_OBSERVE => Some(Self::Observe),
            keys::MODE_ENFORCE => Some(Self::Enforce),
            _ => None,
        }
    }

    /// The stored value, as logs print it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => keys::MODE_OFF,
            Self::Observe => keys::MODE_OBSERVE,
            Self::Enforce => keys::MODE_ENFORCE,
        }
    }
}

/// The policy one pass runs under, as the runtime configuration holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReclaimPolicy {
    /// What the pass does.
    pub mode: ReclaimMode,
    /// Idle days before a seat may be reclaimed.
    pub idle_days: i64,
    /// Days between the warning and the earliest disconnect.
    pub warn_lead_days: i64,
    /// Free seats the sweeper keeps available.
    pub min_free_seats: u32,
    /// Most disconnects, and most warnings, per pass.
    pub max_per_tick: u32,
}

impl ReclaimPolicy {
    /// Read the policy from the system-wide scope of `config`.
    ///
    /// The catalog validates every write, but a row edited in the database
    /// never went through it, so every value is checked here again and the
    /// pass refuses to run on one it would not have accepted.
    ///
    /// # Errors
    ///
    /// Returns an invalid-input error naming the key when a parameter is not
    /// registered, has the wrong type or lies outside its bounds, or when
    /// `warn_lead_days` is not below `idle_days`; and the config read's own
    /// error when it fails.
    pub async fn resolve(config: &dyn AdminConfigLookup) -> AppResult<Self> {
        let mode_value = read(config, keys::MODE_KEY).await?;
        let mode = mode_value
            .as_str()
            .and_then(ReclaimMode::parse)
            .ok_or_else(|| {
                AppError::invalid_input(format!(
                    "{} is {mode_value}, not one of {}, {}, {}",
                    keys::MODE_KEY,
                    keys::MODE_OFF,
                    keys::MODE_OBSERVE,
                    keys::MODE_ENFORCE
                ))
            })?;
        let idle_days = read_bounded(
            config,
            keys::IDLE_DAYS_KEY,
            keys::MIN_IDLE_DAYS,
            keys::MAX_IDLE_DAYS,
        )
        .await?;
        let warn_lead_days = read_bounded(
            config,
            keys::WARN_LEAD_DAYS_KEY,
            keys::MIN_WARN_LEAD_DAYS,
            keys::MAX_WARN_LEAD_DAYS,
        )
        .await?;
        if warn_lead_days >= idle_days {
            return Err(AppError::invalid_input(format!(
                "{} ({warn_lead_days}) must stay below {} ({idle_days})",
                keys::WARN_LEAD_DAYS_KEY,
                keys::IDLE_DAYS_KEY
            )));
        }
        let min_free_seats = read_count(
            config,
            keys::MIN_FREE_SEATS_KEY,
            keys::MIN_MIN_FREE_SEATS,
            keys::MAX_MIN_FREE_SEATS,
        )
        .await?;
        let max_per_tick = read_count(
            config,
            keys::MAX_PER_TICK_KEY,
            keys::MIN_MAX_PER_TICK,
            keys::MAX_MAX_PER_TICK,
        )
        .await?;
        Ok(Self {
            mode,
            idle_days,
            warn_lead_days,
            min_free_seats,
            max_per_tick,
        })
    }

    /// Idle time from which a holder is a candidate and may be warned.
    fn warn_after(&self) -> Duration {
        Duration::days(self.idle_days - self.warn_lead_days)
    }

    /// Idle time from which a warned holder may be disconnected.
    fn reclaim_after(&self) -> Duration {
        Duration::days(self.idle_days)
    }

    /// How old a warning must be before its disconnect.
    fn lead(&self) -> Duration {
        Duration::days(self.warn_lead_days)
    }

    /// Age at which a warning no longer stands and is sent again.
    ///
    /// `idle_days`: an athlete told that long ago and never acted on has been
    /// idle as long again as the whole policy measures, so the notice is too
    /// old to be the one a disconnect follows. The window after the lead is
    /// `idle_days - warn_lead_days`, at least a day, since the catalog keeps
    /// the lead below `idle_days`.
    fn warning_validity(&self) -> Duration {
        Duration::days(self.idle_days)
    }
}

/// A parameter's system-wide value.
async fn read(config: &dyn AdminConfigLookup, key: &str) -> AppResult<Value> {
    config
        .get_value(key, ConfigLookupScope::global())
        .await?
        .ok_or_else(|| {
            AppError::invalid_input(format!("{key} is not a registered configuration parameter"))
        })
}

/// An integer parameter inside `min..=max`.
async fn read_bounded(
    config: &dyn AdminConfigLookup,
    key: &str,
    min: i64,
    max: i64,
) -> AppResult<i64> {
    let value = read(config, key).await?;
    value
        .as_i64()
        .filter(|n| (min..=max).contains(n))
        .ok_or_else(|| AppError::invalid_input(format!("{key} is {value}, outside {min}..={max}")))
}

/// A count parameter inside `min..=max`, whose bounds are non-negative.
async fn read_count(
    config: &dyn AdminConfigLookup,
    key: &str,
    min: i64,
    max: i64,
) -> AppResult<u32> {
    let n = read_bounded(config, key, min, max).await?;
    u32::try_from(n).map_err(|e| AppError::invalid_input(format!("{key} is {n}: {e}")))
}

/// What one pass does with a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReclaimAction {
    /// Disconnect: warned long enough ago and still idle.
    Reclaim,
    /// Send the warning.
    Warn,
    /// Warned; the lead has not run out.
    AwaitLead,
    /// Nothing this pass: enough seats are free, or the pass's budget went to
    /// holders idle longer.
    Hold,
    /// Warned, but the warning reached no push device and no chat channel, so
    /// no disconnect follows it until the athlete is warned again once it
    /// grows stale.
    Unreached,
    /// Never warned or reclaimed: disconnecting them frees no seat the count
    /// sees (a disabled pool app, an app past its cap, or the same app's seat
    /// held through another tenant's token).
    FreesNoSeat,
}

impl ReclaimAction {
    /// The action as logs print it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reclaim => "reclaim",
            Self::Warn => "warn",
            Self::AwaitLead => "await_lead",
            Self::Hold => "hold",
            Self::Unreached => "unreached",
            Self::FreesNoSeat => "frees_no_seat",
        }
    }
}

/// One seat holder idle long enough to be warned, and what the pass does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReclaimCandidate {
    /// The athlete.
    pub user_id: Uuid,
    /// Tenant the Strava token is stored under.
    pub tenant_id: TenantId,
    /// When the athlete was last active on Dravr.
    pub last_active: DateTime<Utc>,
    /// Whole days since then.
    pub idle_days: i64,
    /// The app that issued the token: `None` for the env app, else the pool
    /// app's `client_id`.
    pub app: Option<String>,
    /// Whether disconnecting them frees a seat the count sees.
    pub frees_seat: bool,
    /// The standing warning, if one stands.
    pub warning: Option<StravaSeatReclaimWarning>,
    /// What the pass does with them.
    pub action: ReclaimAction,
}

/// A seat an `enforce` pass disconnected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReclaimedSeat {
    /// The athlete.
    pub user_id: Uuid,
    /// Tenant the Strava token was stored under.
    pub tenant_id: TenantId,
    /// Whether Strava confirmed the grant's revocation.
    pub revocation: RevocationOutcome,
}

/// Everything one pass saw and did.
#[derive(Debug, Clone)]
pub struct ReclaimReport {
    /// The policy the pass ran under.
    pub policy: ReclaimPolicy,
    /// Free seats across the env app and every enabled pool app, before the pass.
    pub free_seats: u32,
    /// Seats short of `min_free_seats`; zero means no pressure.
    pub deficit: u32,
    /// Every candidate, least recently active first, with its planned action.
    pub candidates: Vec<ReclaimCandidate>,
    /// `(athlete, tenant)` of every warning the pass sent, recorded, and that
    /// reached the athlete outside the app.
    pub warned: Vec<(Uuid, TenantId)>,
    /// `(athlete, tenant)` of every warning the pass sent and recorded that
    /// reached no push device and no chat channel.
    pub warned_unreached: Vec<(Uuid, TenantId)>,
    /// Every seat the pass disconnected.
    pub reclaimed: Vec<ReclaimedSeat>,
    /// Planned disconnects dropped because the athlete had come back, or their
    /// warning changed, by the time their turn came.
    pub reclaims_skipped: usize,
    /// Warnings deleted because the athlete came back, reconnected, stopped
    /// being a candidate, or the warning grew stale.
    pub warnings_cleared: usize,
}

impl ReclaimReport {
    fn new(policy: ReclaimPolicy) -> Self {
        Self {
            policy,
            free_seats: 0,
            deficit: 0,
            candidates: Vec::new(),
            warned: Vec::new(),
            warned_unreached: Vec::new(),
            reclaimed: Vec::new(),
            reclaims_skipped: 0,
            warnings_cleared: 0,
        }
    }
}

/// What re-reading a planned disconnect found when its turn came.
enum ReclaimRecheck {
    /// Still idle since the warning it was planned on: disconnect.
    Due,
    /// Active since the warning: keep the seat, drop the warning.
    CameBack,
    /// The account or the warning is gone, or the warning was replaced.
    Changed,
}

/// The sweeper, with everything a pass reads and acts through.
pub struct StravaSeatReclaimer {
    /// The seat holders, the pool apps and the warning rows. Shared with the
    /// rest of the server, which is why it is an `Arc`.
    repos: Arc<RepositoryRegistry>,
    /// Where the policy lives; the admin config service, shared with its routes.
    config: Arc<dyn AdminConfigLookup>,
    /// The disconnect chokepoint, shared with the admin routes.
    disconnector: Arc<dyn ProviderDisconnector>,
    /// The athlete notification path; `None` in a build without notifications,
    /// where no warning can be sent and so nothing is ever reclaimed.
    notifications: Option<Arc<NotificationService>>,
}

impl StravaSeatReclaimer {
    /// A sweeper over these collaborators.
    #[must_use]
    pub fn new(
        repos: Arc<RepositoryRegistry>,
        config: Arc<dyn AdminConfigLookup>,
        disconnector: Arc<dyn ProviderDisconnector>,
        notifications: Option<Arc<NotificationService>>,
    ) -> Self {
        Self {
            repos,
            config,
            disconnector,
            notifications,
        }
    }

    /// Run one pass as of `now`.
    ///
    /// # Errors
    ///
    /// Returns an error when the policy cannot be read or is invalid, or when
    /// listing the holders, the seat counts or a warning fails. A failed warning
    /// or disconnect is logged and retried on the next pass instead.
    pub async fn tick(&self, now: DateTime<Utc>) -> AppResult<ReclaimReport> {
        let policy = ReclaimPolicy::resolve(self.config.as_ref()).await?;
        let mut report = ReclaimReport::new(policy);
        if policy.mode == ReclaimMode::Off {
            debug!(worker = WORKER_NAME, "strava seat reclaim is off");
            return Ok(report);
        }

        let apps = strava_seat_apps(self.repos.oauth_tokens.as_ref()).await?;
        report.free_seats = summarize_seats(&apps).left();
        report.deficit = policy.min_free_seats.saturating_sub(report.free_seats);

        self.collect_candidates(&policy, &apps, now, &mut report)
            .await?;
        plan(&mut report.candidates, &policy, report.deficit, now);
        log_candidates(&report);

        if policy.mode == ReclaimMode::Enforce {
            self.enforce(&mut report, now).await;
        }
        log_pass(&report);
        Ok(report)
    }

    /// Every seat holder idle long enough to be a candidate, least recently
    /// active first, into `report`.
    async fn collect_candidates(
        &self,
        policy: &ReclaimPolicy,
        apps: &[StravaAppSeats],
        now: DateTime<Utc>,
        report: &mut ReclaimReport,
    ) -> AppResult<()> {
        let holders = self.repos.oauth_tokens.list_strava_seat_holders().await?;
        let counting: Vec<&StravaSeatHolder> = holders
            .iter()
            .filter(|holder| holder.counts_as_seat)
            .collect();
        // Seats are counted per athlete and app, so a token whose athlete
        // holds the same app's seat through another tenant frees nothing.
        let mut tokens_per_seat: HashMap<(Uuid, Option<&str>), usize> = HashMap::new();
        for holder in &counting {
            *tokens_per_seat
                .entry((holder.user_id, holder.oauth_app_client_id.as_deref()))
                .or_default() += 1;
        }
        for holder in counting {
            let app = holder.oauth_app_client_id.as_deref();
            let frees_seat = tokens_per_seat
                .get(&(holder.user_id, app))
                .is_some_and(|tokens| *tokens == 1)
                && apps
                    .iter()
                    .find(|seats| seats.attribution.as_deref() == app)
                    .is_some_and(StravaAppSeats::departure_frees_a_seat);
            if let Some(candidate) = self
                .candidate(holder, frees_seat, policy, now, report)
                .await?
            {
                report.candidates.push(candidate);
            }
        }
        report.candidates.sort_by(|a, b| {
            a.last_active
                .cmp(&b.last_active)
                .then_with(|| a.user_id.cmp(&b.user_id))
        });
        Ok(())
    }

    /// `holder` as a candidate, or `None` when they are not idle long enough,
    /// or have no account row to measure activity on or to warn. In `enforce`
    /// a warning that no longer stands is deleted here.
    async fn candidate(
        &self,
        holder: &StravaSeatHolder,
        frees_seat: bool,
        policy: &ReclaimPolicy,
        now: DateTime<Utc>,
        report: &mut ReclaimReport,
    ) -> AppResult<Option<ReclaimCandidate>> {
        let Some(last_active) = holder.last_active else {
            return Ok(None);
        };
        let Ok(tenant_id) = TenantId::parse_str(&holder.tenant_id) else {
            warn!(
                worker = WORKER_NAME,
                user_id = %holder.user_id,
                tenant_id = %holder.tenant_id,
                "strava seat holder's token names an invalid tenant id; skipped"
            );
            return Ok(None);
        };
        let idle = now - last_active;
        let is_candidate = idle >= policy.warn_after();
        let warnings = &self.repos.strava_seat_reclaim_warnings;
        let warning = warnings
            .get_seat_reclaim_warning(holder.user_id, tenant_id)
            .await?;
        let standing = warning.filter(|w| {
            is_candidate
                && last_active <= w.warned_at
                && holder.connected_at <= w.warned_at
                && now - w.warned_at < policy.warning_validity()
        });

        if policy.mode == ReclaimMode::Enforce && warning.is_some() && standing.is_none() {
            warnings
                .clear_seat_reclaim_warning(holder.user_id, tenant_id)
                .await?;
            report.warnings_cleared += 1;
            info!(
                worker = WORKER_NAME,
                user_id = %holder.user_id,
                tenant_id = %tenant_id,
                "strava seat reclaim warning withdrawn: the athlete came back, reconnected or is no longer idle, or the warning grew stale"
            );
        }
        if !is_candidate {
            return Ok(None);
        }
        Ok(Some(ReclaimCandidate {
            user_id: holder.user_id,
            tenant_id,
            last_active,
            idle_days: idle.num_days(),
            app: holder.oauth_app_client_id.clone(),
            frees_seat,
            warning: standing,
            action: ReclaimAction::Hold,
        }))
    }

    /// Carry out the planned disconnects, then the planned warnings.
    ///
    /// Disconnects go first: each one was planned on activity read at the
    /// start of the pass, and a warning's push and channel sends take time
    /// the athlete could spend coming back.
    async fn enforce(&self, report: &mut ReclaimReport, now: DateTime<Utc>) {
        let planned: Vec<ReclaimCandidate> = report
            .candidates
            .iter()
            .filter(|c| matches!(c.action, ReclaimAction::Reclaim | ReclaimAction::Warn))
            .cloned()
            .collect();
        for candidate in planned
            .iter()
            .filter(|c| c.action == ReclaimAction::Reclaim)
        {
            match self.recheck(candidate).await {
                Some(ReclaimRecheck::Due) => {
                    if let Some(seat) = self.reclaim(candidate).await {
                        report.reclaimed.push(seat);
                    }
                }
                Some(ReclaimRecheck::CameBack) => {
                    report.reclaims_skipped += 1;
                    self.withdraw_warning(candidate).await;
                    report.warnings_cleared += 1;
                }
                Some(ReclaimRecheck::Changed) | None => report.reclaims_skipped += 1,
            }
        }
        for candidate in planned.iter().filter(|c| c.action == ReclaimAction::Warn) {
            match self.warn(candidate, &report.policy, now).await {
                Some(true) => report.warned.push((candidate.user_id, candidate.tenant_id)),
                Some(false) => report
                    .warned_unreached
                    .push((candidate.user_id, candidate.tenant_id)),
                None => {}
            }
        }
    }

    /// Read a planned disconnect's activity and warning again, right before
    /// it; `None` (logged) when the read failed, so the next pass decides.
    async fn recheck(&self, candidate: &ReclaimCandidate) -> Option<ReclaimRecheck> {
        let planned_on = candidate.warning?;
        let read = async {
            let user = self.repos.users.get_global(candidate.user_id).await?;
            let warning = self
                .repos
                .strava_seat_reclaim_warnings
                .get_seat_reclaim_warning(candidate.user_id, candidate.tenant_id)
                .await?;
            AppResult::Ok((user, warning))
        };
        let (user, warning) = read
            .await
            .inspect_err(|e| {
                warn!(
                    worker = WORKER_NAME,
                    user_id = %candidate.user_id,
                    error = %e,
                    "strava seat reclaim could not re-read the athlete before disconnecting; retried next pass"
                );
            })
            .ok()?;
        let verdict = match (user, warning) {
            (Some(user), Some(warning)) if warning == planned_on => {
                if user.last_active > warning.warned_at {
                    ReclaimRecheck::CameBack
                } else {
                    ReclaimRecheck::Due
                }
            }
            _ => ReclaimRecheck::Changed,
        };
        if !matches!(verdict, ReclaimRecheck::Due) {
            info!(
                worker = WORKER_NAME,
                user_id = %candidate.user_id,
                tenant_id = %candidate.tenant_id,
                came_back = matches!(verdict, ReclaimRecheck::CameBack),
                "strava seat reclaim skipped: the athlete came back or their warning changed during the pass"
            );
        }
        Some(verdict)
    }

    /// Disconnect a candidate's Strava through the chokepoint and delete the
    /// warning that made it legitimate.
    async fn reclaim(&self, candidate: &ReclaimCandidate) -> Option<ReclaimedSeat> {
        if !self.disconnector.supports(oauth_providers::STRAVA) {
            error!(
                worker = WORKER_NAME,
                "this build cannot disconnect strava; no seat can be reclaimed"
            );
            return None;
        }
        let revocation = self.disconnect(candidate).await?;
        self.withdraw_warning(candidate).await;
        info!(
            worker = WORKER_NAME,
            user_id = %candidate.user_id,
            tenant_id = %candidate.tenant_id,
            idle_days = candidate.idle_days,
            revocation = ?revocation,
            "strava seat reclaimed from an idle athlete"
        );
        Some(ReclaimedSeat {
            user_id: candidate.user_id,
            tenant_id: candidate.tenant_id,
            revocation,
        })
    }

    /// The chokepoint's disconnect, as a seat reclaim; `None` (logged) when it
    /// failed, so the next pass retries it.
    async fn disconnect(&self, candidate: &ReclaimCandidate) -> Option<RevocationOutcome> {
        self.disconnector
            .disconnect(
                candidate.user_id,
                oauth_providers::STRAVA,
                candidate.tenant_id,
                DisconnectReason::SeatReclaim,
            )
            .await
            .inspect_err(|e| {
                warn!(
                    worker = WORKER_NAME,
                    user_id = %candidate.user_id,
                    tenant_id = %candidate.tenant_id,
                    error = %e,
                    "strava seat reclaim disconnect failed; retried next pass"
                );
            })
            .ok()
    }

    /// Delete a candidate's warning row.
    async fn withdraw_warning(&self, candidate: &ReclaimCandidate) {
        if let Err(e) = self
            .repos
            .strava_seat_reclaim_warnings
            .clear_seat_reclaim_warning(candidate.user_id, candidate.tenant_id)
            .await
        {
            warn!(
                worker = WORKER_NAME,
                user_id = %candidate.user_id,
                error = %e,
                "strava seat reclaim warning row was not deleted"
            );
        }
    }

    /// Send a candidate their warning and record it: `Some(reached)` once
    /// recorded, `None` when nothing was recorded.
    ///
    /// A warning suppressed by the athlete's quiet hours or turned-off
    /// category reached nobody and left no row, so it is sent again on the
    /// next pass. One the pipeline persisted is recorded either way, with
    /// whether it reached the athlete outside the app, so an unreached one is
    /// not repeated every pass and never counts towards a disconnect.
    async fn warn(
        &self,
        candidate: &ReclaimCandidate,
        policy: &ReclaimPolicy,
        now: DateTime<Utc>,
    ) -> Option<bool> {
        let Some(notifications) = &self.notifications else {
            warn!(
                worker = WORKER_NAME,
                user_id = %candidate.user_id,
                "no notification service in this build: the athlete cannot be warned, so their seat is kept"
            );
            return None;
        };
        let reached = deliver_warning(notifications, candidate, policy).await?;
        self.record_warning(candidate, policy, now, reached)
            .await
            .then_some(reached)
    }

    /// Record a delivered warning; `true` once stored.
    async fn record_warning(
        &self,
        candidate: &ReclaimCandidate,
        policy: &ReclaimPolicy,
        now: DateTime<Utc>,
        reached: bool,
    ) -> bool {
        match self
            .repos
            .strava_seat_reclaim_warnings
            .record_seat_reclaim_warning(candidate.user_id, candidate.tenant_id, now, reached)
            .await
        {
            Ok(()) => {
                info!(
                    worker = WORKER_NAME,
                    user_id = %candidate.user_id,
                    tenant_id = %candidate.tenant_id,
                    idle_days = candidate.idle_days,
                    lead_days = policy.warn_lead_days,
                    reached,
                    "strava seat reclaim warning sent"
                );
                true
            }
            Err(e) => {
                warn!(
                    worker = WORKER_NAME,
                    user_id = %candidate.user_id,
                    error = %e,
                    "strava seat reclaim warning sent but not recorded; it is sent again next pass"
                );
                false
            }
        }
    }
}

/// Dispatch a candidate's warning: `Some(reached)` when the pipeline
/// persisted it, with whether it reached a push device or a chat channel;
/// `None` when it was suppressed or failed.
async fn deliver_warning(
    notifications: &NotificationService,
    candidate: &ReclaimCandidate,
    policy: &ReclaimPolicy,
) -> Option<bool> {
    // P0, which no persona floor withholds: the disconnect that follows is
    // only fair if the notice got through, and a floor would leave it in the
    // in-app list and a weekly digest, neither of which an idle athlete sees
    // inside the lead.
    match notifications
        .dispatch_event(&warning_dispatch(candidate, policy), PushTier::P0)
        .await
    {
        Ok(Delivery {
            outcome: DispatchOutcome::Suppressed(reason),
            ..
        }) => {
            info!(
                worker = WORKER_NAME,
                user_id = %candidate.user_id,
                reason = ?reason,
                "strava seat reclaim warning suppressed by the athlete's notification settings; not recorded"
            );
            None
        }
        Ok(delivery) => Some(delivery.reached_outside_the_app()),
        Err(e) => {
            warn!(
                worker = WORKER_NAME,
                user_id = %candidate.user_id,
                error = %e,
                "strava seat reclaim warning dispatch failed; retried next pass"
            );
            None
        }
    }
}

/// The warning event for a candidate: rendered in their locale from the
/// catalogue, and routed to the connections screen.
fn warning_dispatch(candidate: &ReclaimCandidate, policy: &ReclaimPolicy) -> EventDispatch {
    EventDispatch {
        user_id: candidate.user_id,
        tenant_id: CommTenantId(candidate.tenant_id.as_uuid()),
        category: NotificationCategory::System,
        event: NotificationEvent::SeatReleaseWarning,
        params: json!({
            "provider_name": PROVIDER_DISPLAY_NAME,
            "idle_days": candidate.idle_days.to_string(),
            "days_left": policy.warn_lead_days.to_string(),
        }),
        route: json!({
            "screen": NotificationScreen::Connections.as_str(),
            "provider": oauth_providers::STRAVA,
        }),
        actions: None,
        bypass_frequency_cap: true,
    }
}

/// One line per candidate, with the action the pass takes (or, in observe,
/// the one enforce would take). The athlete is named by id only.
fn log_candidates(report: &ReclaimReport) {
    for candidate in &report.candidates {
        info!(
            worker = WORKER_NAME,
            mode = report.policy.mode.as_str(),
            user_id = %candidate.user_id,
            tenant_id = %candidate.tenant_id,
            idle_days = candidate.idle_days,
            app = candidate.app.as_deref().unwrap_or("env"),
            frees_seat = candidate.frees_seat,
            warned = candidate.warning.is_some(),
            warning_reached = candidate.warning.is_some_and(|w| w.reached),
            action = candidate.action.as_str(),
            "strava seat reclaim candidate"
        );
    }
}

/// The pass's summary line.
fn log_pass(report: &ReclaimReport) {
    info!(
        worker = WORKER_NAME,
        mode = report.policy.mode.as_str(),
        free_seats = report.free_seats,
        min_free_seats = report.policy.min_free_seats,
        deficit = report.deficit,
        candidates = report.candidates.len(),
        warned = report.warned.len(),
        warned_unreached = report.warned_unreached.len(),
        reclaimed = report.reclaimed.len(),
        reclaims_skipped = report.reclaims_skipped,
        warnings_cleared = report.warnings_cleared,
        "strava seat reclaim pass"
    );
}

/// Decide each candidate's action, in order (least recently active first).
///
/// A candidate whose disconnect frees no counted seat is never warned or
/// reclaimed, and a warning that reached nobody is never acted on. The pass
/// may disconnect `min(deficit, max_per_tick)` candidates whose reached
/// warning's lead has run out. It then warns only as many as the shortfall
/// still needs once the reached warnings already out are counted, so an
/// athlete is warned when their disconnect is likely rather than whenever
/// they are idle.
fn plan(
    candidates: &mut [ReclaimCandidate],
    policy: &ReclaimPolicy,
    deficit: u32,
    now: DateTime<Utc>,
) {
    let reclaim_budget = deficit.min(policy.max_per_tick);
    let mut reclaims = 0_u32;
    let mut outstanding = 0_u32;
    for candidate in candidates.iter_mut() {
        if !candidate.frees_seat {
            candidate.action = ReclaimAction::FreesNoSeat;
            continue;
        }
        let Some(warning) = candidate.warning else {
            continue;
        };
        if !warning.reached {
            candidate.action = ReclaimAction::Unreached;
            continue;
        }
        outstanding = outstanding.saturating_add(1);
        let due = now - candidate.last_active >= policy.reclaim_after()
            && now - warning.warned_at >= policy.lead();
        candidate.action = if !due {
            ReclaimAction::AwaitLead
        } else if reclaims < reclaim_budget {
            reclaims += 1;
            ReclaimAction::Reclaim
        } else {
            ReclaimAction::Hold
        };
    }

    let mut warn_budget = deficit.saturating_sub(outstanding).min(policy.max_per_tick);
    for candidate in candidates
        .iter_mut()
        .filter(|c| c.frees_seat && c.warning.is_none())
    {
        if warn_budget == 0 {
            break;
        }
        candidate.action = ReclaimAction::Warn;
        warn_budget -= 1;
    }
}

/// Start the sweeper on [`spawn_periodic`].
///
/// One pass every [`TICK_INTERVAL`], due per the worker ledger, leased across
/// instances, surviving a failed or panicking pass.
pub fn start_strava_seat_reclaimer(
    reclaimer: Arc<StravaSeatReclaimer>,
    ledger: Arc<dyn WorkerRunRepository>,
) {
    spawn_periodic(WORKER_NAME, TICK_INTERVAL, ledger, move || {
        let reclaimer = Arc::clone(&reclaimer);
        async move { reclaimer.tick(Utc::now()).await.map(|_| ()) }
    });
}
