// ABOUTME: The one engine that decides when the platform speaks first: wakes, gates, and fires each routine
// ABOUTME: Quiet hours, mid-conversation deferral and the daily cap live here once; PlatformPost decides how

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The routine engine (ADR-036, carnet#1790).
//!
//! Every post nobody asked for goes through here: the engine decides *when*,
//! and `PlatformPost` keeps deciding *how*. A routine's next firing is queued
//! as a wake ([`crate::services::routine_waker`]); when the wake is delivered
//! the engine runs the gates below, in this order, and only then the
//! routine's action:
//!
//! 1. **The routine itself.** A wake acts only while it still names the
//!    routine's next firing: a schedule changed or paused since it was queued
//!    makes it stale, and a slot from before the recipient moved zones is
//!    rescheduled instead of fired. Past its window
//!    ([`RoutineKind::window`]) the slot is skipped, never sent late.
//! 2. **Quiet hours.** The rule that holds a coach push holds a routine: the
//!    firing waits half an hour, for as long as its window lasts.
//! 3. **Not mid-conversation.** When the recipient wrote anything in the last
//!    [`CONVERSATION_QUIET`], on any surface of their own, the firing waits
//!    [`DEFER_STEP`], at most [`MAX_DEFERRALS`] times, and is then dropped as
//!    `routine.held`.
//! 4. **The daily cap.** An athlete receives at most [`ATHLETE_DAILY_CAP`]
//!    unprompted posts a local day, counted across every sender that posts
//!    through `PlatformPost`. Over it, the firing is held.
//!
//! The channel's own window — `WhatsApp`'s and Messenger's 24 hours — is the
//! last gate, applied where the post is delivered: a closed DM is held and
//! the post lands in the app (`messaging.proactive_held`).
//!
//! A routine's turn may suggest, never act: it runs read-only
//! ([`crate::services::platform_post::PostAccess::SuggestOnly`]).
//!
//! The engine also runs the platform's own scheduled jobs ([`SystemJob`]) on
//! the same waker, so no sender keeps a timer of its own.
//!
//! ## Reconcile
//!
//! A wake Cloud Tasks lost, or one never queued because the API refused it,
//! is queued again by [`RoutineEngine::reconcile`]: at every start and from
//! the 3-hourly capture cron. Wakes are named by what they fire and when, so
//! re-queuing a pending one queues nothing new.

use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, Weak};

use dravr_tronc::dates::{Elapsed, Timestamp, Zone};
use pierre_core::civil_time::resolve_zone;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Routine, RoutineKind, TenantId, UserStatus};
use pierre_database::repositories::NextFiring;
#[cfg(feature = "client-notifications")]
use pierre_services::notification_digest_scheduler;
use tokio::sync::mpsc;
use tokio::time;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::services::adapter_resolver::{config_adapter_resolver, AdapterResolver};
use crate::services::check_in_post::{run_check_in, CheckInPoster, CheckInRun};
use crate::services::platform_post::{PlatformPost, PostSurface};
use crate::services::post_delivery::{coach_quiet_hours, PostAthlete};
use crate::services::routine_waker::{
    CloudTasksWaker, InProcessWaker, RoutineWaker, SystemJob, Wake,
};

/// How long after the recipient's last message a routine waits before it
/// speaks: a post never lands in a conversation the recipient is in.
pub const CONVERSATION_QUIET: Elapsed = Elapsed::minutes(30);

/// How long a held-back firing waits before it tries again.
pub const DEFER_STEP: Elapsed = Elapsed::minutes(30);

/// How many times a firing waits for a conversation to end before it is
/// dropped as `routine.held`.
pub const MAX_DEFERRALS: u32 = 3;

/// The most unprompted posts an athlete receives in one local day, across
/// every sender that posts through `PlatformPost`.
pub const ATHLETE_DAILY_CAP: u32 = 2;

/// Why a firing was held: nothing was posted for this slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HoldReason {
    /// The slot's window passed before the firing could go out.
    WindowPassed,
    /// The recipient's quiet hours covered the rest of the window.
    QuietHours,
    /// The recipient was still mid-conversation after every deferral.
    MidConversation,
    /// The recipient already had their daily cap of unprompted posts.
    DailyCap,
    /// Nothing could be attempted (no instruction, no thread) inside the window.
    Unattempted,
}

impl HoldReason {
    /// The reason as `routine.held` reports it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WindowPassed => "window_passed",
            Self::QuietHours => "quiet_hours",
            Self::MidConversation => "mid_conversation",
            Self::DailyCap => "daily_cap",
            Self::Unattempted => "unattempted",
        }
    }
}

/// What a delivered wake did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WakeOutcome {
    /// The routine's reply grounded and was stored on that surface.
    Posted(PostSurface),
    /// The routine's turn ran on that surface and its reply was withheld.
    Withheld(PostSurface),
    /// The firing waits until then.
    Deferred(Timestamp),
    /// Nothing was posted for the slot, for that reason.
    Held(HoldReason),
    /// The routine had nothing to say this slot: no data, its slot already
    /// claimed, an account no longer active. It moved on to its next slot.
    Skipped,
    /// The wake no longer names the routine's next firing; nothing ran.
    Stale,
    /// A system job ran.
    Ran,
}

/// What one reconcile did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReconcileOutcome {
    /// Active routines read.
    pub routines: usize,
    /// Wakes queued (routines and system jobs).
    pub queued: usize,
    /// Routines or wakes that failed; logged, and retried on the next reconcile.
    pub errors: usize,
}

/// The routine engine.
pub struct RoutineEngine {
    /// Weak handle to the composition root, for the reason
    /// [`crate::services::platform_post::PipelinePlatformPost`] gives.
    ctx: Weak<ServerContext>,
    /// Runs a routine's turn through the chat pipeline. Shared with the
    /// other platform-post senders installed beside the engine.
    post: Arc<dyn PlatformPost>,
    /// Resolves a DM's channel to its adapter and config: the shared
    /// resolver `config_adapter_resolver` hands every DM-delivering sender.
    resolver: Arc<dyn AdapterResolver>,
    /// Queues each next firing. Shared with whoever drains it: Cloud Tasks'
    /// client, or the in-process loop's channel.
    waker: Arc<dyn RoutineWaker>,
}

impl fmt::Debug for RoutineEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RoutineEngine")
            .field("ctx", &self.ctx)
            .finish_non_exhaustive()
    }
}

/// The routine a wake fired, and the moment it fired for.
struct Firing<'routine> {
    routine: &'routine Routine,
    zone: Zone,
    /// The schedule slot.
    slot: Timestamp,
    /// The firing the wake named: the slot, or a deferral of it.
    fire_at: Timestamp,
}

impl RoutineEngine {
    /// An engine over the composition root, posting through `post`,
    /// delivering DM posts through the adapters `resolver` builds, and
    /// queuing next firings on `waker`.
    #[must_use]
    pub fn new(
        ctx: Weak<ServerContext>,
        post: Arc<dyn PlatformPost>,
        resolver: Arc<dyn AdapterResolver>,
        waker: Arc<dyn RoutineWaker>,
    ) -> Self {
        Self {
            ctx,
            post,
            resolver,
            waker,
        }
    }

    /// Run the wake at `now`.
    ///
    /// # Errors
    ///
    /// Returns a database error when the routine or its recipient cannot be
    /// read; the delivery answers an error and Cloud Tasks retries it.
    pub async fn handle(&self, wake: Wake, now: Timestamp) -> AppResult<WakeOutcome> {
        let Some(fire_at) = wake.fire_at() else {
            return Ok(WakeOutcome::Stale);
        };
        match wake {
            Wake::Routine {
                tenant_id,
                routine_id,
                ..
            } => {
                self.fire(TenantId::from_uuid(tenant_id), routine_id, fire_at, now)
                    .await
            }
            Wake::System { job, .. } => Ok(self.run_system(job, fire_at, now).await),
        }
    }

    /// Queue every pending wake again: each active routine's next firing —
    /// scheduled afresh when it has none, or when its slot passed or belongs
    /// to a zone the recipient left — and each system job's next run.
    ///
    /// # Errors
    ///
    /// Returns a database error only when the tenant list cannot be read.
    pub async fn reconcile(&self, now: Timestamp) -> AppResult<ReconcileOutcome> {
        let Some(resources) = self.ctx.upgrade() else {
            return Ok(ReconcileOutcome::default());
        };
        let mut outcome = ReconcileOutcome::default();
        for tenant in resources.common.repos.tenants.get_all().await? {
            self.reconcile_tenant(&resources, tenant.id, now, &mut outcome)
                .await;
        }
        self.queue_system_jobs(now, &mut outcome).await;
        info!(
            routines = outcome.routines,
            queued = outcome.queued,
            errors = outcome.errors,
            "routines reconciled"
        );
        Ok(outcome)
    }

    /// Queue every active routine of `tenant_id` again, counting into `outcome`.
    async fn reconcile_tenant(
        &self,
        resources: &ServerContext,
        tenant_id: TenantId,
        now: Timestamp,
        outcome: &mut ReconcileOutcome,
    ) {
        let routines = match resources
            .common
            .repos
            .routines
            .active_routines(tenant_id)
            .await
        {
            Ok(routines) => routines,
            Err(e) => {
                warn!(error = %e, tenant_id = %tenant_id, "Routines: a tenant's routines are unreadable");
                outcome.errors += 1;
                return;
            }
        };
        for routine in routines {
            outcome.routines += 1;
            match self.reconcile_one(resources, &routine, now).await {
                Ok(true) => outcome.queued += 1,
                Ok(false) => {}
                Err(e) => {
                    warn!(error = %e, routine_id = %routine.id, "Routines: reconciling a routine failed");
                    outcome.errors += 1;
                }
            }
        }
    }

    /// Queue each system job's current or next run, counting into `outcome`.
    async fn queue_system_jobs(&self, now: Timestamp, outcome: &mut ReconcileOutcome) {
        for job in SystemJob::ALL {
            match self
                .waker
                .queue(Wake::system(job, job.current_or_next(now)))
                .await
            {
                Ok(()) => outcome.queued += 1,
                Err(e) => {
                    warn!(error = %e, job = job.as_str(), "Routines: a system job's wake was not queued");
                    outcome.errors += 1;
                }
            }
        }
    }

    /// Schedule the built-in routine of `kind` that `owner` has, at `now`,
    /// once its preference changed. Logged when it fails: the next reconcile
    /// schedules it.
    pub async fn reschedule(
        &self,
        tenant_id: TenantId,
        owner: Uuid,
        kind: RoutineKind,
        now: Timestamp,
    ) {
        let Some(resources) = self.ctx.upgrade() else {
            return;
        };
        match resources
            .common
            .repos
            .routines
            .builtin_routine(tenant_id, owner, kind)
            .await
        {
            Ok(Some(routine)) if routine.schedule().is_some() => {
                if let Err(e) = self.reconcile_one(&resources, &routine, now).await {
                    warn!(error = %e, kind = kind.as_str(), "Routines: rescheduling failed; the next reconcile retries");
                }
            }
            Ok(_) => {}
            Err(e) => {
                warn!(error = %e, kind = kind.as_str(), "Routines: the routine to reschedule is unreadable");
            }
        }
    }

    /// Queue `routine`'s next firing, scheduling it afresh when it has none
    /// still open. `true` when a wake was queued.
    async fn reconcile_one(
        &self,
        resources: &ServerContext,
        routine: &Routine,
        now: Timestamp,
    ) -> AppResult<bool> {
        let Some(schedule) = routine.schedule() else {
            return Ok(false);
        };
        let zone = recipient_zone(resources, routine.owner_user_id).await?;
        let window = routine.kind.window();
        let pending = match (routine.next_slot, routine.next_fire) {
            (Some(slot), Some(fire)) if schedule.holds_slot(&zone, slot) && now < slot + window => {
                Some(fire)
            }
            _ => None,
        };
        let fire = if let Some(fire) = pending {
            fire
        } else {
            let Some(slot) = schedule.current_or_next(&zone, now, window) else {
                return Ok(false);
            };
            let next = NextFiring {
                slot: Some(slot),
                fire: Some(slot),
                deferrals: 0,
            };
            let moved = resources
                .common
                .repos
                .routines
                .schedule_routine(routine.tenant_id, routine.id, routine.next_fire, next, now)
                .await?;
            if !moved {
                // A delivery or a preference change moved it first.
                return Ok(false);
            }
            slot
        };
        self.waker
            .queue(Wake::routine(routine.tenant_id.as_uuid(), routine.id, fire))
            .await?;
        Ok(true)
    }

    /// Fire `routine_id` for the firing at `fire_at`, at `now`.
    async fn fire(
        &self,
        tenant_id: TenantId,
        routine_id: Uuid,
        fire_at: Timestamp,
        now: Timestamp,
    ) -> AppResult<WakeOutcome> {
        let Some(resources) = self.ctx.upgrade() else {
            return Ok(WakeOutcome::Stale);
        };
        let repos = &resources.common.repos;
        let Some(routine) = repos.routines.routine(tenant_id, routine_id).await? else {
            return Ok(WakeOutcome::Stale);
        };
        let (Some(schedule), Some(slot)) = (routine.schedule(), routine.next_slot) else {
            return Ok(WakeOutcome::Stale);
        };
        if routine.next_fire != Some(fire_at) {
            return Ok(WakeOutcome::Stale);
        }
        let Some(user) = repos.users.get_global(routine.owner_user_id).await? else {
            return Ok(WakeOutcome::Stale);
        };
        let zone = resolve_zone(user.timezone.as_deref());
        if !schedule.holds_slot(&zone, slot) {
            // The recipient moved zones since the slot was computed.
            info!(routine_id = %routine.id, "Routine rescheduled: the recipient's zone changed");
            self.reconcile_with_fresh_slot(&resources, &routine, now)
                .await?;
            return Ok(WakeOutcome::Stale);
        }
        let firing = Firing {
            routine: &routine,
            zone,
            slot,
            fire_at,
        };
        if user.user_status != UserStatus::Active {
            self.advance(&resources, &firing, now, false).await?;
            return Ok(WakeOutcome::Skipped);
        }
        if now >= slot + routine.kind.window() {
            return self
                .hold(&resources, &firing, now, HoldReason::WindowPassed)
                .await;
        }
        if let Some(outcome) = self.gate(&resources, &firing, now).await? {
            return Ok(outcome);
        }
        self.act(&resources, &firing, now).await
    }

    /// Run the gates. `Some` when one deferred or held the firing.
    async fn gate(
        &self,
        resources: &ServerContext,
        firing: &Firing<'_>,
        now: Timestamp,
    ) -> AppResult<Option<WakeOutcome>> {
        let routine = firing.routine;
        let (tenant_id, recipient) = (routine.tenant_id, routine.owner_user_id);
        if coach_quiet_hours(resources, recipient, tenant_id, now, &firing.zone).await {
            return self
                .defer(resources, firing, now, HoldReason::QuietHours)
                .await
                .map(Some);
        }
        if resources
            .common
            .repos
            .chat
            .user_wrote_since(tenant_id, recipient, now - CONVERSATION_QUIET)
            .await?
        {
            return self
                .defer(resources, firing, now, HoldReason::MidConversation)
                .await
                .map(Some);
        }
        let midnight = firing
            .zone
            .start_of_day(firing.zone.date_at(now))
            .map_err(|e| AppError::internal(format!("the recipient's local midnight: {e}")))?;
        let landed = resources
            .common
            .repos
            .platform_post_claims
            .platform_posts_landed_since(tenant_id, recipient, midnight)
            .await?;
        if landed >= ATHLETE_DAILY_CAP {
            return self
                .hold(resources, firing, now, HoldReason::DailyCap)
                .await
                .map(Some);
        }
        Ok(None)
    }

    /// Run the routine's action, then move it to its next slot.
    async fn act(
        &self,
        resources: &ServerContext,
        firing: &Firing<'_>,
        now: Timestamp,
    ) -> AppResult<WakeOutcome> {
        let routine = firing.routine;
        let outcome = match routine.kind {
            RoutineKind::CheckIn => {
                let poster = CheckInPoster {
                    post: self.post.as_ref(),
                    resolver: self.resolver.as_ref(),
                };
                let athlete = PostAthlete {
                    user_id: routine.owner_user_id,
                    tenant_id: routine.tenant_id,
                };
                match run_check_in(resources, poster, athlete, &firing.zone, now).await {
                    CheckInRun::Posted(surface) => WakeOutcome::Posted(surface),
                    CheckInRun::Withheld(surface) => WakeOutcome::Withheld(surface),
                    CheckInRun::Unattempted => {
                        return self
                            .defer(resources, firing, now, HoldReason::Unattempted)
                            .await;
                    }
                    CheckInRun::Off | CheckInRun::NoData | CheckInRun::AlreadyClaimed => {
                        WakeOutcome::Skipped
                    }
                }
            }
        };
        let fired = matches!(outcome, WakeOutcome::Posted(_) | WakeOutcome::Withheld(_));
        if let WakeOutcome::Posted(surface) | WakeOutcome::Withheld(surface) = outcome {
            report_fired(routine, surface, matches!(outcome, WakeOutcome::Posted(_)));
        }
        self.advance(resources, firing, now, fired).await?;
        Ok(outcome)
    }

    /// Hold the firing back [`DEFER_STEP`] for `reason`, or drop it as held
    /// when that would leave its window — or, while the recipient is
    /// mid-conversation, when it already waited [`MAX_DEFERRALS`] times.
    async fn defer(
        &self,
        resources: &ServerContext,
        firing: &Firing<'_>,
        now: Timestamp,
        reason: HoldReason,
    ) -> AppResult<WakeOutcome> {
        let routine = firing.routine;
        let counted = reason == HoldReason::MidConversation;
        let deferrals = routine.deferrals + u32::from(counted);
        let until = firing.fire_at.max(now) + DEFER_STEP;
        if (counted && deferrals > MAX_DEFERRALS) || until >= firing.slot + routine.kind.window() {
            return self.hold(resources, firing, now, reason).await;
        }
        let next = NextFiring {
            slot: Some(firing.slot),
            fire: Some(until),
            deferrals,
        };
        if resources
            .common
            .repos
            .routines
            .schedule_routine(
                routine.tenant_id,
                routine.id,
                Some(firing.fire_at),
                next,
                now,
            )
            .await?
        {
            info!(
                routine_id = %routine.id,
                kind = routine.kind.as_str(),
                reason = reason.as_str(),
                deferrals,
                "routine deferred"
            );
            self.queue_logged(Wake::routine(
                routine.tenant_id.as_uuid(),
                routine.id,
                until,
            ))
            .await;
        }
        Ok(WakeOutcome::Deferred(until))
    }

    /// Drop the slot as held for `reason` and move to the next one.
    async fn hold(
        &self,
        resources: &ServerContext,
        firing: &Firing<'_>,
        now: Timestamp,
        reason: HoldReason,
    ) -> AppResult<WakeOutcome> {
        report_held(firing.routine, reason);
        self.advance(resources, firing, now, false).await?;
        Ok(WakeOutcome::Held(reason))
    }

    /// Move the routine to its first slot after this one, recording that it
    /// fired when `fired`, and queue that slot's wake.
    async fn advance(
        &self,
        resources: &ServerContext,
        firing: &Firing<'_>,
        now: Timestamp,
        fired: bool,
    ) -> AppResult<()> {
        let routine = firing.routine;
        let Some(schedule) = routine.schedule() else {
            return Ok(());
        };
        let slot = schedule.next_after(&firing.zone, now.max(firing.slot));
        let next = NextFiring {
            slot,
            fire: slot,
            deferrals: 0,
        };
        let repo = &resources.common.repos.routines;
        let expected = Some(firing.fire_at);
        let moved = if fired {
            repo.record_routine_fired(routine.tenant_id, routine.id, expected, now, next)
                .await?
        } else {
            repo.schedule_routine(routine.tenant_id, routine.id, expected, next, now)
                .await?
        };
        if let (true, Some(slot)) = (moved, slot) {
            self.queue_logged(Wake::routine(routine.tenant_id.as_uuid(), routine.id, slot))
                .await;
        }
        Ok(())
    }

    /// Schedule `routine` from its current slot, ignoring the one it holds.
    async fn reconcile_with_fresh_slot(
        &self,
        resources: &ServerContext,
        routine: &Routine,
        now: Timestamp,
    ) -> AppResult<bool> {
        let unscheduled = Routine {
            next_slot: None,
            ..routine.clone()
        };
        self.reconcile_one(resources, &unscheduled, now).await
    }

    /// Run `job` for its run at `run`, then queue its next run.
    async fn run_system(&self, job: SystemJob, run: Timestamp, now: Timestamp) -> WakeOutcome {
        let Some(resources) = self.ctx.upgrade() else {
            return WakeOutcome::Stale;
        };
        match job {
            SystemJob::PersonaDigest => run_persona_digest(&resources).await,
        }
        let next = job.next_after(run.max(now - SystemJob::WINDOW));
        self.queue_logged(Wake::system(job, next)).await;
        WakeOutcome::Ran
    }

    /// Queue `wake`, logging a refusal: the next reconcile queues it again.
    async fn queue_logged(&self, wake: Wake) {
        if let Err(e) = self.waker.queue(wake).await {
            warn!(error = %e, task = %wake.task_id(), "Routines: the next wake was not queued; the reconcile retries");
        }
    }
}

/// The recipient's own zone, from their user row; UTC when they have none.
async fn recipient_zone(resources: &ServerContext, user_id: Uuid) -> AppResult<Zone> {
    let user = resources.common.repos.users.get_global(user_id).await?;
    Ok(resolve_zone(
        user.as_ref().and_then(|user| user.timezone.as_deref()),
    ))
}

/// Run the persona digest: the notifications an armed persona policy held
/// back, returned on each athlete's cadence. Inert while the policy is in
/// shadow mode — nothing is held, so nothing is returned.
#[cfg(feature = "client-notifications")]
async fn run_persona_digest(resources: &ServerContext) {
    let Some(service) = resources.common.notification_service.as_ref() else {
        debug!("Persona digest: no notification service; nothing to return");
        return;
    };
    if let Err(e) = notification_digest_scheduler::tick(
        &resources.common.repos,
        service,
        &resources.mcp.messaging_strings_registry,
    )
    .await
    {
        warn!(error = %e, "Persona digest: the run failed; the next run returns what this one held");
    }
}

/// Without push notifications compiled in nothing is held, so there is no
/// digest to return.
#[cfg(not(feature = "client-notifications"))]
async fn run_persona_digest(_resources: &ServerContext) {
    debug!("Persona digest: push notifications are not compiled in; nothing to return");
}

/// Report a routine whose turn ran: where, and whether its reply grounded.
fn report_fired(routine: &Routine, surface: PostSurface, grounded: bool) {
    info!(
        target: "notify",
        event = "routine.fired",
        user_id = %routine.owner_user_id,
        tenant_id = %routine.tenant_id,
        kind = routine.kind.as_str(),
        destination = %surface.destination(),
        grounded,
        "routine fired"
    );
}

/// Report a slot that posted nothing, and why.
fn report_held(routine: &Routine, reason: HoldReason) {
    info!(
        target: "notify",
        event = "routine.held",
        user_id = %routine.owner_user_id,
        tenant_id = %routine.tenant_id,
        kind = routine.kind.as_str(),
        reason = reason.as_str(),
        "routine held"
    );
}

/// Schedule `owner`'s check-in afresh now that their cadence changed.
///
/// The cadence write unscheduled the routine, so a wake queued for the old
/// cadence finds nothing to fire. Before the engine is installed this does
/// nothing, and the start-up reconcile schedules it.
pub async fn reschedule_check_in(resources: &ServerContext, tenant_id: TenantId, owner: Uuid) {
    if let Some(engine) = resources.mcp.routines.get() {
        engine
            .reschedule(tenant_id, owner, RoutineKind::CheckIn, Timestamp::now())
            .await;
    }
}

/// Build the engine over the composition root and install it on the
/// context's slot, beside the platform post it fires through.
///
/// Where turns run on Cloud Tasks, wakes are Cloud Tasks tasks on the same
/// queue; elsewhere the process holds them as timers. Either way the engine
/// reconciles once at start, so whatever a lost wake left unqueued is queued
/// again — on scale-to-zero that is every cold start. A second install is a
/// startup logic bug, logged not fatal.
pub fn install_routine_engine(
    resources: &Arc<ServerContext>,
    post: Arc<dyn PlatformPost>,
) -> Arc<RoutineEngine> {
    let resolver = config_adapter_resolver(
        Arc::clone(&resources.common.repos),
        Arc::clone(&resources.mcp.channel_adapters),
    );
    let (waker, in_process) = resources.common.turn_runner.cloud_tasks().map_or_else(
        || {
            let (local, receiver) = InProcessWaker::channel();
            let waker: Arc<dyn RoutineWaker> = Arc::new(local);
            (waker, Some(receiver))
        },
        |runner| {
            let waker: Arc<dyn RoutineWaker> = Arc::new(CloudTasksWaker::over(runner));
            (waker, None)
        },
    );
    let engine = Arc::new(RoutineEngine::new(
        Arc::downgrade(resources),
        post,
        resolver,
        waker,
    ));
    if let Some(receiver) = in_process {
        run_in_process_wakes(Arc::clone(&engine), receiver);
    }
    if resources.mcp.routines.set(Arc::clone(&engine)).is_err() {
        warn!("Routine engine already installed; skipping");
    }
    let reconciling = Arc::clone(&engine);
    tokio::spawn(async move {
        if let Err(e) = reconciling.reconcile(Timestamp::now()).await {
            warn!(error = %e, "Routines: the start-up reconcile failed; the cron retries");
        }
    });
    engine
}

/// Run every wake the in-process waker hands over, each at its time, in
/// this process. Used wherever turns run in-process (locally): no queue, the
/// process is the waker.
pub fn run_in_process_wakes(engine: Arc<RoutineEngine>, mut wakes: mpsc::UnboundedReceiver<Wake>) {
    tokio::spawn(async move {
        let mut pending: HashSet<String> = HashSet::new();
        let (done_tx, mut done_rx) = mpsc::unbounded_channel::<String>();
        loop {
            tokio::select! {
                Some(task) = done_rx.recv() => {
                    pending.remove(&task);
                }
                wake = wakes.recv() => {
                    let Some(wake) = wake else { break };
                    let task = wake.task_id();
                    if !pending.insert(task.clone()) {
                        continue;
                    }
                    let engine = Arc::clone(&engine);
                    let done = done_tx.clone();
                    tokio::spawn(async move {
                        // Joined, so a panicking firing is logged and its task
                        // leaves `pending`: the same wake can be queued again.
                        let fired = tokio::spawn(async move {
                            if let Some(at) = wake.fire_at() {
                                let wait = (at - Timestamp::now()).to_std().unwrap_or_default();
                                time::sleep(wait).await;
                            }
                            engine.handle(wake, Timestamp::now()).await
                        });
                        match fired.await {
                            Ok(Ok(_)) => {}
                            Ok(Err(e)) => {
                                warn!(error = %e, task = %task, "Routines: an in-process wake failed");
                            }
                            Err(e) => {
                                error!(error = %e, task = %task, "Routines: an in-process wake panicked");
                            }
                        }
                        if done.send(task).is_err() {
                            debug!("Routines: the in-process loop stopped before a wake finished");
                        }
                    });
                }
            }
        }
    });
}
