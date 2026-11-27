// ABOUTME: The human coach's roster from the database: who may read it, which athletes it holds, their consent and the order
// ABOUTME: Read by get_roster_overview, GET /api/coach/roster and the reply review; the training half is pierre_tool_runtime::coach_roster
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coach roster
//!
//! The athletes a human coach holds. The agent tool (`get_roster_overview`),
//! the coach's REST read (`GET /api/coach/roster`, carnet#1251) and the reply
//! review's filters ([`crate::coach_replies`]) all read the roster here; none
//! reimplements a rule below. The per-athlete training is read on top of it
//! by `pierre_tool_runtime::coach_roster`.
//!
//! 1. **Who may read it.** A user who holds coach access
//!    ([`User::holds_coach_access`](pierre_core::models::User::holds_coach_access):
//!    `manages_roster`, or platform admin) — the rule that lets them be
//!    attached as a group's coach at all
//!    (`pierre_groups::creation_policy::may_coach_group`) — and is attached
//!    to at least one active group as its coach. [`roster_access`] answers
//!    it, and it is the one coach rule (carnet#1511): the roster tool and
//!    route above, every act for one athlete
//!    ([`coached_athlete_access`](crate::coached_athlete::coached_athlete_access)),
//!    the name match in front of those acts, and what `get_provider_roster`
//!    tells a caller of their standing all ask it. A coach whose access was
//!    withdrawn after attaching stays attached and reads nothing as a
//!    coach ([`RosterAccess::NoCoachAccess`], which carries the groups they
//!    are attached to so that an act for one group is refused that way for
//!    those groups only). The reply review's filters read rules 2 and 3 for
//!    the groups the caller is attached to, and do not ask this one.
//! 2. **The roster is the coach attachment.** The active groups whose
//!    `coach_user_id` is the caller and their live members, the caller
//!    excluded — the set `list_athletes_coached_by` returns.
//! 3. **Consent is the member's coach consent** (ADR-002, amendment
//!    2026-10-23). Shared through any one group is shared. An athlete who
//!    does not share is named with their status only: no number, nothing
//!    inferred from one.
//! 4. **Order.** Shared athletes with a source first, the longest without a
//!    session at the top, ordered on the activity cache so a page can be cut
//!    before any provider is read; then the athletes whose training is not
//!    listed, by name. Served to an outside client ([`order_roster`]'s
//!    `outside` terms), a session that client may not be shown never places
//!    an athlete, and a sharer whose every connected service keeps its data
//!    in Dravr is listed after the readable sharers, by name, as present with
//!    their data withheld: never first as if they had gone quiet (carnet#1511).

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};

use dravr_tronc::dates::{Elapsed, Timestamp};
use futures_util::stream::{self, StreamExt, TryStreamExt};
use serde::Serialize;
use uuid::Uuid;

use pierre_core::ai_policy::{first_party_only, ProviderTerms};
use pierre_core::constants::oauth_providers::is_consent_class;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::groups::{CoachingGroup, GroupMember};
use pierre_core::models::{Activity, TenantId};
use pierre_database::RepositoryRegistry;

use crate::athlete_display_name::fetch_user_display_name;
use crate::provider_notice::ai_consent_withheld;

/// Per-athlete roster lookups (connections, names, cache dates, a row's plan
/// reads) run at once. Each is a short database read, so a roster of a few
/// hundred is not read one round-trip after another.
pub const ROSTER_LOOKUP_CONCURRENCY: usize = 8;

/// One athlete's place on the roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RosterAthleteStatus {
    /// Their training is shared with the coach and listed.
    Shared,
    /// They revoked the coach's read in every group the coach holds them in
    /// (`/group consent coach no`); `/group consent coach yes` restores it.
    NoCoachConsent,
    /// They share, but no training source is connected.
    NoSource,
}

impl RosterAthleteStatus {
    /// The athlete's standing through one membership: shared unless they
    /// revoked the coach's read there.
    const fn of_member(member: &GroupMember) -> Self {
        if member.coach_sharing_consent {
            Self::Shared
        } else {
            Self::NoCoachConsent
        }
    }
}

/// The `reason` every refusal carries when the caller is attached to a group
/// as its coach and their coach access is off: `details.reason` on a route, a
/// tool refusal's `reason`.
pub const NO_COACH_ACCESS: &str = "no_coach_access";

/// Whether the caller may read a roster, and the groups it covers.
#[derive(Debug)]
pub enum RosterAccess {
    /// The active groups the caller is attached to as their coach, never
    /// empty; the caller holds coach access.
    Granted(Vec<CoachingGroup>),
    /// The caller does not hold coach access — it was withdrawn after they
    /// were attached — and these are the active groups they are attached to
    /// as their coach, never empty. Nothing of them is read as their coach:
    /// a caller asked about one group tells from them whether it is one of
    /// the caller's, and a caller who is also a member of one reads it as
    /// any member does.
    NoCoachAccess(Vec<CoachingGroup>),
    /// Anyone else: attached to no active group, with or without coach
    /// access.
    NotACoach,
}

/// Who may read a roster, and over which groups (rule 1): the one coach rule.
///
/// # Errors
///
/// Returns the repository error from the user or group lookup.
pub async fn roster_access(repos: &RepositoryRegistry, coach_id: Uuid) -> AppResult<RosterAccess> {
    let groups: Vec<CoachingGroup> = repos
        .groups
        .list_groups_coached_by(coach_id)
        .await?
        .into_iter()
        .filter(|g| g.is_active)
        .collect();
    if groups.is_empty() {
        return Ok(RosterAccess::NotACoach);
    }
    let holds_coach_access = repos
        .users
        .get_global(coach_id)
        .await?
        .is_some_and(|user| user.holds_coach_access());
    Ok(if holds_coach_access {
        RosterAccess::Granted(groups)
    } else {
        RosterAccess::NoCoachAccess(groups)
    })
}

/// Whether `user_id` coaches the active group `group_id` by the one coach
/// rule ([`roster_access`]).
///
/// That is: attached to it as its coach and holding coach access. A coach whose access was taken back reads the group's room as any
/// member does, so the chat listing's room classes and the coach's seat ask
/// this, as every coach tool's gate does.
///
/// # Errors
///
/// Returns the repository error from the user or group lookup.
pub async fn coaches_group(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    group_id: &str,
) -> AppResult<bool> {
    Ok(match roster_access(repos, user_id).await? {
        RosterAccess::Granted(groups) => groups.iter().any(|g| g.id.to_string() == group_id),
        RosterAccess::NoCoachAccess(_) | RosterAccess::NotACoach => false,
    })
}

/// A group named on a roster row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct RosterGroupRef {
    /// The group's id.
    pub id: String,
    /// The group's name.
    pub name: String,
}

/// One group the roster covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, schemars::JsonSchema)]
pub struct RosterGroupSummary {
    /// The group's id.
    pub id: String,
    /// The group's name.
    pub name: String,
    /// Live athletes in the group.
    pub athlete_count: usize,
}

/// An athlete gathered across the coach's groups, before any training is read.
#[derive(Debug, Clone)]
pub struct RosterMember {
    /// The athlete.
    pub user_id: Uuid,
    /// The tenant of the group through which the athlete shares (or, when
    /// they share through none, of the first group holding them). Their AI
    /// consents are looked up under it.
    pub tenant: TenantId,
    /// The coach's groups the athlete belongs to.
    pub groups: Vec<RosterGroupRef>,
    /// Their standing on the roster.
    pub status: RosterAthleteStatus,
    /// A group through which they share with the coach, for the by-id coaching
    /// gate; `None` when they share through none.
    pub shared_via: Option<Uuid>,
    /// The name a membership row gives them, when one does; the first group
    /// holding them that names them wins.
    pub member_display_name: Option<String>,
    /// The tenants their connected sources live under — where their cached
    /// activities are. Empty for anyone the coach may not read.
    pub source_tenants: Vec<TenantId>,
    /// The services they connected, as the connection rows name them. Empty
    /// for anyone the coach may not read.
    pub source_providers: Vec<String>,
    /// When they last stopped sharing with the coach: the latest known
    /// instant across the coach's groups holding them, only while the status
    /// is `NoCoachConsent` and only when one is recorded. `None` otherwise,
    /// including for a revoke from before the instant was recorded.
    pub sharing_stopped_at: Option<Timestamp>,
    /// Their first live membership among the coach's groups: when they joined
    /// the coach, and through which group.
    pub joined: RosterJoin,
    /// They allowed the coach's AI assistant to include them in the roster
    /// tools through a group in which they share with the coach
    /// ([`CoachConsents::assistant_read`](pierre_core::models::groups::CoachConsents::assistant_read),
    /// carnet#1511).
    pub assistant_read: bool,
}

/// When an athlete joined one of the coach's groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RosterJoin {
    /// When they joined.
    pub at: Timestamp,
    /// The group they joined.
    pub group: RosterGroupRef,
}

/// The roster before any training is read.
#[derive(Debug, Clone)]
pub struct GatheredRoster {
    /// The groups covered, with their live athlete count.
    pub groups: Vec<RosterGroupSummary>,
    /// Every athlete once, with the most useful status any group grants.
    pub members: Vec<RosterMember>,
}

impl GatheredRoster {
    /// Athletes whose training the roster shows the coach: `shared` only.
    ///
    /// A `no_source` athlete consented but has nothing to share, and the
    /// route gives their row no training and no plan, as for one who does not
    /// share; counting them here would make the scope line ("42 of 45 share")
    /// one more than the rows every client lists as sharers.
    #[must_use]
    pub fn visible_members(&self) -> usize {
        self.members
            .iter()
            .filter(|m| m.status == RosterAthleteStatus::Shared)
            .count()
    }

    /// Athletes the roster holds.
    #[must_use]
    pub fn roster_members(&self) -> usize {
        self.members.len()
    }

    /// The roster as an outside client acting for the coach is served it:
    /// without the athletes who share with the coach in none of the groups
    /// (EFVP 2026-11-17, decision 9.4c), and each group counting only the
    /// athletes left. Dravr's own surfaces read the whole roster.
    #[must_use]
    pub fn served_to_outside_client(mut self) -> Self {
        self.members
            .retain(|member| member.status != RosterAthleteStatus::NoCoachConsent);
        for group in &mut self.groups {
            group.athlete_count = self
                .members
                .iter()
                .filter(|member| member.groups.iter().any(|held| held.id == group.id))
                .count();
        }
        self
    }

    /// The roster as the coach's AI assistant is served it by the roster
    /// tools (carnet#1511): only the athletes who allowed it through a group
    /// in which they share with the coach, each group counting only them, and
    /// how many athletes were left out. A member who does not share is left
    /// out with the others, never told apart from them (EFVP 9.4c).
    #[must_use]
    pub fn included_by_coach_assistant(mut self) -> (Self, usize) {
        let before = self.members.len();
        self.members.retain(|member| member.assistant_read);
        let left_out = before - self.members.len();
        for group in &mut self.groups {
            group.athlete_count = self
                .members
                .iter()
                .filter(|member| member.groups.iter().any(|held| held.id == group.id))
                .count();
        }
        (self, left_out)
    }
}

/// Parse a tenant id read from a stored row. One that does not parse is a
/// corrupt row, reported as such rather than read under a made-up tenant.
fn stored_tenant(table: &str, tenant_id: &str) -> AppResult<TenantId> {
    Uuid::parse_str(tenant_id)
        .map(TenantId::from_uuid)
        .map_err(|e| AppError::database(format!("{table}.tenant_id {tenant_id:?}: {e}")))
}

/// Gather each live athlete of `groups` once (rules 2 and 3), and mark those
/// who share but have no connected source: the roster both coach reads list.
///
/// # Errors
///
/// Returns the repository error from a member or connection lookup.
pub async fn gather_roster(
    repos: &RepositoryRegistry,
    coach_id: Uuid,
    groups: &[CoachingGroup],
) -> AppResult<GatheredRoster> {
    let mut roster = gather_members(repos, coach_id, groups).await?;
    mark_sources(repos, &mut roster.members).await?;
    Ok(roster)
}

/// Gather each live athlete of `groups` once with their coach consent
/// (rules 2 and 3), reading memberships only — no connection, no training.
/// The reply review's filters read this alone.
///
/// # Errors
///
/// Returns the repository error from a member lookup, or a database error
/// for a group row whose tenant does not parse.
pub async fn gather_members(
    repos: &RepositoryRegistry,
    coach_id: Uuid,
    groups: &[CoachingGroup],
) -> AppResult<GatheredRoster> {
    let mut summaries = Vec::with_capacity(groups.len());
    let mut members: Vec<RosterMember> = Vec::new();
    let mut index: HashMap<Uuid, usize> = HashMap::new();
    for group in groups {
        let tenant = stored_tenant("coaching_groups", &group.tenant_id)?;
        let group_ref = RosterGroupRef {
            id: group.id.to_string(),
            name: group.name.clone(),
        };
        let rows = repos.groups.list_members(&group.id.to_string()).await?;
        let mut athlete_count = 0;
        for member in rows.iter().filter(|m| m.left_at.is_none()) {
            if member.user_id == coach_id {
                continue;
            }
            athlete_count += 1;
            let status = RosterAthleteStatus::of_member(member);
            let stopped_at = (status == RosterAthleteStatus::NoCoachConsent)
                .then(|| revoked_at(member))
                .flatten();
            if let Some(&at) = index.get(&member.user_id) {
                // Shared through any one group is shared: the consent is per
                // membership, and each grants the same coach the same read.
                let entry = &mut members[at];
                entry.groups.push(group_ref.clone());
                if entry.member_display_name.is_none() {
                    entry.member_display_name.clone_from(&member.display_name);
                }
                if status == RosterAthleteStatus::Shared
                    && entry.status != RosterAthleteStatus::Shared
                {
                    entry.status = status;
                    entry.tenant = tenant;
                    entry.shared_via = Some(group.id);
                }
                // Only a revoke in every group is a stop; keep the latest
                // known instant, and none once any group shares.
                entry.sharing_stopped_at = if entry.status == RosterAthleteStatus::NoCoachConsent {
                    entry.sharing_stopped_at.max(stopped_at)
                } else {
                    None
                };
                entry.assistant_read |= includes_assistant(member);
                if member.joined_at < entry.joined.at {
                    entry.joined = RosterJoin {
                        at: member.joined_at,
                        group: group_ref.clone(),
                    };
                }
            } else {
                index.insert(member.user_id, members.len());
                members.push(RosterMember {
                    user_id: member.user_id,
                    tenant,
                    groups: vec![group_ref.clone()],
                    status,
                    shared_via: (status == RosterAthleteStatus::Shared).then_some(group.id),
                    member_display_name: member.display_name.clone(),
                    source_tenants: Vec::new(),
                    source_providers: Vec::new(),
                    sharing_stopped_at: stopped_at,
                    joined: RosterJoin {
                        at: member.joined_at,
                        group: group_ref.clone(),
                    },
                    assistant_read: includes_assistant(member),
                });
            }
        }
        summaries.push(RosterGroupSummary {
            id: group.id.to_string(),
            name: group.name.clone(),
            athlete_count,
        });
    }

    Ok(GatheredRoster {
        groups: summaries,
        members,
    })
}

/// Whether one membership lets the coach's AI assistant include the member in
/// the roster tools: they share with the coach there and allowed it there.
const fn includes_assistant(member: &GroupMember) -> bool {
    member.coach_sharing_consent && member.coach_consents.assistant_read
}

/// When the member stopped sharing with the coach: the instant of their last
/// coach-sharing decision, but only when it came after they joined.
///
/// A join writes that instant together with `joined_at`, whatever was
/// decided, so a member who joined without sharing carries their join instant
/// there. That is when they declined, not when they stopped; reporting it as a
/// stop told the coach "sharing cut on <join day>" for someone who never
/// shared.
fn revoked_at(member: &GroupMember) -> Option<Timestamp> {
    member
        .coach_sharing_consent_at
        .filter(|at| *at > member.joined_at)
}

/// Mark each sharer with no connected source `no_source`, and record where
/// the others' sources live.
///
/// A sharer with no connected source has nothing to read, and saying so
/// beats an empty card. A connection lives under the athlete's own tenant,
/// which may not be the group's.
///
/// # Errors
///
/// Returns the repository error from a connection lookup, or a database error
/// for a connection row whose tenant does not parse.
async fn mark_sources(repos: &RepositoryRegistry, members: &mut [RosterMember]) -> AppResult<()> {
    let sharers: Vec<usize> = members
        .iter()
        .enumerate()
        .filter(|(_, m)| m.status == RosterAthleteStatus::Shared)
        .map(|(at, _)| at)
        .collect();
    let ids: Vec<Uuid> = sharers.iter().map(|&at| members[at].user_id).collect();
    let sources: Vec<(Vec<TenantId>, Vec<String>)> = stream::iter(ids)
        .map(|user_id| async move {
            let connections = repos
                .provider_connections
                .get_for_user(user_id, None)
                .await?;
            let mut tenants: Vec<TenantId> = connections
                .iter()
                .map(|c| stored_tenant("provider_connections", &c.tenant_id))
                .collect::<AppResult<_>>()?;
            tenants.sort_unstable_by_key(TenantId::as_uuid);
            tenants.dedup();
            let mut providers: Vec<String> = connections.into_iter().map(|c| c.provider).collect();
            providers.sort_unstable();
            providers.dedup();
            AppResult::Ok((tenants, providers))
        })
        .buffered(ROSTER_LOOKUP_CONCURRENCY)
        .try_collect()
        .await?;
    let found = sharers.into_iter().zip(sources);
    for (at, (tenants, providers)) in found {
        let member = &mut members[at];
        if tenants.is_empty() {
            member.status = RosterAthleteStatus::NoSource;
        } else {
            member.source_tenants = tenants;
            member.source_providers = providers;
        }
    }
    Ok(())
}

/// An athlete in roster order, with the name the order and the row use.
#[derive(Debug, Clone)]
pub struct OrderedMember {
    /// The athlete.
    pub member: RosterMember,
    /// Display name — never their account identifier.
    pub name: String,
    /// The day the activity cache last saw a session from them; only for a
    /// readable sharer, and only a session the reader may be shown.
    pub last_cached_activity: Option<Timestamp>,
    /// They share and have connected services, and every one of those keeps
    /// its data in Dravr: served to an outside client, nothing of their
    /// training reaches it. Listed as present with their data withheld, never
    /// as an athlete who stopped training.
    pub kept_in_dravr: bool,
}

impl OrderedMember {
    /// Whether their training is read for this roster: they share, and some
    /// of it may reach the reader.
    #[must_use]
    pub fn readable(&self) -> bool {
        self.member.status == RosterAthleteStatus::Shared && !self.kept_in_dravr
    }
}

/// Put the roster in order (rule 4) from the database alone, so a page can be
/// cut before any snapshot is fetched.
///
/// `outside` is the provider terms when the roster is served to a client
/// outside Dravr's own surfaces, `None` in Dravr's own app. Outside, a session
/// whose terms keep it in Dravr never sets an athlete's place, which would
/// tell the client who trained when no value does (carnet#1511), and a sharer
/// whose every connected service keeps its data in Dravr is
/// [`OrderedMember::kept_in_dravr`], placed by name after the readable
/// sharers and never read from the cache at all.
///
/// # Errors
///
/// Returns the repository error from the activity-cache lookup.
pub async fn order_roster(
    repos: &RepositoryRegistry,
    members: Vec<RosterMember>,
    outside: Option<&dyn ProviderTerms>,
) -> AppResult<Vec<OrderedMember>> {
    let mut ordered: Vec<OrderedMember> = stream::iter(members)
        .map(|member| async move {
            let name = fetch_user_display_name(repos, member.user_id).await;
            let kept_in_dravr = outside.is_some_and(|terms| kept_in_dravr(terms, &member));
            let last_cached_activity = if kept_in_dravr {
                None
            } else {
                last_consented_activity(repos, &member, outside).await?
            };
            AppResult::Ok(OrderedMember {
                member,
                name,
                last_cached_activity,
                kept_in_dravr,
            })
        })
        .buffered(ROSTER_LOOKUP_CONCURRENCY)
        .try_collect()
        .await?;
    ordered.sort_by(roster_order);
    Ok(ordered)
}

/// Whether `member` shares, has connected services, and every one of them
/// keeps its data in Dravr by `terms`.
fn kept_in_dravr(terms: &dyn ProviderTerms, member: &RosterMember) -> bool {
    member.status == RosterAthleteStatus::Shared
        && !member.source_providers.is_empty()
        && member
            .source_providers
            .iter()
            .all(|provider| first_party_only(terms, provider, None))
}

/// Cached sessions read per page while looking past withheld ones.
const WITHHELD_SCAN_PAGE: i64 = 100;

/// Pages read before an athlete whose recent sessions are all withheld is
/// ordered as having none: what the order shows must never be a withheld
/// session's day, so giving up reads as "no session", never as a guess.
const WITHHELD_SCAN_MAX_PAGES: usize = 20;

/// When the athlete last trained, as far as the cache knows, by the sources
/// whose AI use they have not withheld and, served to an outside client
/// (`outside`), by the sessions whose terms let them reach it.
///
/// The order is read by the agent and the coach alike, so a source the athlete
/// withheld from AI must not place them either: its session's day would show
/// through the order while their row carries none (carnet#1262). A relay
/// caches a withheld provider's session under its own key with that provider
/// as its `source`, so an athlete who withheld any provider is ordered by
/// their sessions themselves, each judged by provider and source.
async fn last_consented_activity(
    repos: &RepositoryRegistry,
    member: &RosterMember,
    outside: Option<&dyn ProviderTerms>,
) -> AppResult<Option<Timestamp>> {
    if member.source_tenants.is_empty() {
        return Ok(None);
    }
    // Providers only: a consent class names no provider's sessions.
    let mut withheld = ai_consent_withheld(repos, member.tenant.as_uuid(), member.user_id).await;
    withheld.retain(|key| !is_consent_class(key));
    let mut latest = None;
    for tenant in &member.source_tenants {
        let start = if withheld.is_empty() && outside.is_none() {
            repos
                .activity_cache
                .latest_activity_start(member.user_id, tenant)
                .await?
        } else {
            let hidden = Hidden {
                withheld: &withheld,
                outside,
            };
            latest_unwithheld_start(repos, member.user_id, tenant, &hidden).await?
        };
        latest = latest.max(start);
    }
    Ok(latest)
}

/// What a reader of the roster's order may not be shown of a session.
struct Hidden<'reader> {
    /// The providers whose AI use the athlete withheld.
    withheld: &'reader BTreeSet<String>,
    /// The provider terms, when the reader is a client outside Dravr.
    outside: Option<&'reader dyn ProviderTerms>,
}

impl Hidden<'_> {
    /// Whether `activity` is hidden from the reader: it comes from or was
    /// relayed from a withheld provider, or its terms keep it in Dravr and
    /// the reader is outside.
    fn hides(&self, activity: &Activity) -> bool {
        let is_withheld = |name: &str| self.withheld.contains(&name.to_ascii_lowercase());
        is_withheld(activity.provider())
            || activity.source().is_some_and(is_withheld)
            || self.outside.is_some_and(|terms| {
                first_party_only(terms, activity.provider(), activity.source())
            })
    }
}

/// The start of the user's newest cached session in `tenant` that `hidden`
/// does not hide.
async fn latest_unwithheld_start(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: &TenantId,
    hidden: &Hidden<'_>,
) -> AppResult<Option<Timestamp>> {
    let mut end = Timestamp::now();
    let mut previous_oldest = None;
    for _ in 0..WITHHELD_SCAN_MAX_PAGES {
        let page = repos
            .activity_cache
            .get_cached_activities(
                user_id,
                tenant,
                None,
                Timestamp::UNIX_EPOCH,
                end,
                WITHHELD_SCAN_PAGE,
            )
            .await?;
        let Some(oldest) = page.last().map(Activity::start_date) else {
            return Ok(None);
        };
        if let Some(kept) = page.iter().find(|a| !hidden.hides(a)) {
            return Ok(Some(kept.start_date()));
        }
        if i64::try_from(page.len()).unwrap_or(i64::MAX) < WITHHELD_SCAN_PAGE {
            return Ok(None);
        }
        // The window is inclusive and ends on the oldest instant read, so a
        // session sharing that instant beyond the page is read next, not
        // skipped; the withheld ones re-read with it are passed over again. A
        // page holding that one instant alone made no progress: step past it.
        end = if previous_oldest == Some(oldest) {
            oldest - Elapsed::microseconds(1)
        } else {
            oldest
        };
        previous_oldest = Some(oldest);
    }
    Ok(None)
}

/// Readable sharers first, the longest without a session (none at all
/// first); then the sharers whose data stays in Dravr; then everyone else; by
/// name within each.
fn roster_order(a: &OrderedMember, b: &OrderedMember) -> Ordering {
    let rank = |m: &OrderedMember| {
        if m.readable() {
            0_u8
        } else if m.member.status == RosterAthleteStatus::Shared {
            1
        } else {
            2
        }
    };
    rank(a)
        .cmp(&rank(b))
        .then_with(|| match (a.last_cached_activity, b.last_cached_activity) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Less,
            (Some(_), None) => Ordering::Greater,
            (Some(x), Some(y)) => x.cmp(&y),
        })
        .then_with(|| a.name.cmp(&b.name))
        .then_with(|| a.member.user_id.cmp(&b.member.user_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ordered(name: &str, status: RosterAthleteStatus, last: Option<i64>) -> OrderedMember {
        ordered_kept(name, status, last, false)
    }

    fn ordered_kept(
        name: &str,
        status: RosterAthleteStatus,
        last: Option<i64>,
        kept_in_dravr: bool,
    ) -> OrderedMember {
        OrderedMember {
            member: RosterMember {
                user_id: Uuid::new_v4(),
                tenant: TenantId::from_uuid(Uuid::nil()),
                groups: Vec::new(),
                status,
                shared_via: None,
                member_display_name: None,
                source_tenants: Vec::new(),
                source_providers: Vec::new(),
                sharing_stopped_at: None,
                joined: RosterJoin {
                    at: Timestamp::UNIX_EPOCH,
                    group: RosterGroupRef {
                        id: String::new(),
                        name: String::new(),
                    },
                },
                assistant_read: false,
            },
            name: name.to_owned(),
            last_cached_activity: last.map(|secs| Timestamp::from_epoch_seconds(secs).unwrap()),
            kept_in_dravr,
        }
    }

    #[test]
    fn a_sharer_whose_data_stays_in_dravr_follows_the_readable_sharers_by_name() {
        let mut roster = [
            ordered("Zoé", RosterAthleteStatus::NoCoachConsent, None),
            ordered_kept("Kept B", RosterAthleteStatus::Shared, None, true),
            ordered("Recent", RosterAthleteStatus::Shared, Some(2_000)),
            ordered_kept("Kept A", RosterAthleteStatus::Shared, None, true),
            ordered("Never", RosterAthleteStatus::Shared, None),
        ];
        roster.sort_by(roster_order);
        let names: Vec<&str> = roster.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            names,
            ["Never", "Recent", "Kept A", "Kept B", "Zoé"],
            "an athlete with nothing an outside client may see is never ordered as gone quiet"
        );
    }

    #[test]
    fn readable_sharers_lead_longest_quiet_first_then_the_rest_by_name() {
        let mut roster = [
            ordered("Zoé", RosterAthleteStatus::NoCoachConsent, None),
            ordered("Recent", RosterAthleteStatus::Shared, Some(2_000)),
            ordered("Alice", RosterAthleteStatus::NoSource, None),
            ordered("Quiet", RosterAthleteStatus::Shared, Some(1_000)),
            ordered("Never", RosterAthleteStatus::Shared, None),
        ];
        roster.sort_by(roster_order);
        let names: Vec<&str> = roster.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            names,
            ["Never", "Quiet", "Recent", "Alice", "Zoé"],
            "sharers lead, never-active first then the quietest; the rest follow by name"
        );
    }

    #[test]
    fn a_decline_at_join_is_no_stop_and_a_later_revoke_is() {
        let member = |consent_at: &str| {
            serde_json::from_value::<GroupMember>(serde_json::json!({
                "id": Uuid::nil(),
                "group_id": Uuid::nil(),
                "user_id": Uuid::nil(),
                "tenant_id": "t",
                "role": "member",
                "peer_sharing_consent": false,
                "coach_sharing_consent": false,
                "coach_sharing_consent_at": consent_at,
                "consent_given_at": "2026-10-05T00:00:00Z",
                "joined_at": "2026-10-05T00:00:00Z",
                "left_at": null
            }))
            .expect("member")
        };
        assert_eq!(
            revoked_at(&member("2026-10-05T00:00:00Z")),
            None,
            "declined when joining: never shared"
        );
        assert_eq!(
            revoked_at(&member("2026-10-27T21:48:00Z")),
            Timestamp::parse_from_rfc3339("2026-10-27T21:48:00Z").ok(),
            "a decision after the join is a stop"
        );
    }

    #[test]
    fn a_member_is_shared_until_they_revoke_the_coach_read() {
        let json = |coach_sharing_consent: bool| {
            serde_json::from_value::<GroupMember>(serde_json::json!({
                "id": Uuid::nil(),
                "group_id": Uuid::nil(),
                "user_id": Uuid::nil(),
                "tenant_id": "t",
                "role": "member",
                "peer_sharing_consent": false,
                "coach_sharing_consent": coach_sharing_consent,
                "consent_given_at": "2026-10-05T00:00:00Z",
                "joined_at": "2026-10-05T00:00:00Z",
                "left_at": null
            }))
            .expect("member")
        };
        // Peer consent off, coach consent on: the coach still reads.
        assert_eq!(
            RosterAthleteStatus::of_member(&json(true)),
            RosterAthleteStatus::Shared,
            "coach consent alone shares with the coach"
        );
        assert_eq!(
            RosterAthleteStatus::of_member(&json(false)),
            RosterAthleteStatus::NoCoachConsent,
            "a revoked coach consent withholds the read"
        );
    }
}
