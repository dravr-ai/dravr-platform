// ABOUTME: get_provider_roster — the caller's coaching-platform roster, each athlete where they stand with the caller's groups
// ABOUTME: Names Dravr members only, by display name; every other athlete is a count by state and access
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Provider roster
//!
//! A human coach's coaching platform (Intervals.icu, `TrainingPeaks`) lists
//! the athletes who share with the coach's account there; the coach's Dravr
//! groups list members. [`GetProviderRosterTool`] tells, from the coach's own
//! conversation, where each roster athlete stands
//! ([`pierre_services::delegated_link_states`]): linked, asked to confirm,
//! ready to be proposed a link ([`super::athlete_link`]), or why not.
//!
//! Four rules decide what comes back:
//!
//! 1. **Members by name, everyone else by count.** Roster names, emails and
//!    ids of athletes who have not confirmed a link never reach a model. A
//!    Dravr member the roster names is named by their Dravr display name
//!    ([`pierre_services::athlete_display_name`]), never by the name the platform
//!    lists; every other roster athlete is a count by state and access.
//! 2. **Terms before reads.** Each platform's roster is checked against the
//!    call's transport before it is read, from the cache too
//!    ([`check_roster_transport`]): a platform whose terms keep its data
//!    first-party is refused whole over an external transport, and stamps the
//!    turn on Dravr's own surfaces.
//! 3. **The coach's own conversation.** A group's room is read by every
//!    member, so the roster is refused there, and in a conversation that does
//!    not resolve as the caller's own.
//! 4. **Standing, not just the roster.** A coach with no group, or without
//!    coach access, still reads their roster, with what it takes to link
//!    anyone on it.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::info;
use uuid::Uuid;

use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{CoachAccessStatus, RosterAccess, TenantId};
use pierre_mcp_schema::{PropertySchema, ToolAnnotations};
use pierre_runtime_context::DataContext;
use pierre_services::coach_platform::{check_roster_transport, roster_platforms, CoachPlatform};
use pierre_services::delegated_link_states::{roster_link_states, LinkState, RosterLinkState};
use pierre_tools_core::ToolResult;

use super::coach_scope::{refusal, room_refusal};
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, tool_definition, tool_result_to_response,
};
use crate::protocol::coach_calendar::delegation_services;
use crate::runtime::ToolRuntime;
use pierre_services::athlete_display_name::fetch_user_display_name;

/// The caller's access to a roster athlete, as their platform reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRosterAccess {
    /// The platform lets the caller coach the athlete.
    Coach,
    /// The platform lets the caller follow the athlete: their data reads,
    /// nothing is written.
    Follower,
    /// The platform lists the athlete but gives the caller no access to them.
    NoAccess,
    /// The platform did not say: read as neither coach nor follower.
    Unknown,
}

impl ProviderRosterAccess {
    /// The access a roster entry carries; a platform that said nothing reads
    /// as unknown.
    const fn of(access: Option<RosterAccess>) -> Self {
        match access {
            Some(RosterAccess::Coach) => Self::Coach,
            Some(RosterAccess::Follower) => Self::Follower,
            Some(RosterAccess::NoAccess) => Self::NoAccess,
            None => Self::Unknown,
        }
    }
}

/// Where a roster athlete stands with the caller's groups and links.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProviderRosterState {
    /// Linked: the member's data on the platform reads through the caller's
    /// account.
    Linked,
    /// Linked, but the link reads nothing: see `not_reading_because`.
    LinkedNotReading,
    /// A link was proposed; the member has not confirmed it in the app yet.
    Proposed,
    /// The member reads the platform through another human coach's link.
    LinkedThroughAnotherCoach,
    /// The member already holds a link of the caller's for another roster
    /// entry.
    MemberHoldsAnotherLink,
    /// The member can be proposed a link with `propose_athlete_link`.
    Proposable,
    /// The roster email is the member's, but they have not verified it on
    /// Dravr, so it binds no one yet.
    MemberEmailUnverified,
    /// The member connects the platform with an account of their own: no
    /// link is needed, the coach reaches their calendar through the coach's
    /// own account.
    MemberOwnConnection,
    /// The member stopped sharing with the caller in every group the caller
    /// coaches them in (`/group consent coach yes` restores it).
    MemberNotSharing,
    /// The platform gives the caller no access to the athlete, so no link is
    /// proposed.
    NoAccess,
    /// The platform shares no email for the athlete, so no member binds.
    RosterEmailMissing,
    /// The athlete's roster email is no member's of a group the caller
    /// coaches.
    NotInYourGroups,
}

impl ProviderRosterState {
    /// The state a roster athlete's link state reads as.
    const fn of(state: &LinkState) -> Self {
        match state {
            LinkState::Linked { .. } => Self::Linked,
            LinkState::LinkedNotReading { .. } => Self::LinkedNotReading,
            LinkState::Proposed { .. } => Self::Proposed,
            LinkState::LinkedThroughAnotherCoach { .. } => Self::LinkedThroughAnotherCoach,
            LinkState::MemberHoldsAnotherLink { .. } => Self::MemberHoldsAnotherLink,
            LinkState::Proposable { .. } => Self::Proposable,
            LinkState::MemberEmailUnverified { .. } => Self::MemberEmailUnverified,
            LinkState::MemberOwnConnection { .. } => Self::MemberOwnConnection,
            LinkState::MemberNotSharing { .. } => Self::MemberNotSharing,
            LinkState::NoAccess { .. } => Self::NoAccess,
            LinkState::RosterEmailMissing => Self::RosterEmailMissing,
            LinkState::NotInYourGroups => Self::NotInYourGroups,
        }
    }
}

/// A Dravr member the roster names, and where they stand.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ProviderRosterMember {
    /// The member's Dravr display name, never the name the platform lists.
    pub name: String,
    /// The group of the caller's the member is in, where a link to them is
    /// made.
    pub group: String,
    /// Where they stand.
    pub state: ProviderRosterState,
    /// The caller's access to them on the platform.
    pub access: ProviderRosterAccess,
    /// Why a `linked_not_reading` link reads nothing
    /// (`athlete_email_missing`, `athlete_email_mismatch`,
    /// `member_email_unverified`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_reading_because: Option<String>,
}

/// How many roster athletes, of those no Dravr member is named for, share
/// one state and access.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ProviderRosterCount {
    /// Their state: `not_in_your_groups` or `roster_email_missing`, or a
    /// link's state when the group the link was made in is gone.
    pub state: ProviderRosterState,
    /// The caller's access to them on the platform.
    pub access: ProviderRosterAccess,
    /// How many athletes.
    pub athletes: usize,
}

/// Why a platform's roster was not read, in the refusal shape every tool
/// answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ProviderRosterRefusal {
    /// The reason a caller branches on.
    pub reason: String,
    /// What to tell the user.
    pub error: String,
}

/// One coaching platform's roster.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ProviderRosterPlatform {
    /// The platform, as the `provider` argument names it.
    pub provider: String,
    /// Why the roster was not read, when it was not; nothing else is listed
    /// then.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refusal: Option<ProviderRosterRefusal>,
    /// Athletes the roster lists, the caller's own account not among them;
    /// absent when it was not read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roster_size: Option<usize>,
    /// The Dravr members the roster names, by name.
    pub members: Vec<ProviderRosterMember>,
    /// Every other roster athlete, as counts by state and access.
    pub others: Vec<ProviderRosterCount>,
}

/// What `get_provider_roster` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ProviderRosterResult {
    /// Each coaching platform read: the one named, else every one the caller
    /// connected.
    pub platforms: Vec<ProviderRosterPlatform>,
    /// Whether the caller holds coach access: what coaching a Dravr group
    /// takes.
    pub coach_access: bool,
    /// Whether a request of the caller's for coach access waits for the
    /// Dravr team to decide: asked in the app, or filed when their
    /// Intervals.icu account coaches active athletes. Never true for a caller
    /// who holds coach access.
    pub coach_access_request_pending: bool,
    /// Active Dravr groups the caller is the human coach of.
    pub groups_coached: usize,
    /// What it takes, from the caller's standing, to link the athletes not
    /// linked yet.
    pub next_steps: Vec<String>,
}

/// The refusal a coaching-platform step answered with, when it names one: a
/// linking step's reason (`details.reason`), or a read the platform's terms
/// keep off this transport. `None` for any other error.
pub(super) fn platform_refusal(error: &AppError) -> Option<ProviderRosterRefusal> {
    if error.code == ErrorCode::UnavailableOverTransport {
        return Some(ProviderRosterRefusal {
            reason: "unavailable_over_transport".to_owned(),
            error: "This platform's terms keep its roster to Dravr's own app, so it is not read \
                    over this interface."
                .to_owned(),
        });
    }
    let reason = error.details.as_deref()?.get("reason")?.as_str()?;
    Some(ProviderRosterRefusal {
        reason: reason.to_owned(),
        error: error.message.clone(),
    })
}

/// The caller's standing as a coach: what linking a roster athlete takes.
struct Standing {
    coach_access: bool,
    access_requested: bool,
    groups_coached: usize,
    can_invite: bool,
}

/// Read `coach`'s standing: coach access as coaching a group takes it
/// (`manages_roster`, or platform admin), whether a request for it waits
/// when they do not hold it, the active groups they coach, and whether they
/// may issue the invite of any of them (an owner or admin membership: a
/// coach attached by a coach invite holds none).
async fn standing(data: &DataContext, coach: Uuid) -> AppResult<Standing> {
    let repos = data.repos();
    let coach_access = repos
        .users
        .get_global(coach)
        .await?
        .is_some_and(|user| user.manages_roster || user.is_admin);
    let access_requested = !coach_access
        && repos
            .coach_access_requests
            .latest_for_user(coach)
            .await?
            .is_some_and(|request| request.status == CoachAccessStatus::Pending);
    let mut groups_coached = 0;
    let mut can_invite = false;
    for group in repos.groups.list_groups_coached_by(coach).await? {
        if !group.is_active {
            continue;
        }
        groups_coached += 1;
        if !can_invite {
            can_invite = repos
                .groups
                .get_member(&group.id.to_string(), coach)
                .await?
                .is_some_and(|member| member.role.can_manage_members());
        }
    }
    Ok(Standing {
        coach_access,
        access_requested,
        groups_coached,
        can_invite,
    })
}

/// What it takes, from `standing`, to link a roster athlete not linked yet.
fn next_steps(standing: &Standing) -> Vec<String> {
    let mut steps = Vec::new();
    if standing.access_requested {
        steps.push(
            "Coaching a Dravr group takes coach access, which you do not hold yet. Your request \
             for it is waiting for the Dravr team to decide: there is nothing more to ask."
                .to_owned(),
        );
    } else if !standing.coach_access {
        steps.push(
            "Coaching a Dravr group takes coach access, which you do not hold yet: it comes with \
             a TrainingPeaks coach account, or the Dravr team turns it on when you ask for it in \
             the app."
                .to_owned(),
        );
    }
    let joined = "Once an athlete joins with the email your platform lists for them and verifies \
                  it, `propose_athlete_link` asks them to confirm the link in the app.";
    if standing.groups_coached == 0 {
        steps.push(format!(
            "You coach no Dravr group yet: create one you coach in the app, then share its \
             `/group invite` link with your athletes. {joined}"
        ));
    } else if standing.can_invite {
        steps.push(format!(
            "An athlete who is not in your groups joins one through its `/group invite` link, \
             which you issue in the group's room. {joined}"
        ));
    } else {
        steps.push(format!(
            "You coach your groups through a coach invite, so you cannot issue their \
             `/group invite` link yourself: ask a group's owner to share it with an athlete who \
             is not in your groups. {joined}"
        ));
    }
    steps
}

/// A platform's roster as refused.
fn refused(platform: &dyn CoachPlatform, refusal: ProviderRosterRefusal) -> ProviderRosterPlatform {
    ProviderRosterPlatform {
        provider: platform.user_facing().to_owned(),
        refusal: Some(refusal),
        roster_size: None,
        members: Vec::new(),
        others: Vec::new(),
    }
}

/// The Dravr member `state` names and the group they are named in: a link's
/// member in the link's group, else the member the roster email binds in the
/// group a link to them would be made in. `None` for an athlete no member of
/// the caller's stands behind, and for a link whose group is gone.
///
/// # Errors
///
/// Returns the repository error from the group lookup.
async fn named_member(
    data: &DataContext,
    state: &LinkState,
    link_groups: &mut HashMap<Uuid, Option<String>>,
) -> AppResult<Option<(Uuid, String)>> {
    if let Some(member) = state.member() {
        return Ok(Some((member.user_id, member.group.name.clone())));
    }
    let Some(link) = state.link() else {
        return Ok(None);
    };
    if let Some(name) = link_groups.get(&link.group_id) {
        return Ok(name.clone().map(|group| (link.member_user_id, group)));
    }
    let name = data
        .repos()
        .groups
        .get_group(&link.group_id.to_string(), link.coach_tenant_id)
        .await?
        .map(|group| group.name);
    link_groups.insert(link.group_id, name.clone());
    Ok(name.map(|group| (link.member_user_id, group)))
}

/// `states`, one platform's roster, as the tool lists it: members by display
/// name, everyone else as counts.
///
/// # Errors
///
/// Returns the repository error from a group lookup.
async fn project(
    data: &DataContext,
    platform: &dyn CoachPlatform,
    states: Vec<RosterLinkState>,
) -> AppResult<ProviderRosterPlatform> {
    let roster_size = states.len();
    let mut names: HashMap<Uuid, String> = HashMap::new();
    let mut link_groups: HashMap<Uuid, Option<String>> = HashMap::new();
    let mut members = Vec::new();
    let mut others: BTreeMap<(ProviderRosterState, ProviderRosterAccess), usize> = BTreeMap::new();
    for RosterLinkState { athlete, state } in states {
        let access = ProviderRosterAccess::of(athlete.access);
        let kind = ProviderRosterState::of(&state);
        let Some((member_id, group)) = named_member(data, &state, &mut link_groups).await? else {
            *others.entry((kind, access)).or_default() += 1;
            continue;
        };
        let name = if let Some(name) = names.get(&member_id) {
            name.clone()
        } else {
            let name = fetch_user_display_name(data.repos(), member_id).await;
            names.insert(member_id, name.clone());
            name
        };
        let not_reading_because = match state {
            LinkState::LinkedNotReading { reason, .. } => Some(reason.to_owned()),
            _ => None,
        };
        members.push(ProviderRosterMember {
            name,
            group,
            state: kind,
            access,
            not_reading_because,
        });
    }
    members.sort_by(|a, b| (&a.name, &a.group).cmp(&(&b.name, &b.group)));
    Ok(ProviderRosterPlatform {
        provider: platform.user_facing().to_owned(),
        refusal: None,
        roster_size: Some(roster_size),
        members,
        others: others
            .into_iter()
            .map(|((state, access), athletes)| ProviderRosterCount {
                state,
                access,
                athletes,
            })
            .collect(),
    })
}

/// `platform`'s roster for the caller, each athlete where they stand, or the
/// refusal that kept it from being read: the one read both roster tools make.
///
/// The transport check runs first, before any roster read, the cache's
/// included.
///
/// # Errors
///
/// Returns a repository or provider error that is no linking refusal.
pub(super) async fn roster_states(
    context: &ToolExecutionContext,
    platform: &dyn CoachPlatform,
    tenant: TenantId,
) -> AppResult<Result<Vec<RosterLinkState>, ProviderRosterRefusal>> {
    let services = delegation_services(context.resources.as_ref());
    let read = match check_roster_transport(services.registry, platform) {
        Ok(()) => roster_link_states(services, platform, context.user_id, tenant).await,
        Err(e) => Err(e),
    };
    match read {
        Ok(states) => Ok(Ok(states)),
        Err(e) => platform_refusal(&e).map(Err).ok_or(e),
    }
}

/// One platform's roster for the caller, or why it was not read.
///
/// # Errors
///
/// Returns a repository or provider error that is no linking refusal.
async fn platform_roster(
    context: &ToolExecutionContext,
    platform: &dyn CoachPlatform,
    tenant: TenantId,
) -> AppResult<ProviderRosterPlatform> {
    match roster_states(context, platform, tenant).await? {
        Ok(states) => project(&context.resources.data(), platform, states).await,
        Err(why) => Ok(refused(platform, why)),
    }
}

/// Read the caller's coaching-platform roster, each athlete where they stand.
pub struct GetProviderRosterTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetProviderRosterTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Optional coaching platform to read (`intervals_icu`, `trainingpeaks`). Omit \
                     to read every one you connected."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, None);

        answers_with::<ProviderRosterResult>(tool_definition(
            "get_provider_roster",
            "Read the roster of the coaching platform you coach on (Intervals.icu, TrainingPeaks) \
             and where each athlete on it stands with your Dravr groups. Dravr members the roster \
             names are listed by their Dravr name with a `state`: `linked`, `proposed` (waiting \
             for them to confirm in the app), `proposable` (link them with \
             `propose_athlete_link`), `member_email_unverified` (they verify their Dravr email \
             first), `member_not_sharing` (they stopped sharing with you), \
             `member_own_connection` (they connect the platform themselves: no link is needed, \
             you reach their calendar through your own account), \
             `linked_through_another_coach`, \
             `member_holds_another_link`, `no_access` or `linked_not_reading`. Every other \
             athlete is only counted, by state (`not_in_your_groups`, `roster_email_missing`) \
             and by your access on the platform (`coach`, `follower`, `no_access`, `unknown`); \
             their names are never shown. `next_steps` says what linking them takes. Read it \
             from your own conversation, never a group's room.",
            schema,
            Some(ToolAnnotations {
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
                ..ToolAnnotations::default()
            }),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async {
            let tenant = TenantId::from_uuid(context.require_tenant()?);
            if let Some(refused) = room_refusal(&context, tenant, "Read your roster").await? {
                return Ok(refused);
            }
            let requested = args
                .get("provider")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let repos = context.resources.repos();
            let platforms = match roster_platforms(
                repos,
                context.resources.provider_registry(),
                context.user_id,
                tenant,
                requested,
            )
            .await
            {
                Ok(platforms) => platforms,
                Err(e) => {
                    return match platform_refusal(&e) {
                        Some(r) => Ok(refusal(&r.reason, &r.error)),
                        None => Err(e),
                    };
                }
            };

            let mut rosters = Vec::with_capacity(platforms.len());
            for platform in &platforms {
                rosters.push(platform_roster(&context, platform.as_ref(), tenant).await?);
            }
            // One platform, refused: the refusal is the answer.
            if let [only] = rosters.as_slice() {
                if let Some(r) = &only.refusal {
                    return Ok(ToolResult::error(json!({
                        "reason": r.reason,
                        "error": r.error,
                        "provider": only.provider,
                    })));
                }
            }

            let standing = standing(&context.resources.data(), context.user_id).await?;
            info!(
                requester = %context.user_id,
                platforms = rosters.len(),
                read = rosters.iter().filter(|r| r.refusal.is_none()).count(),
                groups_coached = standing.groups_coached,
                "get_provider_roster: coaching-platform roster read"
            );
            ok_typed(
                "get_provider_roster",
                ProviderRosterResult {
                    next_steps: next_steps(&standing),
                    platforms: rosters,
                    coach_access: standing.coach_access,
                    coach_access_request_pending: standing.access_requested,
                    groups_coached: standing.groups_coached,
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// Members' display names and group names are third-party text.
crate::declare_security!(GetProviderRosterTool => UNTRUSTED_OUTPUT);

#[cfg(test)]
mod tests {
    use super::*;

    fn standing(coach_access: bool, groups_coached: usize, can_invite: bool) -> Standing {
        Standing {
            coach_access,
            access_requested: false,
            groups_coached,
            can_invite,
        }
    }

    #[test]
    fn a_coach_whose_request_waits_is_told_so_and_not_to_ask() {
        let waiting = Standing {
            access_requested: true,
            ..standing(false, 0, false)
        };
        let steps = next_steps(&waiting);
        assert_eq!(steps.len(), 2, "{steps:?}");
        assert!(steps[0].contains("waiting for the Dravr team"), "{steps:?}");
        assert!(!steps[0].contains("when you ask for it"), "{steps:?}");
        assert!(steps[1].contains("create one you coach"), "{steps:?}");
    }

    #[test]
    fn a_coach_without_coach_access_is_told_how_it_is_granted() {
        let steps = next_steps(&standing(false, 0, false));
        assert_eq!(steps.len(), 2, "{steps:?}");
        assert!(steps[0].contains("coach access"), "{steps:?}");
        assert!(steps[1].contains("create one you coach"), "{steps:?}");
        assert!(
            next_steps(&standing(true, 0, false))
                .iter()
                .all(|step| !step.contains("coach access")),
            "a coach holding it is not told to ask"
        );
    }

    #[test]
    fn only_a_coach_who_may_issue_an_invite_is_told_to_issue_one() {
        let owner = next_steps(&standing(true, 2, true));
        assert_eq!(owner.len(), 1);
        assert!(owner[0].contains("which you issue"), "{owner:?}");
        let attached = next_steps(&standing(true, 2, false));
        assert!(attached[0].contains("ask a group's owner"), "{attached:?}");
    }

    #[test]
    fn a_platform_that_said_nothing_reads_as_unknown_access() {
        assert_eq!(
            ProviderRosterAccess::of(None),
            ProviderRosterAccess::Unknown
        );
        assert_eq!(
            ProviderRosterAccess::of(Some(RosterAccess::Follower)),
            ProviderRosterAccess::Follower
        );
    }

    #[test]
    fn a_linking_refusal_keeps_its_reason_and_any_other_error_is_none() {
        let mut refused = AppError::invalid_input("Reconnect Intervals.icu to read your roster");
        refused.details = Some(Box::new(json!({
            "reason": "coach_platform_reconnect_needed",
            "provider": "intervals_icu",
        })));
        let mapped = platform_refusal(&refused).expect("a reason is a refusal");
        assert_eq!(mapped.reason, "coach_platform_reconnect_needed");
        assert_eq!(mapped.error, "Reconnect Intervals.icu to read your roster");

        let over_transport =
            platform_refusal(&AppError::unavailable_over_transport("not here")).unwrap();
        assert_eq!(over_transport.reason, "unavailable_over_transport");

        assert!(platform_refusal(&AppError::internal("database down")).is_none());
    }
}
