// ABOUTME: Background scheduler that sends each coaching group its weekly digest at a fixed local morning slot
// ABOUTME: Posts it into the group's bound chat over the members who share, and gives managers the full digest in-app
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Group Weekly-Digest Scheduler
//!
//! Sends every coaching group of a tenant whose tier enables the
//! `weekly_digest` flag
//! ([`pierre_groups::strategies::tier::GroupFeatureFlags::weekly_digest`],
//! `true` for Professional/Enterprise) one digest per week.
//!
//! ## When
//!
//! The worker ticks every [`DEFAULT_TICK_INTERVAL`] under the worker ledger.
//! A tick sends a group its digest only inside the group's slot — Monday 08:00
//! in the group's own zone, caught up later the same ISO week in daytime (see
//! [`crate::group_digest_slot`]) — and only once it has claimed that group's
//! week in the delivery ledger
//! ([`pierre_database::repositories::CoachingGroupRepository::claim_group_digest`]),
//! so across ticks, restarts and instances a group gets one digest a week.
//! The week is closed as soon as the attempt is over, whatever it delivered: a
//! digest split into several chat messages must never be re-sent part by
//! part. Only an instance that dies mid-send leaves its claim to lapse after
//! [`DIGEST_CLAIM_LEASE`], and a later tick in the slot retries.
//!
//! ## What, and to whom
//!
//! Each tick enumerates the tenants, keeps those whose tier enables the
//! digest, lists their active groups and, for a group whose week is due and
//! claimed, builds member snapshots through the canonical
//! [`fetch_member_snapshots`] builder (the same all-providers + deduplicated
//! path the chat agent and the REST analytics endpoints use).
//!
//! - **The group's chat.** A group bound to a Telegram group, a Slack channel
//!   or a Discord channel ([`digest_room`]) has its digest posted there
//!   through the [`GroupChatPoster`] seam. The chat copy is computed over the
//!   members who share their training
//!   ([`GroupService::peer_sharing_user_ids`], the rule the agent's group
//!   context applies) and names or counts nobody else
//!   ([`room_digest_params`]). When fewer than all share it opens by saying
//!   how many do and how to join in; when none do, that line is all it says.
//!   It is written in the language most members read ([`plurality_locale`]).
//!   A room whose `respond_mode` is mentions-only still receives it — that
//!   mode governs replies — and no transcript row is written for it.
//! - **Managers** (owner + admins) get the full digest over every member as a
//!   [`NotificationEvent::GroupWeeklyDigest`], which the notification
//!   localizer renders in each recipient's own language and the stored row
//!   keeps as parameters, so the feed renders it again after a language
//!   change. When the digest reached the group's chat they get it in the app
//!   only — the stored row and the device push — so no manager reads it twice
//!   in chat. A group with no chat, or bound to a channel that cannot take a
//!   proactive message, keeps the fan-out to each manager's linked channels.
//!
//! Everything is best-effort: a failed read, snapshot fetch, post or
//! notification is logged and counted, and never aborts the rest of the sweep.

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use chrono_tz::Tz;
use pierre_core::errors::AppResult;
use pierre_core::models::groups::{
    CoachingGroup, FlagEvidence, GroupAggregateStats, GroupHealthFlag, GroupMember, GroupTrend,
    MemberFitnessSnapshot, MemberFlag,
};
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::{TenantId, User};
use pierre_database::repositories::MessagingRepository;
use pierre_database::RepositoryRegistry;
use pierre_groups::strategies::tier::tier_strategy_for;
use pierre_groups::GroupService;
use pierre_notifications::events::NotificationEvent;
use pierre_runtime_context::{GroupsCtx, MiddlewareCtx};
use pierre_services::locale::user_locale;
use pierre_services::notification_text::{
    NotificationTextRenderer, CONCERN_DEEP_FATIGUE, CONCERN_HEAVY_BLOCK, CONCERN_INACTIVE,
    CONCERN_OVERTRAINING_RISK, CONCERN_VOLUME_DROP, PARAM_ROSTER_MEMBERS, PARAM_SHARED_MEMBERS,
};
use pierre_services::periodic::spawn_periodic;
use pierre_tool_runtime::group_fitness::fetch_member_snapshots;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Map, Value};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

#[cfg(feature = "client-notifications")]
use pierre_notifications::{
    models::NotificationCategory, EventDispatch, NotificationService, PushTier,
    TenantId as CommTenantId,
};

use crate::group_chat_poster::GroupChatPoster;
use crate::group_digest_slot::{due_week, group_zone};

/// How often the digest worker ticks.
///
/// The slot is an hour of the group's own day, so the tick has to be finer
/// than that; the per-group delivery ledger, not the tick, keeps each group to
/// one digest a week.
pub const DEFAULT_TICK_INTERVAL: StdDuration = StdDuration::from_mins(15);

/// How long a claimed group-week is held before a later tick may take it over.
///
/// Longer than the slowest snapshot fetch and send, short enough that a crash
/// mid-send is retried the same day.
pub const DIGEST_CLAIM_LEASE: StdDuration = StdDuration::from_hours(1);

/// Most members the digest lists by volume. A larger group keeps its summary,
/// trend, highlights and concerns, and loses only the tail of the roster.
pub const MAX_DIGEST_MEMBER_LINES: usize = 12;

/// The worker's name on every log line and its key in the worker ledger.
const WORKER_NAME: &str = "group weekly-digest scheduler";

/// Outcome of a single scheduler tick — exposed for tests and metrics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DigestTickOutcome {
    /// Tenants examined this tick.
    pub tenants_scanned: usize,
    /// Tenants whose tier enables `weekly_digest`.
    pub tenants_eligible: usize,
    /// Groups whose week was due and claimed, and whose digest was built.
    pub groups_reported: usize,
    /// Groups whose digest reached their own chat.
    pub room_posts: usize,
    /// Manager notification attempts (a no-op when no notification service).
    pub dispatched: usize,
    /// Read, claim, dispatch or post errors. Logged; the sweep continues.
    pub errors: usize,
}

/// Where a group's digest is posted: its bound chat, on a channel that takes
/// a message the platform starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DigestRoom<'a> {
    /// The channel the group is bound on.
    pub channel: ChannelType,
    /// The channel-native id of the group's chat.
    pub chat_id: &'a str,
}

/// What one sweep hands every group it visits.
struct Sweep<'a, C> {
    /// The server context: repositories, the group service and the strings.
    ctx: &'a Arc<C>,
    /// The same context as the runtime the snapshot builder takes.
    runtime: &'a Arc<dyn ToolRuntime>,
    /// The managers' notification service; `None` when not configured.
    #[cfg(feature = "client-notifications")]
    notifications: Option<&'a NotificationService>,
    /// The chat poster; `None` in a build without messaging.
    poster: Option<&'a dyn GroupChatPoster>,
    /// The instant the sweep started; every group's slot is read at it.
    now: DateTime<Utc>,
    /// Monotonic start of the sweep, so a claim late in a long sweep leases
    /// from when it was taken rather than from when the sweep began.
    started: Instant,
}

impl<C> Sweep<'_, C> {
    /// The current instant on the sweep's clock: its start plus the time the
    /// sweep has run. A lease written from it lasts its full length however
    /// late in the sweep the group is reached.
    fn clock(&self) -> DateTime<Utc> {
        self.now + ChronoDuration::from_std(self.started.elapsed()).unwrap_or_default()
    }
}

/// The people a group's digest is about and for: its live members, and the
/// user rows of everyone whose zone or language it reads — members, owner
/// and human coach — from one batch query.
struct Audience {
    /// Live members, in join order.
    members: Vec<GroupMember>,
    /// User rows by id.
    users: HashMap<Uuid, User>,
}

impl Audience {
    /// The zone the group's slot is read in (see [`group_zone`]).
    fn zone(&self, group: &CoachingGroup) -> Option<Tz> {
        let zone_of = |id: Uuid| self.users.get(&id).and_then(|u| u.timezone.as_deref());
        group_zone(
            zone_of(group.owner_id),
            group.coach_user_id.and_then(zone_of),
            self.members.iter().map(|m| zone_of(m.user_id)),
        )
    }
}

/// Run a single digest sweep across all tenants, reading the slot at `now`.
///
/// Exposed (rather than only the loop) so integration tests can drive one
/// sweep at a chosen instant without `tokio::sleep`. Production ticks it from
/// [`start_digest_scheduler`] with the wall clock.
///
/// `notification_service` and `poster` are `Option`s because some build
/// profiles compile out notifications or messaging entirely.
///
/// # Errors
///
/// Returns the database error only if the initial `get_all` tenant query
/// fails. Per-tenant and per-group errors are counted in
/// `DigestTickOutcome.errors` but do not abort the sweep.
pub async fn tick<C>(
    ctx: &Arc<C>,
    runtime: &Arc<dyn ToolRuntime>,
    #[cfg(feature = "client-notifications")] notification_service: Option<&NotificationService>,
    poster: Option<&dyn GroupChatPoster>,
    now: DateTime<Utc>,
) -> AppResult<DigestTickOutcome>
where
    C: ToolRuntime + GroupsCtx + MiddlewareCtx,
{
    let tenants = MiddlewareCtx::repos(ctx.as_ref()).tenants.get_all().await?;
    let sweep = Sweep {
        ctx,
        runtime,
        #[cfg(feature = "client-notifications")]
        notifications: notification_service,
        poster,
        now,
        started: Instant::now(),
    };

    let mut outcome = DigestTickOutcome {
        tenants_scanned: tenants.len(),
        ..DigestTickOutcome::default()
    };

    for tenant in tenants {
        if !tier_enables_digest(&tenant.plan) {
            continue;
        }
        outcome.tenants_eligible += 1;
        process_tenant(&sweep, tenant.id, &mut outcome).await;
    }

    if outcome.groups_reported > 0 || outcome.errors > 0 {
        info!(
            tenants_scanned = outcome.tenants_scanned,
            tenants_eligible = outcome.tenants_eligible,
            groups_reported = outcome.groups_reported,
            room_posts = outcome.room_posts,
            dispatched = outcome.dispatched,
            errors = outcome.errors,
            "group weekly-digest sweep complete"
        );
    } else {
        debug!(
            tenants_scanned = outcome.tenants_scanned,
            tenants_eligible = outcome.tenants_eligible,
            "group weekly-digest sweep: no group due this tick"
        );
    }

    Ok(outcome)
}

/// Whether a tenant plan's tier enables the `weekly_digest` feature. This is
/// the read that makes the previously-dormant tier flag load-bearing.
#[must_use]
pub fn tier_enables_digest(plan: &str) -> bool {
    tier_strategy_for(plan).allowed_features().weekly_digest
}

/// The chat a group's digest is posted into, or `None` when it has none that
/// takes one.
///
/// A REST-created group has no chat. `WhatsApp` and Messenger refuse a text
/// the platform starts outside the 24-hour window after a member's last
/// message, so a group bound there keeps its managers' full notification
/// instead of a post that would bounce.
#[must_use]
pub fn digest_room(group: &CoachingGroup) -> Option<DigestRoom<'_>> {
    let channel = ChannelType::from_str(group.channel_type.as_deref()?).ok()?;
    let chat_id = group
        .channel_chat_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())?;
    matches!(
        channel,
        ChannelType::Telegram | ChannelType::Slack | ChannelType::Discord
    )
    .then_some(DigestRoom { channel, chat_id })
}

/// Process every active group for one eligible tenant.
async fn process_tenant<C>(
    sweep: &Sweep<'_, C>,
    tenant_id: TenantId,
    outcome: &mut DigestTickOutcome,
) where
    C: ToolRuntime + GroupsCtx + MiddlewareCtx,
{
    let groups = match MiddlewareCtx::repos(sweep.ctx.as_ref())
        .groups
        .list_active_groups_for_tenant(tenant_id)
        .await
    {
        Ok(groups) => groups,
        Err(e) => {
            error!(tenant_id = %tenant_id, error = %e, "digest: failed to list active groups");
            outcome.errors += 1;
            return;
        }
    };

    for group in groups {
        process_group(sweep, tenant_id, &group, outcome).await;
    }
}

/// Send one group its digest when its week is due and this tick claims it.
///
/// `tenant_id` is the group's own tenant — the listing is scoped by it — which
/// is also the tenant that owns the channel config of the group's chat.
async fn process_group<C>(
    sweep: &Sweep<'_, C>,
    tenant_id: TenantId,
    group: &CoachingGroup,
    outcome: &mut DigestTickOutcome,
) where
    C: ToolRuntime + GroupsCtx + MiddlewareCtx,
{
    let repos = MiddlewareCtx::repos(sweep.ctx.as_ref());
    let Some(audience) = load_audience(repos, group, outcome).await else {
        return;
    };
    let Some(week_key) = due_week(sweep.now, audience.zone(group), group.created_at) else {
        return;
    };
    if !claim_week(repos, tenant_id, group, &week_key, sweep.clock(), outcome).await {
        return;
    }

    let user_ids: Vec<Uuid> = audience.members.iter().map(|m| m.user_id).collect();
    let snapshots = fetch_member_snapshots(sweep.runtime, &user_ids, tenant_id).await;
    outcome.groups_reported += 1;
    let posted = post_digest_to_room(sweep, tenant_id, group, &audience, &snapshots, outcome).await;

    // The week closes as soon as the chat has been tried, before the managers
    // are told: a crash in the managers' fan-out then costs their copies, never
    // a second post into the group's chat once the claim lapses.
    close_week(repos, tenant_id, group, &week_key, sweep.clock(), outcome).await;

    #[cfg(feature = "client-notifications")]
    dispatch_to_managers(
        sweep, tenant_id, group, &audience, &snapshots, posted, outcome,
    )
    .await;

    debug!(
        group_id = %group.id,
        members = audience.members.len(),
        posted_to_room = posted,
        "digest: weekly report delivered"
    );
}

/// Mark the group's `week_key` delivered in the ledger. A failure is counted:
/// the claim then lapses and a later tick may send the week again.
async fn close_week(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    group: &CoachingGroup,
    week_key: &str,
    now: DateTime<Utc>,
    outcome: &mut DigestTickOutcome,
) {
    if let Err(e) = repos
        .groups
        .finish_group_digest(tenant_id, group.id, week_key, now.timestamp_millis())
        .await
    {
        error!(
            group_id = %group.id,
            week = %week_key,
            error = %e,
            "digest: failed to close the week; a later tick may send it again once the claim lapses"
        );
        outcome.errors += 1;
    }
}

/// The group's live members and the user rows its zone and language are read
/// from, or `None` when the group has no member or a read failed.
async fn load_audience(
    repos: &RepositoryRegistry,
    group: &CoachingGroup,
    outcome: &mut DigestTickOutcome,
) -> Option<Audience> {
    let members = match repos.groups.list_members(&group.id.to_string()).await {
        Ok(members) => members,
        Err(e) => {
            error!(group_id = %group.id, error = %e, "digest: failed to list members");
            outcome.errors += 1;
            return None;
        }
    };
    if members.is_empty() {
        return None;
    }

    let mut ids: Vec<Uuid> = members.iter().map(|m| m.user_id).collect();
    ids.push(group.owner_id);
    ids.extend(group.coach_user_id);
    ids.sort_unstable();
    ids.dedup();
    match repos.users.get_global_many(&ids).await {
        Ok(users) => Some(Audience { members, users }),
        Err(e) => {
            error!(group_id = %group.id, error = %e, "digest: failed to read the members' profiles");
            outcome.errors += 1;
            None
        }
    }
}

/// Claim the group's `week_key` in the delivery ledger. `false` when the week
/// was already sent, another tick holds it, or the claim failed.
async fn claim_week(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    group: &CoachingGroup,
    week_key: &str,
    now: DateTime<Utc>,
    outcome: &mut DigestTickOutcome,
) -> bool {
    let lease_ms = i64::try_from(DIGEST_CLAIM_LEASE.as_millis()).unwrap_or(i64::MAX);
    match repos
        .groups
        .claim_group_digest(
            tenant_id,
            group.id,
            week_key,
            now.timestamp_millis(),
            lease_ms,
        )
        .await
    {
        Ok(true) => true,
        Ok(false) => {
            debug!(group_id = %group.id, week = %week_key, "digest: week already sent or held by another tick");
            false
        }
        Err(e) => {
            error!(group_id = %group.id, week = %week_key, error = %e, "digest: failed to claim the week");
            outcome.errors += 1;
            false
        }
    }
}

/// Post the digest into the group's chat when it has one that takes a
/// proactive message. Whether any of it arrived.
async fn post_digest_to_room<C>(
    sweep: &Sweep<'_, C>,
    tenant_id: TenantId,
    group: &CoachingGroup,
    audience: &Audience,
    snapshots: &[MemberFitnessSnapshot],
    outcome: &mut DigestTickOutcome,
) -> bool
where
    C: ToolRuntime + GroupsCtx + MiddlewareCtx,
{
    let Some(room) = digest_room(group) else {
        return false;
    };
    let posted = post_to_room(sweep, tenant_id, group, room, audience, snapshots, outcome).await;
    if posted {
        outcome.room_posts += 1;
    }
    posted
}

/// Post the chat copy of the digest into the group's chat, in the language
/// most members read. Whether any of it arrived; a chat that took none of it
/// counts as an error, so a room that refuses every week shows in the sweep.
async fn post_to_room<C>(
    sweep: &Sweep<'_, C>,
    tenant_id: TenantId,
    group: &CoachingGroup,
    room: DigestRoom<'_>,
    audience: &Audience,
    snapshots: &[MemberFitnessSnapshot],
    outcome: &mut DigestTickOutcome,
) -> bool
where
    C: ToolRuntime + GroupsCtx + MiddlewareCtx,
{
    let Some(poster) = sweep.poster else {
        return false;
    };
    let ctx = sweep.ctx.as_ref();
    let params = room_digest_params(ctx.group_service(), group, &audience.members, snapshots);
    let locale = room_locale(
        MiddlewareCtx::repos(ctx),
        tenant_id,
        group,
        room.channel,
        audience,
    )
    .await;
    let empty = Map::new();
    let text = NotificationTextRenderer::new(GroupsCtx::messaging_strings_registry(ctx), &locale)
        .channel_text(
            NotificationEvent::GroupWeeklyDigest,
            params.as_object().unwrap_or(&empty),
        );

    let parts = poster
        .post(tenant_id, room.channel, room.chat_id, &text)
        .await;
    if parts == 0 {
        outcome.errors += 1;
        warn!(
            group_id = %group.id,
            channel = %room.channel,
            "digest: nothing reached the group's chat; its managers get the digest on their own channels"
        );
    } else {
        info!(group_id = %group.id, channel = %room.channel, parts, locale = %locale, "digest: posted into the group's chat");
    }
    parts > 0
}

/// The parameters of the digest posted into a group's chat.
///
/// [`digest_params`] over the members who share their training
/// ([`GroupService::peer_sharing_user_ids`]) and nobody else — the summary,
/// the trend, the volume lines, the highlights and the concerns are all
/// computed over that set, so every line of the chat copy, the all-clear
/// included, is true of it. It also carries how many members that is and how
/// many the group holds, which the renderer turns into its scope line.
#[must_use]
pub fn room_digest_params(
    service: &GroupService,
    group: &CoachingGroup,
    members: &[GroupMember],
    snapshots: &[MemberFitnessSnapshot],
) -> Value {
    let sharing = GroupService::peer_sharing_user_ids(group, members);
    // The reducers take owned slices; this copy is made once per group per week.
    let shared: Vec<MemberFitnessSnapshot> = snapshots
        .iter()
        .filter(|s| sharing.contains(&s.user_id))
        .cloned()
        .collect();
    let stats = service.compute_aggregate_stats(&shared);
    let flags = GroupService::compute_health_flags(&shared);
    let mut params = digest_params(&group.name, &stats, &shared, &flags);
    if let Some(object) = params.as_object_mut() {
        object.insert(PARAM_SHARED_MEMBERS.to_owned(), json!(shared.len()));
        object.insert(PARAM_ROSTER_MEMBERS.to_owned(), json!(members.len()));
    }
    params
}

/// The language a group's chat reads its digest in: each member votes with
/// the locale they read on that channel ([`member_locale`]), the most-read
/// one wins and the owner's wins a tie ([`plurality_locale`]).
async fn room_locale(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    group: &CoachingGroup,
    channel: ChannelType,
    audience: &Audience,
) -> String {
    let messaging = repos.messaging.as_ref();
    let mut votes = Vec::with_capacity(audience.members.len());
    for member in &audience.members {
        let user = audience.users.get(&member.user_id);
        votes.push(member_locale(messaging, tenant_id, channel, member.user_id, user).await);
    }
    let owner_vote = audience
        .members
        .iter()
        .zip(&votes)
        .find(|(member, _)| member.user_id == group.owner_id);
    let owner = if let Some((_, locale)) = owner_vote {
        locale.clone()
    } else {
        let user = audience.users.get(&group.owner_id);
        member_locale(messaging, tenant_id, channel, group.owner_id, user).await
    };
    plurality_locale(votes.iter().map(String::as_str), &owner)
}

/// One person's language on the group's channel: their locale override for
/// that channel, else the locale on their profile, else the default — the
/// chain the messaging ingress resolves. The links are read under the group's
/// tenant, which owns the channel and every link made through it.
async fn member_locale(
    messaging: &dyn MessagingRepository,
    tenant_id: TenantId,
    channel: ChannelType,
    user_id: Uuid,
    user: Option<&User>,
) -> String {
    let channel = channel.to_string();
    let links = messaging
        .list_user_channel_links(tenant_id, &user_id.to_string())
        .await
        .unwrap_or_else(|e| {
            debug!(user_id = %user_id, error = %e, "digest: channel links unreadable; using the profile locale");
            Vec::new()
        });
    links
        .iter()
        .filter(|link| link.get("channel_type").and_then(Value::as_str) == Some(channel.as_str()))
        .find_map(|link| {
            link.get("locale")
                .and_then(Value::as_str)
                .filter(|locale| !locale.trim().is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| user_locale(user))
}

/// The language most of `votes` name; among equals the owner's when it is one
/// of them, else the first one voted. With no votes, the owner's.
#[must_use]
pub fn plurality_locale<'a>(votes: impl IntoIterator<Item = &'a str>, owner: &str) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for vote in votes {
        match counts.iter_mut().find(|(locale, _)| *locale == vote) {
            Some((_, count)) => *count += 1,
            None => counts.push((vote, 1)),
        }
    }
    let top = counts.iter().map(|(_, count)| *count).max().unwrap_or(0);
    let leaders: Vec<&str> = counts
        .iter()
        .filter(|(_, count)| *count == top)
        .map(|(locale, _)| *locale)
        .collect();
    leaders
        .iter()
        .find(|locale| **locale == owner)
        .or_else(|| leaders.first())
        .map_or(owner, |locale| *locale)
        .to_owned()
}

/// Notify every member who can manage the group (owner + admins) of the full
/// digest over every member. Best-effort per recipient.
///
/// When the digest reached the group's chat the notification stays in the
/// app, so a manager does not read it a second time on their own chat
/// channels; otherwise it fans out to those channels as any notification does.
#[cfg(feature = "client-notifications")]
async fn dispatch_to_managers<C>(
    sweep: &Sweep<'_, C>,
    tenant_id: TenantId,
    group: &CoachingGroup,
    audience: &Audience,
    snapshots: &[MemberFitnessSnapshot],
    posted_to_room: bool,
    outcome: &mut DigestTickOutcome,
) where
    C: ToolRuntime + GroupsCtx + MiddlewareCtx,
{
    let Some(service) = sweep.notifications else {
        return;
    };
    let stats = sweep.ctx.group_service().compute_aggregate_stats(snapshots);
    let flags = GroupService::compute_health_flags(snapshots);
    let params = digest_params(&group.name, &stats, snapshots, &flags);

    for member in audience
        .members
        .iter()
        .filter(|m| m.role.can_manage_members())
    {
        outcome.dispatched += 1;
        let dispatch = EventDispatch {
            user_id: member.user_id,
            tenant_id: CommTenantId(tenant_id.into()),
            category: NotificationCategory::Coach,
            event: NotificationEvent::GroupWeeklyDigest,
            params: params.clone(),
            route: Value::Null,
            actions: None,
            bypass_frequency_cap: false,
        };
        // P3: a weekly roll-up is ambient by construction — any persona floor
        // below "everything" prefers it in the in-app list over a push.
        let delivery = if posted_to_room {
            service.dispatch_event_in_app(&dispatch, PushTier::P3).await
        } else {
            service.dispatch_event(&dispatch, PushTier::P3).await
        };
        if let Err(e) = delivery {
            warn!(
                group_id = %group.id,
                user_id = %member.user_id,
                error = %e,
                "digest: notification dispatch failed (best-effort)"
            );
            outcome.errors += 1;
        }
    }
}

/// The parameters of one group's [`NotificationEvent::GroupWeeklyDigest`].
///
/// Numbers stay numbers and states stay codes, so the renderer can phrase
/// them in each reader's language: the summary counts, the trend, the
/// members by weekly volume (at most [`MAX_DIGEST_MEMBER_LINES`]), the members
/// in fresh form, and one concern per health flag.
#[must_use]
pub fn digest_params(
    group_name: &str,
    stats: &GroupAggregateStats,
    snapshots: &[MemberFitnessSnapshot],
    flags: &[GroupHealthFlag],
) -> Value {
    let trend = match stats.weekly_trend {
        GroupTrend::Improving => "improving",
        GroupTrend::Stable => "stable",
        GroupTrend::Declining => "declining",
    };

    let mut by_volume: Vec<&MemberFitnessSnapshot> = snapshots.iter().collect();
    by_volume.sort_by(|a, b| b.weekly_volume_km.total_cmp(&a.weekly_volume_km));
    let members: Vec<Value> = by_volume
        .into_iter()
        .take(MAX_DIGEST_MEMBER_LINES)
        .map(|s| {
            json!({
                "name": s.display_name,
                "km": s.weekly_volume_km,
                "prev_km": s.previous_week_volume_km,
            })
        })
        .collect();

    let highlights: Vec<Value> = GroupService::fresh_members(snapshots)
        .map(|(s, form_pct)| json!({ "name": s.display_name, "form_pct": form_pct }))
        .collect();

    let concerns: Vec<Value> = flags
        .iter()
        .map(|flag| {
            let (code, value) = match flag.evidence {
                FlagEvidence::FormShare { form_pct, .. }
                    if flag.flag_type == MemberFlag::DeepFatigue =>
                {
                    (CONCERN_DEEP_FATIGUE, json!(form_pct))
                }
                FlagEvidence::FormShare { form_pct, .. } => (CONCERN_HEAVY_BLOCK, json!(form_pct)),
                FlagEvidence::OvertrainingRisk => (CONCERN_OVERTRAINING_RISK, Value::Null),
                FlagEvidence::InactiveDays { days } => (CONCERN_INACTIVE, json!(days)),
                FlagEvidence::VolumeBelowGroup { pct_below } => {
                    (CONCERN_VOLUME_DROP, json!(pct_below))
                }
            };
            json!({ "code": code, "name": flag.display_name, "value": value })
        })
        .collect();

    json!({
        "group_name": group_name,
        "active_members": stats.active_members,
        "total_members": stats.total_members,
        "avg_volume_km": stats.avg_weekly_volume_km,
        "trend": trend,
        "members": members,
        "highlights": highlights,
        "concerns": concerns,
    })
}

/// Spawn the weekly-digest scheduler as a background tokio task.
///
/// Called once at server bootstrap. The task runs for the server's lifetime,
/// ticking every [`DEFAULT_TICK_INTERVAL`] on the wall clock; the
/// [`AbortHandle`](tokio::task::AbortHandle) is discarded because the
/// scheduler is best-effort and both the worker ledger and the per-group
/// delivery ledger carry its state across restarts. `poster` is the seam into
/// the groups' chats, `None` in a build without messaging.
pub fn start_digest_scheduler<C>(
    ctx: Arc<C>,
    #[cfg(feature = "client-notifications")] notification_service: Option<Arc<NotificationService>>,
    poster: Option<Arc<dyn GroupChatPoster>>,
) where
    C: ToolRuntime + GroupsCtx + MiddlewareCtx,
{
    // The same context, once as itself and once as the runtime trait object
    // the tick signature takes; both are cloned per tick so the closure stays
    // callable for the life of the worker.
    let cloned: Arc<C> = Arc::clone(&ctx);
    let runtime: Arc<dyn ToolRuntime> = cloned;
    let ledger = Arc::clone(&MiddlewareCtx::repos(ctx.as_ref()).worker_runs);

    spawn_periodic(WORKER_NAME, DEFAULT_TICK_INTERVAL, ledger, move || {
        let ctx = Arc::clone(&ctx);
        let runtime = Arc::clone(&runtime);
        #[cfg(feature = "client-notifications")]
        let notification_service = notification_service.clone();
        let poster = poster.clone();
        async move {
            tick(
                &ctx,
                &runtime,
                #[cfg(feature = "client-notifications")]
                notification_service.as_deref(),
                poster.as_deref(),
                Utc::now(),
            )
            .await?;
            Ok(())
        }
    });
}
