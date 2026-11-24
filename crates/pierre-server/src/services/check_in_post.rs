// ABOUTME: The coach's check-in on the cadence the athlete chose — every morning, three a week, weekly, or never
// ABOUTME: Run by the routine engine when the athlete's check-in routine fires: claims the day once, posts through PlatformPost
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The coach checks in (carnet#831).
//!
//! The athlete chooses how often the coach writes to them first — asked
//! during onboarding on every surface, changed in Settings
//! ([`CheckInCadence`]). Their choice is also their check-in *routine*
//! (ADR-036): the cadence's days at 08:00 of their own clock, paused for
//! `off`.
//!
//! ## When
//!
//! The routine engine ([`crate::services::routine_engine`]) decides: it wakes
//! the server at the routine's slot, holds the check-in while the athlete's
//! quiet hours cover the moment or they are mid-conversation, keeps it under
//! the daily cap, and runs [`run_check_in`] once all of that passes. A
//! check-in still not out
//! [`CHECK_IN_WINDOW_HOURS`](pierre_core::models::check_in::CHECK_IN_WINDOW_HOURS)
//! hours after its slot is skipped, never sent late.
//!
//! An account with no data yet — no activity and no sleep or recovery source
//! (carnet#1099) — is not checked in on: a check-in reads the athlete's own
//! data, and with none to read its reply could only be withheld. A plan is
//! not needed: without one the check-in advises on the day.
//!
//! ## Once a day, across replicas
//!
//! Before any model call the day is claimed in `platform_post_claims` under
//! [`PostPurpose::CheckIn`], retaken only once the last claim predates the
//! athlete's local midnight — so a wake Cloud Tasks delivers twice still
//! posts once. A turn that ran keeps its claim even when its reply was
//! withheld, so a check-in costs at most one model call a day; only a
//! check-in nothing could be attempted for — no instruction in the athlete's
//! language, no thread to post into — releases it, and the engine tries again
//! inside the window. A check-in whose reply was stored is marked posted,
//! which is how the athlete's reply that day is known to answer it
//! (carnet#1099).
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
//!
//! ## What the turn may do
//!
//! Suggest, never act ([`PostAccess::SuggestOnly`]): the athlete did not type
//! this turn, so it reads their data and writes nothing — no plan saved, no
//! calendar pushed, nobody else messaged.

use dravr_tronc::dates::{Timestamp, Zone};
use pierre_contremaitre::messaging_strings::{
    KEY_PLATFORM_POST_CHECK_IN_DAILY_PROMPT, KEY_PLATFORM_POST_CHECK_IN_THREE_PER_WEEK_PROMPT,
    KEY_PLATFORM_POST_CHECK_IN_WEEKLY_PROMPT,
};
use pierre_contremaitre::use_case_catalogue::Predicate;
use pierre_core::models::{CheckInCadence, PostPurpose, TenantId};
use pierre_services::athlete_state::AthleteState;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::services::adapter_resolver::AdapterResolver;
use crate::services::platform_post::{PlatformPost, PostAccess, PostSurface};
use crate::services::post_delivery::{linked_dms, post_to_one, PostAthlete, PostIntent};

/// What one check-in did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CheckInRun {
    /// The reply grounded in the athlete's data and was stored there.
    Posted(PostSurface),
    /// The turn ran on that surface and its reply was withheld.
    Withheld(PostSurface),
    /// The athlete's cadence is `off`, or they never chose one.
    Off,
    /// The athlete has no data for a check-in to read yet.
    NoData,
    /// Today's check-in was already claimed — posted, withheld, or running.
    AlreadyClaimed,
    /// Nothing could be attempted anywhere; the day's claim was released.
    Unattempted,
}

/// The services a check-in posts through.
#[derive(Clone, Copy)]
pub(crate) struct CheckInPoster<'svc> {
    /// Runs the check-in through the chat pipeline.
    pub post: &'svc dyn PlatformPost,
    /// Resolves a DM's channel to its adapter and config.
    pub resolver: &'svc dyn AdapterResolver,
}

/// Check in on `athlete` at `now`, whose clock is `zone`: claim the day,
/// post, and mark the day posted.
pub(crate) async fn run_check_in(
    resources: &ServerContext,
    poster: CheckInPoster<'_>,
    athlete: PostAthlete,
    zone: &Zone,
    now: Timestamp,
) -> CheckInRun {
    let cadence = match chosen_cadence(resources, athlete).await {
        Ok(Some(cadence)) => cadence,
        Ok(None) => return CheckInRun::Off,
        Err(run) => return run,
    };
    if !has_data(resources, athlete.user_id, athlete.tenant_id, now).await {
        debug!(
            cadence = cadence.as_str(),
            "Check-in skipped: no activity or recovery data to read yet"
        );
        return CheckInRun::NoData;
    }
    // The athlete's local midnight: a claim taken before it was an earlier day's.
    let Ok(since) = zone.start_of_day(zone.date_at(now)) else {
        warn!("Check-in: the athlete's local midnight is out of range");
        return CheckInRun::Unattempted;
    };
    let Some(lease) =
        claim_check_in(resources, athlete.user_id, athlete.tenant_id, since, now).await
    else {
        return CheckInRun::AlreadyClaimed;
    };
    post_claimed(resources, poster, athlete, cadence, &lease, now).await
}

/// The cadence the athlete chose, `None` for `off` or no choice;
/// [`CheckInRun::Unattempted`] when it is unreadable (logged).
async fn chosen_cadence(
    resources: &ServerContext,
    athlete: PostAthlete,
) -> Result<Option<CheckInCadence>, CheckInRun> {
    resources
        .common
        .repos
        .check_in_preferences
        .check_in_cadence(athlete.tenant_id, athlete.user_id)
        .await
        .map(|cadence| cadence.filter(|chosen| *chosen != CheckInCadence::Off))
        .map_err(|e| {
            warn!(error = %e, "Check-in: the athlete's cadence is unreadable");
            CheckInRun::Unattempted
        })
}

/// Post the day's check-in under the claim `lease` names: mark the day posted
/// when the reply was stored, release it when nothing could be attempted.
async fn post_claimed(
    resources: &ServerContext,
    poster: CheckInPoster<'_>,
    athlete: PostAthlete,
    cadence: CheckInCadence,
    lease: &str,
    now: Timestamp,
) -> CheckInRun {
    let PostAthlete { user_id, tenant_id } = athlete;
    let prompt = |locale: &str| check_in_prompt(resources, cadence, locale);
    let dms = linked_dms(resources, athlete, now).await;
    let intent = PostIntent {
        purpose: PostPurpose::CheckIn,
        access: PostAccess::SuggestOnly,
    };
    let Some((surface, stored)) = post_to_one(
        resources,
        poster.post,
        poster.resolver,
        athlete,
        dms,
        &prompt,
        intent,
    )
    .await
    else {
        release_check_in(resources, user_id, tenant_id, lease).await;
        return CheckInRun::Unattempted;
    };
    report_check_in(user_id, tenant_id, cadence, surface, stored);
    if stored {
        mark_check_in_posted(resources, user_id, tenant_id, lease, now).await;
        CheckInRun::Posted(surface)
    } else {
        CheckInRun::Withheld(surface)
    }
}

/// Whether the athlete has data for the check-in to read: activities (or a
/// connected provider nobody has read yet), or a sleep or recovery source (a
/// stored night or recovery day, however it arrived) — the catalogue's
/// `has_activities` and `has_recovery_source`, read as the starters read
/// them. An unreadable state reads as none (logged): no model call is spent
/// on a post that cannot ground.
async fn has_data(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: TenantId,
    now: Timestamp,
) -> bool {
    match AthleteState::read(&resources.common.repos, tenant_id, user_id, now).await {
        Ok((state, _)) => {
            state.holds(Predicate::HasActivities) || state.holds(Predicate::HasRecoverySource)
        }
        Err(e) => {
            warn!(error = %e, "Check-in: the athlete's data is unreadable");
            false
        }
    }
}

/// Mark today's check-in posted under the claim `lease` names, so a reply the
/// athlete sends later that day is read as answering it. Logged when it fails
/// or the claim no longer holds the row: the check-in itself landed.
async fn mark_check_in_posted(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: TenantId,
    lease: &str,
    now: Timestamp,
) {
    match resources
        .common
        .repos
        .platform_post_claims
        .mark_platform_post_posted(
            tenant_id,
            user_id,
            PostPurpose::CheckIn.as_str(),
            PostPurpose::NO_SUBJECT,
            lease,
            now,
        )
        .await
    {
        Ok(true) => {}
        Ok(false) => warn!("Check-in: the claim lapsed before the check-in was marked posted"),
        Err(e) => warn!(error = %e, "Check-in: marking the check-in posted failed"),
    }
}

/// Claim today's check-in. `None` when a replica or an earlier delivery
/// already holds it, or the claim failed (logged).
async fn claim_check_in(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: TenantId,
    since: Timestamp,
    now: Timestamp,
) -> Option<String> {
    match resources
        .common
        .repos
        .platform_post_claims
        .claim_platform_post_since(
            tenant_id,
            user_id,
            PostPurpose::CheckIn.as_str(),
            PostPurpose::NO_SUBJECT,
            since,
            now,
        )
        .await
    {
        Ok(claimed) => claimed,
        Err(e) => {
            warn!(error = %e, "Check-in: the day's claim failed");
            None
        }
    }
}

/// Release today's claim of a check-in nothing could be attempted for, so the
/// engine's next try inside the window can take it.
async fn release_check_in(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: TenantId,
    lease: &str,
) {
    if let Err(e) = resources
        .common
        .repos
        .platform_post_claims
        .release_platform_post(
            tenant_id,
            user_id,
            PostPurpose::CheckIn.as_str(),
            PostPurpose::NO_SUBJECT,
            lease,
        )
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

/// Report one check-in: where it was posted, and whether the agent's reply
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
