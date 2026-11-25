// ABOUTME: The coach's reaction to a workout a live sync just landed, posted once a day or once per ride
// ABOUTME: Gates the landing, resolves activity_landed.* for the athlete, claims it, and posts through PlatformPost

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The coach reacts when a workout syncs (carnet#1486).
//!
//! A live write that lands a workout newer than everything the athlete held,
//! and no copy of one, reaches [`ActivityLandedPoster::react`] through the
//! server's [`SessionLandedRouter`](crate::services::session_landed::SessionLandedRouter).
//! The post is one ordinary verified pipeline turn ([`PlatformPost`]), so it
//! is grounded or withheld like the check-in and spends the athlete's budget.
//!
//! ## Gates, cheap first, no model call before the claim
//!
//! 0. Not the athlete's first data (that is the first-sync post's), and the
//!    workout started within [`LANDED_FRESH_FOR`].
//! 1. The account is active.
//! 2. The athlete has a coach thread: their personal thread, or a linked
//!    messaging DM, open or held. Without one the landing goes to the digest.
//! 3. Their check-in cadence is not `off` (never chosen posts).
//! 4. They did not write to the coach within [`ACTIVE_CONVERSATION`] in the
//!    thread or the DM the post could land in.
//! 5. Their quiet hours for coach notifications do not cover the moment.
//! 6. `activity_landed.frequency` and `.digest` are read for the athlete,
//!    user → tenant → system-wide → default, once per landing.
//! 7. The claim: under `per_day` the day's one row (the check-in's day
//!    claim), under `per_ride` one row per `<provider>:<activity_id>`.
//!
//! A skipped gate claims nothing, and since a later copy of the same workout
//! never lands again, a skipped landing never posts later.
//!
//! ## Where
//!
//! As the check-in decides it ([`post_to_one`]): the DM the athlete last
//! wrote from while its window is open, otherwise the app's thread.

use std::fmt;
use std::sync::{Arc, Weak};

use dravr_tronc::dates::{Elapsed, Timestamp, Zone};
use pierre_config::activity_landed::{LandingDigest, LandingFrequency, DIGEST_KEY, FREQUENCY_KEY};
use pierre_contremaitre::messaging_strings::KEY_PLATFORM_POST_ACTIVITY_LANDED_PROMPT;
use pierre_core::civil_time::resolve_zone;
use pierre_core::civil_time::LOCAL_MINUTE;
use pierre_core::models::{CheckInCadence, PostPurpose, TenantId, UserStatus};
use pierre_core::transport::TransportPolicy;
use pierre_runtime_context::{AdminConfigLookup, ConfigLookupScope};
use pierre_services::live_activities::LandedSession;
use pierre_services::personal_thread::find_personal_thread;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::services::adapter_resolver::{config_adapter_resolver, AdapterResolver};
use crate::services::platform_post::{PlatformPost, PostAccess, PostSurface};
use crate::services::post_delivery::{
    coach_quiet_hours, linked_dms, post_to_one, PostAthlete, PostIntent, POST_CLAIM_LEASE,
};

/// How old a workout may be when it lands and still be reacted to: past it,
/// "your ride from last Tuesday" would arrive after a reconnect (OQ4).
pub const LANDED_FRESH_FOR: Elapsed = Elapsed::hours(48);

/// How recently the athlete wrote to the coach for the landing to count as
/// mid-conversation: the live reply reads the same store, so no post (OQ5).
pub const ACTIVE_CONVERSATION: Elapsed = Elapsed::minutes(30);

/// The newest messages read per conversation for the mid-conversation gate.
const RECENT_MESSAGES: i64 = 10;

/// What one landing came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Landed {
    /// The reaction was stored on `PostSurface`; the digest mode resolved for
    /// the athlete says whether the per-session digest still goes.
    Posted(PostSurface, LandingDigest),
    /// The turn ran and its reply was withheld (ungrounded or a refusal). The
    /// claim is kept: the model call is spent and the workout is not retried.
    Withheld,
    /// A gate held the landing, or nothing could be attempted.
    Skipped(SkipReason),
    /// The athlete has no coach thread: the landing is the digest's.
    Digest,
}

impl Landed {
    /// Whether the per-session digest still goes after this landing: always,
    /// unless a stored post replaced it.
    #[must_use]
    pub const fn sends_digest(self) -> bool {
        !matches!(self, Self::Posted(_, LandingDigest::ReplaceDigest))
    }
}

/// Why a landing posted nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The server is shutting down.
    ShuttingDown,
    /// The store held nothing before: the first-sync post's moment.
    FirstData,
    /// The workout started more than [`LANDED_FRESH_FOR`] before it landed.
    Stale,
    /// The account is not active, or unreadable.
    Inactive,
    /// The athlete's check-in cadence is `off`.
    CadenceOff,
    /// The athlete wrote to the coach within [`ACTIVE_CONVERSATION`].
    MidConversation,
    /// The athlete's quiet hours cover the moment.
    QuietHours,
    /// Today's post, or this ride's, is already claimed.
    Claimed,
    /// No instruction in the athlete's language, or no thread to post in.
    NothingAttempted,
}

impl SkipReason {
    /// The reason as the log names it.
    const fn as_str(self) -> &'static str {
        match self {
            Self::ShuttingDown => "shutting_down",
            Self::FirstData => "first_data",
            Self::Stale => "stale",
            Self::Inactive => "inactive",
            Self::CadenceOff => "cadence_off",
            Self::MidConversation => "mid_conversation",
            Self::QuietHours => "quiet_hours",
            Self::Claimed => "claimed",
            Self::NothingAttempted => "nothing_attempted",
        }
    }
}

/// `activity_landed.*` as resolved for one athlete at one landing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LandingConfig {
    /// One post per local day, or one per landing.
    pub frequency: LandingFrequency,
    /// Whether a stored post replaces the per-session digest.
    pub digest: LandingDigest,
}

impl LandingConfig {
    /// Both keys read for `athlete`, user → tenant → system-wide → default.
    /// No admin config, or a failed read (logged), takes the defaults.
    async fn resolve(config: Option<&dyn AdminConfigLookup>, athlete: PostAthlete) -> Self {
        let Some(config) = config else {
            return Self::default();
        };
        let user = athlete.user_id.to_string();
        let tenant = athlete.tenant_id.to_string();
        let scope = ConfigLookupScope::user(&user, &tenant);
        let frequency = read_key(config, FREQUENCY_KEY, scope).await;
        let digest = read_key(config, DIGEST_KEY, scope).await;
        Self {
            frequency: LandingFrequency::from_value(frequency.as_ref()),
            digest: LandingDigest::from_value(digest.as_ref()),
        }
    }
}

/// `key`'s effective value at `scope`; `None` (logged) when the read failed.
async fn read_key(
    config: &dyn AdminConfigLookup,
    key: &str,
    scope: ConfigLookupScope<'_>,
) -> Option<serde_json::Value> {
    match config.get_value(key, scope).await {
        Ok(value) => value,
        Err(e) => {
            warn!(key, error = %e, "Landing post: config unreadable; using the default");
            None
        }
    }
}

/// A claim this landing took: the subject it is under and its lease token.
struct Claim {
    subject: String,
    lease: String,
}

/// The production landing poster.
pub struct ActivityLandedPoster {
    /// Weak handle to the composition root, for the reason
    /// [`crate::services::platform_post::PipelinePlatformPost`] gives.
    ctx: Weak<ServerContext>,
    /// Runs each post through the chat pipeline.
    post: Arc<dyn PlatformPost>,
    /// Resolves a DM's channel to its adapter and config.
    resolver: Arc<dyn AdapterResolver>,
}

impl fmt::Debug for ActivityLandedPoster {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ActivityLandedPoster")
            .field("ctx", &self.ctx)
            .finish_non_exhaustive()
    }
}

impl ActivityLandedPoster {
    /// A poster over the composition root, posting through `post` and
    /// delivering DM posts through the adapters `resolver` builds.
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

    /// React to `session` on the server's in-flight turn tracker, so a
    /// shutdown drain awaits the post and a panic in it is logged.
    pub(crate) async fn react(
        self: &Arc<Self>,
        athlete: PostAthlete,
        session: LandedSession,
    ) -> Landed {
        let Some(resources) = self.ctx.upgrade() else {
            return Landed::Skipped(SkipReason::ShuttingDown);
        };
        let tracker = resources.common.turns.tracker();
        drop(resources);
        let poster = Arc::clone(self);
        let handle = tracker.spawn(async move {
            poster
                .land(
                    athlete.user_id,
                    athlete.tenant_id,
                    session,
                    Timestamp::now(),
                )
                .await
        });
        match handle.await {
            Ok(landed) => landed,
            Err(e) => {
                error!(error = %e, "Landing post: the tracked post did not finish");
                Landed::Skipped(SkipReason::NothingAttempted)
            }
        }
    }

    /// Run the gates and, past them, the post for `session` at `now`.
    ///
    /// Exposed, beside [`Self::react`], so integration tests drive one
    /// landing at the instant they choose.
    pub async fn land(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        session: LandedSession,
        now: Timestamp,
    ) -> Landed {
        let athlete = PostAthlete { user_id, tenant_id };
        let landed = self.gate_and_post(athlete, &session, now).await;
        match landed {
            Landed::Skipped(reason) => info!(
                user_id = %athlete.user_id,
                provider = %session.provider,
                activity_id = %session.activity_id,
                reason = reason.as_str(),
                "Landing post skipped"
            ),
            Landed::Digest => info!(
                user_id = %athlete.user_id,
                "Landing post: no coach thread; the landing goes to the digest"
            ),
            Landed::Posted(..) | Landed::Withheld => {}
        }
        landed
    }

    async fn gate_and_post(
        &self,
        athlete: PostAthlete,
        session: &LandedSession,
        now: Timestamp,
    ) -> Landed {
        let Some(resources) = self.ctx.upgrade() else {
            return Landed::Skipped(SkipReason::ShuttingDown);
        };
        if session.first_in_store {
            return Landed::Skipped(SkipReason::FirstData);
        }
        if now - session.start > LANDED_FRESH_FOR {
            return Landed::Skipped(SkipReason::Stale);
        }
        let Some(zone) = active_zone(&resources, athlete).await else {
            return Landed::Skipped(SkipReason::Inactive);
        };
        let dms = linked_dms(&resources, athlete, now).await;
        let thread = match find_personal_thread(
            &resources.common.repos,
            &athlete.user_id.to_string(),
            athlete.tenant_id,
        )
        .await
        {
            Ok(thread) => thread.map(|thread| thread.id),
            Err(e) => {
                warn!(error = %e, "Landing post: the athlete's personal thread is unreadable");
                None
            }
        };
        if thread.is_none() && dms.open.is_none() && dms.held.is_empty() {
            return Landed::Digest;
        }
        if cadence_is_off(&resources, athlete).await {
            return Landed::Skipped(SkipReason::CadenceOff);
        }
        let mut conversations: Vec<&str> = thread.iter().map(String::as_str).collect();
        if let Some(dm) = &dms.open {
            conversations.push(dm.conversation_id());
        }
        if mid_conversation(&resources, athlete, &conversations, now).await {
            return Landed::Skipped(SkipReason::MidConversation);
        }
        if coach_quiet_hours(&resources, athlete.user_id, athlete.tenant_id, now, &zone).await {
            return Landed::Skipped(SkipReason::QuietHours);
        }
        let config = LandingConfig::resolve(
            resources
                .agent
                .admin_config
                .as_deref()
                .map(|config| -> &dyn AdminConfigLookup { config }),
            athlete,
        )
        .await;
        let Some(claim) = claim(&resources, athlete, session, config.frequency, &zone, now).await
        else {
            return Landed::Skipped(SkipReason::Claimed);
        };
        let start = zone.format_with(session.start, &LOCAL_MINUTE);
        let prompt = |locale: &str| landing_prompt(&resources, locale, &start, session);
        let posted = post_to_one(
            &resources,
            self.post.as_ref(),
            self.resolver.as_ref(),
            athlete,
            dms,
            &prompt,
            PostIntent {
                purpose: PostPurpose::ActivityLanded,
                // The athlete's own workout landing; moves onto the routine
                // engine, and its read-only grant, with carnet#1795.
                access: PostAccess::Owner,
                findings: None,
                push: false,
            },
        )
        .await;
        match posted {
            Some((surface, true)) => {
                report_landed(athlete, surface, true);
                mark_posted(&resources, athlete, &claim, now).await;
                Landed::Posted(surface, config.digest)
            }
            Some((surface, false)) => {
                report_landed(athlete, surface, false);
                Landed::Withheld
            }
            None => {
                release(&resources, athlete, &claim).await;
                Landed::Skipped(SkipReason::NothingAttempted)
            }
        }
    }
}

/// The athlete's own zone when their account is active; `None` otherwise,
/// and when it is unreadable (logged).
async fn active_zone(resources: &ServerContext, athlete: PostAthlete) -> Option<Zone> {
    match resources
        .common
        .repos
        .users
        .get_global(athlete.user_id)
        .await
    {
        Ok(Some(user)) if user.user_status == UserStatus::Active => {
            Some(resolve_zone(user.timezone.as_deref()))
        }
        Ok(_) => None,
        Err(e) => {
            warn!(error = %e, "Landing post: the athlete's account is unreadable");
            None
        }
    }
}

/// Whether the athlete chose no check-ins. Never chosen posts (OQ2); an
/// unreadable choice holds the post (logged), since `off` may be what it says.
async fn cadence_is_off(resources: &ServerContext, athlete: PostAthlete) -> bool {
    match resources
        .common
        .repos
        .check_in_preferences
        .check_in_cadence(athlete.tenant_id, athlete.user_id)
        .await
    {
        Ok(cadence) => cadence == Some(CheckInCadence::Off),
        Err(e) => {
            warn!(error = %e, "Landing post: the check-in cadence is unreadable; holding the post");
            true
        }
    }
}

/// Whether the athlete wrote to the coach within [`ACTIVE_CONVERSATION`] in
/// any of `conversations`, the places the post could land.
async fn mid_conversation(
    resources: &ServerContext,
    athlete: PostAthlete,
    conversations: &[&str],
    now: Timestamp,
) -> bool {
    let user = athlete.user_id.to_string();
    for conversation in conversations {
        // FirstPartyOnly admits every row: the strictest stamp is readable.
        let messages = match resources
            .common
            .repos
            .chat
            .get_recent_messages(
                conversation,
                &user,
                athlete.tenant_id,
                RECENT_MESSAGES,
                TransportPolicy::FirstPartyOnly,
            )
            .await
        {
            Ok(messages) => messages,
            Err(e) => {
                warn!(error = %e, "Landing post: recent messages unreadable");
                continue;
            }
        };
        let wrote_lately = messages
            .iter()
            .filter(|message| message.role == "user")
            .filter_map(|message| Timestamp::parse_from_rfc3339(&message.created_at).ok())
            .any(|at| now - at < ACTIVE_CONVERSATION);
        if wrote_lately {
            return true;
        }
    }
    false
}

/// Take the landing's claim under `frequency`: the day's one row, retaken
/// once its claim predates the athlete's local midnight, or this ride's own
/// row. `None` when a replica, a retry or an earlier landing holds it, or the
/// claim failed (logged).
async fn claim(
    resources: &ServerContext,
    athlete: PostAthlete,
    session: &LandedSession,
    frequency: LandingFrequency,
    zone: &Zone,
    now: Timestamp,
) -> Option<Claim> {
    let claims = &resources.common.repos.platform_post_claims;
    let purpose = PostPurpose::ActivityLanded.as_str();
    let (subject, claimed) = match frequency {
        LandingFrequency::PerDay => {
            let since_midnight = zone.civil_at(now).time().num_seconds_from_midnight();
            let midnight = now - Elapsed::seconds(i64::from(since_midnight));
            let claimed = claims
                .claim_platform_post_since(
                    athlete.tenant_id,
                    athlete.user_id,
                    purpose,
                    PostPurpose::NO_SUBJECT,
                    midnight,
                    now,
                )
                .await;
            (PostPurpose::NO_SUBJECT.to_owned(), claimed)
        }
        LandingFrequency::PerRide => {
            // A provider key never contains ':', so the pair is unambiguous
            // even when the id itself carries one.
            let subject = format!("{}:{}", session.provider, session.activity_id);
            let claimed = claims
                .claim_platform_post_once(
                    athlete.tenant_id,
                    athlete.user_id,
                    purpose,
                    &subject,
                    now - POST_CLAIM_LEASE,
                    now,
                )
                .await;
            (subject, claimed)
        }
    };
    match claimed {
        Ok(lease) => lease.map(|lease| Claim { subject, lease }),
        Err(e) => {
            warn!(error = %e, "Landing post: the claim failed");
            None
        }
    }
}

/// Mark the landing's post made, so its claim is never retaken.
async fn mark_posted(
    resources: &ServerContext,
    athlete: PostAthlete,
    claim: &Claim,
    now: Timestamp,
) {
    match resources
        .common
        .repos
        .platform_post_claims
        .mark_platform_post_posted(
            athlete.tenant_id,
            athlete.user_id,
            PostPurpose::ActivityLanded.as_str(),
            &claim.subject,
            &claim.lease,
            now,
        )
        .await
    {
        Ok(true) => {}
        Ok(false) => warn!("Landing post: the claim lapsed before the post was marked"),
        Err(e) => warn!(error = %e, "Landing post: marking the post made failed"),
    }
}

/// Release the claim of a landing nothing could be attempted for.
async fn release(resources: &ServerContext, athlete: PostAthlete, claim: &Claim) {
    if let Err(e) = resources
        .common
        .repos
        .platform_post_claims
        .release_platform_post(
            athlete.tenant_id,
            athlete.user_id,
            PostPurpose::ActivityLanded.as_str(),
            &claim.subject,
            &claim.lease,
        )
        .await
    {
        warn!(error = %e, "Landing post: releasing the claim of an unmade post failed");
    }
}

/// The landing's instruction in `locale`: the workout's local start, its id
/// and its provider, never its name (athlete text, read fenced through the
/// tool result). `None` (logged) when the catalogue does not carry it.
fn landing_prompt(
    resources: &ServerContext,
    locale: &str,
    start: &str,
    session: &LandedSession,
) -> Option<String> {
    let prompt = resources.mcp.messaging_strings_registry.render(
        KEY_PLATFORM_POST_ACTIVITY_LANDED_PROMPT,
        locale,
        &[start, &session.activity_id, &session.provider],
    );
    if prompt.trim().is_empty() {
        warn!(
            locale,
            "Landing post: the catalogue carries no instruction; nothing posted"
        );
        return None;
    }
    Some(prompt)
}

/// Report one landing post: where it was posted, and whether the agent's
/// reply grounded in the athlete's data and was stored.
fn report_landed(athlete: PostAthlete, surface: PostSurface, grounded: bool) {
    info!(
        target: "notify",
        event = "coach.activity_landed",
        user_id = %athlete.user_id,
        tenant_id = %athlete.tenant_id,
        destination = %surface.destination(),
        grounded,
        "coach reaction to a synced workout"
    );
}

/// Layer the landing poster onto the context's
/// [`SessionLandedRouter`](crate::services::session_landed::SessionLandedRouter),
/// now that the composition-root `Arc<ServerContext>` exists. A second call
/// is a startup logic bug, logged not fatal.
pub fn install_activity_landed(resources: &Arc<ServerContext>, post: Arc<dyn PlatformPost>) {
    let poster = Arc::new(ActivityLandedPoster::new(
        Arc::downgrade(resources),
        post,
        config_adapter_resolver(
            Arc::clone(&resources.common.repos),
            Arc::clone(&resources.mcp.channel_adapters),
        ),
    ));
    if !resources.mcp.session_landed.install(poster) {
        warn!("Landing poster already installed; skipping");
    }
}

#[cfg(test)]
mod tests {
    use pierre_core::models::TenantId;
    use uuid::Uuid;

    use super::*;

    #[tokio::test]
    async fn no_admin_config_reads_as_once_a_day_replacing_the_digest() {
        let athlete = PostAthlete {
            user_id: Uuid::new_v4(),
            tenant_id: TenantId::generate(),
        };
        assert_eq!(
            LandingConfig::resolve(None, athlete).await,
            LandingConfig {
                frequency: LandingFrequency::PerDay,
                digest: LandingDigest::ReplaceDigest,
            },
            "bug: a server with no admin config must take the documented defaults"
        );
    }

    #[test]
    fn only_a_post_that_replaced_the_digest_holds_it_back() {
        assert!(
            !Landed::Posted(PostSurface::InApp, LandingDigest::ReplaceDigest).sends_digest(),
            "bug: a stored post under replace_digest replaces the digest"
        );
        for landed in [
            Landed::Posted(PostSurface::InApp, LandingDigest::KeepDigest),
            Landed::Withheld,
            Landed::Skipped(SkipReason::QuietHours),
            Landed::Digest,
        ] {
            assert!(
                landed.sends_digest(),
                "bug: {landed:?} must leave the digest to go"
            );
        }
    }
}
