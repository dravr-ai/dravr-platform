// ABOUTME: The coach's check-in on the cadence the athlete chose — every morning, three a week, weekly, or never
// ABOUTME: A periodic tick finds athletes whose local morning is due, claims the day once, and posts through PlatformPost
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The coach checks in (carnet#831).
//!
//! The athlete chooses how often the coach writes to them first — asked
//! during onboarding on every surface, changed in Settings
//! ([`CheckInCadence`]). This worker keeps that promise.
//!
//! ## When
//!
//! Every [`CHECK_IN_TICK_INTERVAL`] the tick reads, tenant by tenant, the
//! athletes whose cadence posts: `off`, and an athlete who never chose, are
//! not read at all. One is due when their own wall clock — the zone on their
//! user row — is inside the morning window of a day their cadence posts on
//! ([`CheckInCadence::due_at`]), and their quiet hours for coach
//! notifications do not cover the moment: the rule that holds a push holds a
//! check-in. An account that is not active is never checked in on, nor is
//! one with no provider connected: a check-in reads the athlete's own data,
//! and with none to read its reply could only be withheld.
//!
//! ## Once a day, across replicas
//!
//! Before any model call the day is claimed in `platform_post_claims` under
//! [`PostPurpose::CheckIn`], retaken only once the last claim predates the
//! athlete's local midnight. A turn that ran keeps its claim even when its
//! reply was withheld, so a check-in costs at most one model call a day; only
//! a check-in nothing could be attempted for — no instruction in the
//! athlete's language, no thread to post into — releases it for the next
//! tick.
//!
//! ## Where
//!
//! One destination per check-in, decided before the turn
//! ([`crate::services::post_delivery`]): the messaging DM the athlete last
//! wrote from, when its channel may be written to unprompted; otherwise their
//! own in-app thread. A `WhatsApp` or Messenger DM whose 24-hour window has
//! closed is held as the commitment reporter holds a verdict there — nothing
//! is sent on it, `messaging.proactive_held` says so, and the check-in lands
//! in the app.

use std::sync::{Arc, Weak};
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Timelike, Utc};
use chrono_tz::Tz;
use pierre_contremaitre::messaging_strings::{
    KEY_PLATFORM_POST_CHECK_IN_DAILY_PROMPT, KEY_PLATFORM_POST_CHECK_IN_THREE_PER_WEEK_PROMPT,
    KEY_PLATFORM_POST_CHECK_IN_WEEKLY_PROMPT,
};
use pierre_core::civil_time::resolve_zone;
use pierre_core::errors::AppResult;
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::{CheckInCadence, ConnectionType, TenantId, UserStatus};
use pierre_database::repositories::CheckInSubscriber;
use pierre_services::periodic::spawn_periodic;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::services::backfill_notifier::{config_adapter_resolver, AdapterResolver};
use crate::services::platform_post::{PlatformPost, PostPurpose, PostSurface};
use crate::services::post_delivery::{linked_dms, post_in_app, post_to_dm, PostAthlete};

/// How often the check-in tick runs.
///
/// Short against the morning window
/// ([`pierre_core::models::check_in::CHECK_IN_WINDOW_HOURS`]), so a check-in
/// lands soon after the hour and a tick held by quiet hours or a missed one
/// is retried inside the same morning.
pub const CHECK_IN_TICK_INTERVAL: StdDuration = StdDuration::from_mins(30);

/// The worker's name in logs and in the worker-run ledger.
const WORKER_NAME: &str = "coach check-in scheduler";

/// Why a check-in on a closed window was not sent, as
/// `messaging.proactive_held` reports it.
const HELD_WINDOW_CLOSED: &str = "window_closed";

/// Outcome of one check-in tick — exposed for tests and metrics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CheckInTickOutcome {
    /// Tenants examined.
    pub tenants_scanned: usize,
    /// Athletes whose cadence posts, across those tenants.
    pub subscribers: usize,
    /// Athletes whose morning window was open on a day their cadence posts.
    pub due: usize,
    /// Due athletes held by their quiet hours.
    pub quiet: usize,
    /// Check-ins whose reply was stored, in the app or on a DM.
    pub posted: usize,
    /// Per-athlete or per-tenant errors. Logged; the sweep continues.
    pub errors: usize,
}

/// The production check-in worker: posts each due athlete's check-in.
pub struct CheckInPoster {
    /// Weak handle to the composition root, for the reason
    /// [`crate::services::platform_post::PipelinePlatformPost`] gives.
    ctx: Weak<ServerContext>,
    /// Runs each check-in through the chat pipeline.
    post: Arc<dyn PlatformPost>,
    /// Resolves a DM's channel to its adapter and config.
    resolver: Arc<dyn AdapterResolver>,
}

impl CheckInPoster {
    /// A worker over the composition root, posting through `post` and
    /// delivering DM check-ins through the adapters `resolver` builds.
    #[must_use]
    pub fn new(
        ctx: Weak<ServerContext>,
        post: Arc<dyn PlatformPost>,
        resolver: Arc<dyn AdapterResolver>,
    ) -> Self {
        Self {
            ctx,
            post,
            resolver,
        }
    }

    /// Run one sweep at `now`: every due athlete of every tenant is checked
    /// in on, at most once a day.
    ///
    /// Exposed, rather than only the loop [`start_check_in_scheduler`] runs,
    /// so integration tests drive one sweep at the instant they choose.
    ///
    /// # Errors
    ///
    /// Returns the database error only when the tenant list cannot be read.
    /// Per-tenant and per-athlete failures are counted in
    /// [`CheckInTickOutcome::errors`] and do not stop the sweep.
    pub async fn tick(&self, now: DateTime<Utc>) -> AppResult<CheckInTickOutcome> {
        let Some(resources) = self.ctx.upgrade() else {
            return Ok(CheckInTickOutcome::default());
        };
        let tenants = resources.common.repos.tenants.get_all().await?;
        let mut outcome = CheckInTickOutcome {
            tenants_scanned: tenants.len(),
            ..CheckInTickOutcome::default()
        };
        for tenant in tenants {
            self.sweep_tenant(&resources, tenant.id, now, &mut outcome)
                .await;
        }
        log_sweep(&outcome);
        Ok(outcome)
    }

    /// Check in on every athlete of `tenant_id` whose cadence posts.
    async fn sweep_tenant(
        &self,
        resources: &ServerContext,
        tenant_id: TenantId,
        now: DateTime<Utc>,
        outcome: &mut CheckInTickOutcome,
    ) {
        let subscribers = match resources
            .common
            .repos
            .check_in_preferences
            .check_in_subscribers(tenant_id)
            .await
        {
            Ok(subscribers) => subscribers,
            Err(e) => {
                warn!(error = %e, tenant_id = %tenant_id, "Check-in: subscribers unreadable");
                outcome.errors += 1;
                return;
            }
        };
        for subscriber in subscribers {
            outcome.subscribers += 1;
            self.check_in(resources, tenant_id, subscriber, now, outcome)
                .await;
        }
    }

    /// Check in on one athlete when their morning is due.
    async fn check_in(
        &self,
        resources: &ServerContext,
        tenant_id: TenantId,
        subscriber: CheckInSubscriber,
        now: DateTime<Utc>,
        outcome: &mut CheckInTickOutcome,
    ) {
        let CheckInSubscriber { user_id, cadence } = subscriber;
        let Some(zone) = due_zone(resources, subscriber, now, outcome).await else {
            return;
        };
        outcome.due += 1;
        if !reads_a_provider(resources, user_id, tenant_id).await {
            debug!(
                cadence = cadence.as_str(),
                "Check-in skipped: no provider connected to read"
            );
            return;
        }
        if coach_quiet_hours(resources, user_id, tenant_id, now, zone).await {
            info!(
                cadence = cadence.as_str(),
                "Check-in held: the athlete's quiet hours"
            );
            outcome.quiet += 1;
            return;
        }
        // The athlete's local midnight: a claim taken before it was an
        // earlier day's.
        let since_midnight = now.with_timezone(&zone).time().num_seconds_from_midnight();
        let since = now - Duration::seconds(i64::from(since_midnight));
        if !claim_check_in(resources, user_id, tenant_id, since, now).await {
            return;
        }
        let athlete = PostAthlete { user_id, tenant_id };
        match self.post_check_in(resources, athlete, cadence, now).await {
            Some((surface, posted)) => {
                report_check_in(user_id, tenant_id, cadence, surface, posted);
                if posted {
                    outcome.posted += 1;
                }
            }
            None => release_check_in(resources, user_id, tenant_id).await,
        }
    }

    /// Post a claimed check-in to its one destination: the DM the athlete
    /// last wrote from while its window is open, else their in-app thread.
    ///
    /// Answers the destination and whether the reply was stored, or `None`
    /// when nothing could be attempted anywhere.
    async fn post_check_in(
        &self,
        resources: &ServerContext,
        athlete: PostAthlete,
        cadence: CheckInCadence,
        now: DateTime<Utc>,
    ) -> Option<(PostSurface, bool)> {
        let prompt = |locale: &str| check_in_prompt(resources, cadence, locale);
        let dms = linked_dms(resources, athlete, now).await;
        if let Some(dm) = dms.open {
            let surface = PostSurface::Channel(dm.channel_type);
            let on_dm = post_to_dm(
                resources,
                self.post.as_ref(),
                self.resolver.as_ref(),
                athlete,
                dm,
                &prompt,
            )
            .await;
            if let Some(posted) = on_dm {
                return Some((surface, posted));
            }
            // No instruction in the DM's language: the app's thread, in the
            // athlete's own, is still a destination.
        } else {
            for channel in &dms.held {
                report_held(athlete.tenant_id, *channel);
            }
        }
        post_in_app(resources, self.post.as_ref(), athlete, &prompt)
            .await
            .map(|posted| (PostSurface::InApp, posted))
    }
}

/// The athlete's own zone when their check-in is due at `now`: an active
/// account, on a day their cadence posts on, inside the morning window.
/// `None` otherwise, and when the account is unreadable (counted).
async fn due_zone(
    resources: &ServerContext,
    subscriber: CheckInSubscriber,
    now: DateTime<Utc>,
    outcome: &mut CheckInTickOutcome,
) -> Option<Tz> {
    let user = match resources
        .common
        .repos
        .users
        .get_global(subscriber.user_id)
        .await
    {
        Ok(user) => user?,
        Err(e) => {
            warn!(error = %e, "Check-in: the athlete's account is unreadable");
            outcome.errors += 1;
            return None;
        }
    };
    if user.user_status != UserStatus::Active {
        return None;
    }
    let zone = resolve_zone(user.timezone.as_deref());
    subscriber
        .cadence
        .due_at(now.with_timezone(&zone).naive_local())
        .then_some(zone)
}

/// Whether the athlete has a provider connected in `tenant_id` — a real
/// connection, not a synthetic one — for the check-in to read. An unreadable
/// list reads as none (logged): no model call is spent on a post that cannot
/// ground.
async fn reads_a_provider(resources: &ServerContext, user_id: Uuid, tenant_id: TenantId) -> bool {
    match resources
        .common
        .repos
        .provider_connections
        .get_for_user(user_id, Some(tenant_id))
        .await
    {
        Ok(connections) => connections
            .iter()
            .any(|connection| connection.connection_type != ConnectionType::Synthetic),
        Err(e) => {
            warn!(error = %e, "Check-in: provider connections unreadable");
            false
        }
    }
}

/// Log one sweep: at info when anyone was due, else at debug.
fn log_sweep(outcome: &CheckInTickOutcome) {
    if outcome.due > 0 {
        info!(
            tenants_scanned = outcome.tenants_scanned,
            subscribers = outcome.subscribers,
            due = outcome.due,
            quiet = outcome.quiet,
            posted = outcome.posted,
            errors = outcome.errors,
            "check-in sweep complete"
        );
    } else {
        debug!(
            subscribers = outcome.subscribers,
            "check-in sweep: nobody's morning is due"
        );
    }
}

/// Claim today's check-in. `false` when a replica or an earlier tick already
/// holds it, or the claim failed (logged).
async fn claim_check_in(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: TenantId,
    since: DateTime<Utc>,
    now: DateTime<Utc>,
) -> bool {
    match resources
        .common
        .repos
        .platform_post_claims
        .claim_platform_post_since(
            tenant_id,
            user_id,
            PostPurpose::CheckIn.as_str(),
            since,
            now,
        )
        .await
    {
        Ok(claimed) => claimed,
        Err(e) => {
            warn!(error = %e, "Check-in: the day's claim failed");
            false
        }
    }
}

/// Release today's claim of a check-in nothing could be attempted for, so the
/// next tick of the same morning tries again.
async fn release_check_in(resources: &ServerContext, user_id: Uuid, tenant_id: TenantId) {
    if let Err(e) = resources
        .common
        .repos
        .platform_post_claims
        .release_platform_post(tenant_id, user_id, PostPurpose::CheckIn.as_str())
        .await
    {
        warn!(error = %e, "Check-in: releasing the claim of an unmade check-in failed");
    }
}

/// The check-in instruction `cadence` gives in `locale`, or `None` (logged)
/// when the catalogue does not carry it: an empty instruction would have the
/// coach say nothing in particular.
fn check_in_prompt(
    resources: &ServerContext,
    cadence: CheckInCadence,
    locale: &str,
) -> Option<String> {
    let key = match cadence {
        CheckInCadence::Daily => KEY_PLATFORM_POST_CHECK_IN_DAILY_PROMPT,
        CheckInCadence::ThreePerWeek => KEY_PLATFORM_POST_CHECK_IN_THREE_PER_WEEK_PROMPT,
        CheckInCadence::Weekly => KEY_PLATFORM_POST_CHECK_IN_WEEKLY_PROMPT,
        CheckInCadence::Off => return None,
    };
    let prompt = resources.mcp.messaging_strings_registry.get(key, locale);
    if prompt.trim().is_empty() {
        warn!(
            locale,
            cadence = cadence.as_str(),
            "Check-in: the catalogue carries no instruction; nothing posted"
        );
        return None;
    }
    Some(prompt)
}

/// Whether the athlete's quiet hours for coach notifications cover `now`.
///
/// Read on the clock the preference names, else the athlete's own zone. An
/// athlete with no such preference, or an unreadable one, has none.
#[cfg(feature = "client-notifications")]
async fn coach_quiet_hours(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: TenantId,
    now: DateTime<Utc>,
    zone: Tz,
) -> bool {
    use pierre_core::models::check_in::within_quiet_hours;
    use pierre_notifications::models::NotificationCategory;
    use pierre_notifications::TenantId as CommTenantId;

    let Some(service) = resources.common.notification_service.as_ref() else {
        return false;
    };
    let preferences = match service
        .get_notification_preferences(user_id, CommTenantId(tenant_id.as_uuid()))
        .await
    {
        Ok(preferences) => preferences,
        Err(e) => {
            warn!(error = %e, "Check-in: notification preferences unreadable; no quiet hours applied");
            return false;
        }
    };
    let Some(coach) = preferences
        .iter()
        .find(|preference| preference.category == NotificationCategory::Coach)
    else {
        return false;
    };
    let (Some(start), Some(end)) = (coach.quiet_hours_start, coach.quiet_hours_end) else {
        return false;
    };
    let clock = coach
        .timezone
        .as_deref()
        .and_then(|name| name.parse::<Tz>().ok())
        .unwrap_or(zone);
    within_quiet_hours(start, end, now.with_timezone(&clock).time())
}

/// Notification preferences are compiled out of this build, so no quiet
/// hours are on record to hold a check-in.
#[cfg(not(feature = "client-notifications"))]
async fn coach_quiet_hours(
    _resources: &ServerContext,
    _user_id: Uuid,
    _tenant_id: TenantId,
    _now: DateTime<Utc>,
    _zone: Tz,
) -> bool {
    false
}

/// Report one check-in: where it was posted, and whether the coach's reply
/// grounded in the athlete's data and was stored.
fn report_check_in(
    user_id: Uuid,
    tenant_id: TenantId,
    cadence: CheckInCadence,
    surface: PostSurface,
    grounded: bool,
) {
    info!(
        target: "notify",
        event = "coach.check_in",
        user_id = %user_id,
        tenant_id = %tenant_id,
        cadence = cadence.as_str(),
        destination = %surface.destination(),
        grounded,
        "coach check-in"
    );
}

/// Report a DM whose window had closed, so the check-in was not sent there.
fn report_held(tenant_id: TenantId, channel: ChannelType) {
    info!(
        target: "notify",
        event = "messaging.proactive_held",
        tenant_id = %tenant_id,
        channel = %channel,
        purpose = PostPurpose::CheckIn.as_str(),
        reason = HELD_WINDOW_CLOSED,
        "proactive check-in held"
    );
}

/// Start the check-in worker: a [`CheckInPoster`] over the composition root,
/// ticking every [`CHECK_IN_TICK_INTERVAL`] on the worker-run ledger, so
/// across instances a tick runs once.
pub fn start_check_in_scheduler(resources: &Arc<ServerContext>, post: Arc<dyn PlatformPost>) {
    let poster = Arc::new(CheckInPoster::new(
        Arc::downgrade(resources),
        post,
        config_adapter_resolver(Arc::clone(&resources.common.repos)),
    ));
    spawn_periodic(
        WORKER_NAME,
        CHECK_IN_TICK_INTERVAL,
        Arc::clone(&resources.common.repos.worker_runs),
        move || {
            let poster = Arc::clone(&poster);
            async move {
                poster.tick(Utc::now()).await?;
                Ok(())
            }
        },
    );
}
