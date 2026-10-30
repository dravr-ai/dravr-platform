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
//! 1. **Who may read it.** A user with `manages_roster` or platform admin —
//!    the rule that lets them be attached as a group's coach at all
//!    (`pierre_groups::creation_policy::may_coach_group`) — who is the human
//!    coach of at least one active group. A coach whose permission was
//!    withdrawn after attaching reads nothing.
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
//!    listed, by name.

use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Duration, Utc};
use futures_util::stream::{self, StreamExt, TryStreamExt};
use serde::Serialize;
use uuid::Uuid;

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

/// Whether the caller may read a roster, and the groups it covers.
#[derive(Debug)]
pub enum RosterAccess {
    /// The active groups the caller coaches, never empty.
    Granted(Vec<CoachingGroup>),
    /// The caller holds no coaching permission, or coaches no active group.
    NotACoach,
}

/// Who may read a roster, and over which groups (rule 1).
///
/// # Errors
///
/// Returns the repository error from the user or group lookup.
pub async fn roster_access(repos: &RepositoryRegistry, coach_id: Uuid) -> AppResult<RosterAccess> {
    let may_coach = repos
        .users
        .get_global(coach_id)
        .await?
        .is_some_and(|user| user.manages_roster || user.is_admin);
    if !may_coach {
        return Ok(RosterAccess::NotACoach);
    }
    let groups: Vec<CoachingGroup> = repos
        .groups
        .list_groups_coached_by(coach_id)
        .await?
        .into_iter()
        .filter(|g| g.is_active)
        .collect();
    Ok(if groups.is_empty() {
        RosterAccess::NotACoach
    } else {
        RosterAccess::Granted(groups)
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
    /// When they last stopped sharing with the coach: the latest known
    /// instant across the coach's groups holding them, only while the status
    /// is `NoCoachConsent` and only when one is recorded. `None` otherwise,
    /// including for a revoke from before the instant was recorded.
    pub sharing_stopped_at: Option<DateTime<Utc>>,
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
    /// Athletes whose training the coach may see (`shared` or `no_source`).
    #[must_use]
    pub fn visible_members(&self) -> usize {
        self.members
            .iter()
            .filter(|m| m.status != RosterAthleteStatus::NoCoachConsent)
            .count()
    }

    /// Athletes the roster holds.
    #[must_use]
    pub fn roster_members(&self) -> usize {
        self.members.len()
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
                .then_some(member.coach_sharing_consent_at)
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
                    sharing_stopped_at: stopped_at,
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
    let sources: Vec<Vec<TenantId>> = stream::iter(ids)
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
            AppResult::Ok(tenants)
        })
        .buffered(ROSTER_LOOKUP_CONCURRENCY)
        .try_collect()
        .await?;
    for (at, tenants) in sharers.into_iter().zip(sources) {
        let member = &mut members[at];
        if tenants.is_empty() {
            member.status = RosterAthleteStatus::NoSource;
        } else {
            member.source_tenants = tenants;
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
    /// readable sharer.
    pub last_cached_activity: Option<DateTime<Utc>>,
}

/// Put the roster in order (rule 4) from the database alone, so a page can be
/// cut before any snapshot is fetched.
///
/// # Errors
///
/// Returns the repository error from the activity-cache lookup.
pub async fn order_roster(
    repos: &RepositoryRegistry,
    members: Vec<RosterMember>,
) -> AppResult<Vec<OrderedMember>> {
    let mut ordered: Vec<OrderedMember> = stream::iter(members)
        .map(|member| async move {
            let name = fetch_user_display_name(repos, member.user_id).await;
            let last_cached_activity = last_consented_activity(repos, &member).await?;
            AppResult::Ok(OrderedMember {
                member,
                name,
                last_cached_activity,
            })
        })
        .buffered(ROSTER_LOOKUP_CONCURRENCY)
        .try_collect()
        .await?;
    ordered.sort_by(roster_order);
    Ok(ordered)
}

/// Cached sessions read per page while looking past withheld ones.
const WITHHELD_SCAN_PAGE: i64 = 100;

/// Pages read before an athlete whose recent sessions are all withheld is
/// ordered as having none: what the order shows must never be a withheld
/// session's day, so giving up reads as "no session", never as a guess.
const WITHHELD_SCAN_MAX_PAGES: usize = 20;

/// When the athlete last trained, as far as the cache knows, by the sources
/// whose AI use they have not withheld.
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
) -> AppResult<Option<DateTime<Utc>>> {
    if member.source_tenants.is_empty() {
        return Ok(None);
    }
    let withheld = ai_consent_withheld(repos, member.tenant.as_uuid(), member.user_id).await;
    let mut latest = None;
    for tenant in &member.source_tenants {
        let start = if withheld.is_empty() {
            repos
                .activity_cache
                .latest_activity_start(member.user_id, tenant)
                .await?
        } else {
            latest_unwithheld_start(repos, member.user_id, tenant, &withheld).await?
        };
        latest = latest.max(start);
    }
    Ok(latest)
}

/// The start of the user's newest cached session in `tenant` that neither
/// comes from nor was relayed from a provider in `withheld`.
async fn latest_unwithheld_start(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: &TenantId,
    withheld: &BTreeSet<String>,
) -> AppResult<Option<DateTime<Utc>>> {
    let is_withheld = |name: &str| withheld.contains(&name.to_ascii_lowercase());
    let mut end = Utc::now();
    let mut previous_oldest = None;
    for _ in 0..WITHHELD_SCAN_MAX_PAGES {
        let page = repos
            .activity_cache
            .get_cached_activities(
                user_id,
                tenant,
                None,
                DateTime::UNIX_EPOCH,
                end,
                WITHHELD_SCAN_PAGE,
            )
            .await?;
        let Some(oldest) = page.last().map(Activity::start_date) else {
            return Ok(None);
        };
        if let Some(kept) = page
            .iter()
            .find(|a| !is_withheld(a.provider()) && !a.source().is_some_and(is_withheld))
        {
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
            oldest - Duration::microseconds(1)
        } else {
            oldest
        };
        previous_oldest = Some(oldest);
    }
    Ok(None)
}

/// Readable sharers first, the longest without a session (none at all
/// first), then everyone else; by name within each.
fn roster_order(a: &OrderedMember, b: &OrderedMember) -> Ordering {
    let rank = |m: &OrderedMember| u8::from(m.member.status != RosterAthleteStatus::Shared);
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
        OrderedMember {
            member: RosterMember {
                user_id: Uuid::new_v4(),
                tenant: TenantId::from_uuid(Uuid::nil()),
                groups: Vec::new(),
                status,
                shared_via: None,
                member_display_name: None,
                source_tenants: Vec::new(),
                sharing_stopped_at: None,
            },
            name: name.to_owned(),
            last_cached_activity: last.map(|secs| DateTime::from_timestamp(secs, 0).unwrap()),
        }
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
