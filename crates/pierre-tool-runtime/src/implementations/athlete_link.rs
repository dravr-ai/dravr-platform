// ABOUTME: propose_athlete_link — a human coach asks a member of their group, from chat, to confirm a coaching-platform link
// ABOUTME: The member is found by name among the coach's groups, the roster athlete by the member's verified Dravr email
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Athlete link
//!
//! [`ProposeAthleteLinkTool`] is the chat path into the delegated link
//! ([`pierre_services::delegated_connections::propose`]): the coach names a
//! member of a group they coach, the roster athlete whose platform email is
//! the member's verified Dravr email is found, and the member is asked to
//! confirm in the app. Nothing is read until they do.
//!
//! The coach names the member, never the roster athlete: the member is
//! matched by display name among the live members of the groups the coach is
//! attached to ([`super::plan_scope::match_coached_member`]), and the roster
//! athlete is the one their email binds, as
//! [`pierre_services::delegated_link_states`] reads it. A member who stopped
//! sharing with the coach is not proposed.
//!
//! Each candidate platform's terms are checked against the call's transport
//! before its roster is read, the cache's included
//! ([`super::provider_roster::roster_states`]). With no `provider` named,
//! only the platforms that pass are considered, so a coach on a
//! first-party-only platform and another one is never refused for the first
//! when the second serves; when the member is on more than one, the coach is
//! asked which.
//!
//! Nothing in the answer comes from the roster beyond whether the member is
//! on it: no roster name, email or athlete id.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::Value;
use tracing::info;
use uuid::Uuid;

use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::errors::AppResult;
use pierre_core::models::groups::CoachingGroup;
use pierre_core::models::TenantId;
use pierre_mcp_schema::{PropertySchema, ToolAnnotations};
use pierre_services::coach_platform::{roster_platforms, CoachPlatform};
use pierre_services::delegated_connections::{propose, Proposal};
use pierre_services::delegated_link_states::{LinkState, RosterLinkState};
use pierre_services::delegation_refusal::Refusal;
use pierre_tools_core::ToolResult;

use super::coach_scope::{refusal, room_refusal};
use super::plan_scope::{match_coached_member, MemberMiss};
use super::provider_roster::{platform_refusal, roster_states, ProviderRosterRefusal};
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, tool_definition, tool_result_to_response,
};
use crate::protocol::coach_calendar::delegation_services;
use crate::runtime::ToolRuntime;

/// What `propose_athlete_link` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct ProposeAthleteLinkResult {
    /// The member asked to confirm, by their Dravr display name.
    pub member: String,
    /// The group the link is made in.
    pub group: String,
    /// The coaching platform the link reads through, as `provider` names it.
    pub provider: String,
    /// What happens next.
    pub next_step: String,
}

/// Where the member stands on one platform's roster.
enum Standing {
    /// The one roster athlete whose email binds the member, and the group a
    /// link to them is made in.
    Bindable {
        athlete_id: String,
        group: Box<CoachingGroup>,
    },
    /// The roster names the member, but no link can be proposed: why.
    Blocked(ProviderRosterRefusal),
    /// No roster athlete names the member.
    Absent,
}

/// A refusal in the shape a platform's roster carries one.
fn refused(reason: &str, error: String) -> ProviderRosterRefusal {
    ProviderRosterRefusal {
        reason: reason.to_owned(),
        error,
    }
}

/// Why a roster athlete in `state`, whose email names the member `name`,
/// cannot be proposed on `brand`; `None` for a proposable one and for a state
/// naming no member.
fn state_refusal(state: &LinkState, name: &str, brand: &str) -> Option<ProviderRosterRefusal> {
    Some(match state {
        LinkState::Linked { .. } => refused(
            Refusal::AthleteAlreadyLinked.as_str(),
            format!("{name} is already linked to you on {brand}."),
        ),
        LinkState::LinkedNotReading { reason, .. } => refused(
            Refusal::AthleteAlreadyLinked.as_str(),
            format!("{name} is already linked to you on {brand}, but the link reads nothing ({reason})."),
        ),
        LinkState::Proposed { .. } => refused(
            Refusal::AlreadyProposed.as_str(),
            format!(
                "{name} already has a {brand} link from you, waiting for them to confirm it in \
                 the app."
            ),
        ),
        LinkState::MemberHoldsAnotherLink { .. } => refused(
            Refusal::AlreadyProposed.as_str(),
            format!("{name} already holds a {brand} link from you for another entry on your roster."),
        ),
        LinkState::LinkedThroughAnotherCoach { .. } => refused(
            "linked_through_another_coach",
            format!(
                "{name} reads {brand} through another human coach's link, so they cannot confirm \
                 a second one."
            ),
        ),
        LinkState::MemberEmailUnverified { .. } => refused(
            Refusal::MemberEmailUnverified.as_str(),
            format!(
                "{name} has not verified their Dravr email yet, so your {brand} roster cannot be \
                 matched to them: ask them to verify it, then propose again."
            ),
        ),
        LinkState::MemberOwnConnection { .. } => refused(
            Refusal::OwnConnection.as_str(),
            format!(
                "{name} connects {brand} with an account of their own, so a link through yours \
                 is refused while they do."
            ),
        ),
        LinkState::MemberNotSharing { .. } => refused(
            "member_not_sharing",
            format!(
                "{name} has stopped sharing their data with you. They can share it again with \
                 `/group consent coach yes`."
            ),
        ),
        LinkState::NoAccess { .. } => refused(
            Refusal::AthleteNoAccess.as_str(),
            format!("Your {brand} account has no access to {name}, so a link would read nothing."),
        ),
        LinkState::Proposable { .. }
        | LinkState::RosterEmailMissing
        | LinkState::NotInYourGroups => return None,
    })
}

/// Where `member`, named `name`, stands on one platform's roster `states`.
///
/// A live link of the coach's holding the member decides first; then the one
/// roster athlete proposable to them; two such athletes share the member's
/// email and are refused, since a link cannot tell them apart.
fn standing_on(states: &[RosterLinkState], member: Uuid, name: &str, brand: &str) -> Standing {
    let mut held = None;
    let mut proposable = Vec::new();
    let mut blocked = None;
    for RosterLinkState { athlete, state } in states {
        match state {
            _ if state
                .link()
                .is_some_and(|link| link.member_user_id == member) =>
            {
                held = held.or(Some(state));
            }
            LinkState::Proposable { member: bound } if bound.user_id == member => {
                proposable.push((athlete.id.clone(), Box::new(bound.group.clone())));
            }
            _ if state.member().is_some_and(|m| m.user_id == member) => {
                blocked = blocked.or(Some(state));
            }
            _ => {}
        }
    }
    if let Some(refusal) = held.and_then(|state| state_refusal(state, name, brand)) {
        return Standing::Blocked(refusal);
    }
    match proposable.len() {
        0 => blocked
            .and_then(|state| state_refusal(state, name, brand))
            .map_or(Standing::Absent, Standing::Blocked),
        1 => {
            let (athlete_id, group) = proposable.swap_remove(0);
            Standing::Bindable { athlete_id, group }
        }
        _ => Standing::Blocked(refused(
            "roster_email_shared",
            format!(
                "Two athletes on your {brand} roster share {name}'s email, so a link cannot tell \
                 which one is them: link them from the group's page in the app."
            ),
        )),
    }
}

/// What reading one candidate platform for the member gave.
enum Reading {
    /// The platform's roster was not read: why.
    Refused(ProviderRosterRefusal),
    /// The roster was read; where the member stands on it.
    Read(Standing),
}

/// Read `platform`'s roster for `member`, once its terms let it be read for
/// this call ([`roster_states`]).
///
/// # Errors
///
/// Returns a repository or provider error that is no linking refusal.
async fn read_platform(
    context: &ToolExecutionContext,
    platform: &dyn CoachPlatform,
    tenant: TenantId,
    member: Uuid,
    name: &str,
) -> AppResult<Reading> {
    Ok(match roster_states(context, platform, tenant).await? {
        Ok(states) => Reading::Read(standing_on(&states, member, name, platform.brand())),
        Err(why) => Reading::Refused(why),
    })
}

/// The platform to propose on, or the refusal that answers the call: the one
/// platform the member binds on; else the first reason the roster gives
/// about them; else that no roster read lists them; else why no roster was
/// read.
fn choose<'a>(
    readings: Vec<(&'a dyn CoachPlatform, Reading)>,
    name: &str,
) -> Result<(&'a dyn CoachPlatform, String, Box<CoachingGroup>), ProviderRosterRefusal> {
    let mut bindable = Vec::new();
    let mut blocked = None;
    let mut read = Vec::new();
    let mut not_read = None;
    for (platform, reading) in readings {
        match reading {
            Reading::Read(Standing::Bindable { athlete_id, group }) => {
                bindable.push((platform, athlete_id, group));
            }
            Reading::Read(Standing::Blocked(refusal)) => {
                blocked = blocked.or(Some(refusal));
                read.push(platform);
            }
            Reading::Read(Standing::Absent) => read.push(platform),
            Reading::Refused(refusal) => not_read = not_read.or(Some(refusal)),
        }
    }
    if bindable.len() > 1 {
        let brands: Vec<&str> = bindable.iter().map(|(p, _, _)| p.brand()).collect();
        let names: Vec<&str> = bindable.iter().map(|(p, _, _)| p.user_facing()).collect();
        return Err(refused(
            "provider_ambiguous",
            format!(
                "{name} is on your {} rosters: say which with `provider` ({}).",
                brands.join(" and "),
                names.join(" or ")
            ),
        ));
    }
    if let Some(chosen) = bindable.pop() {
        return Ok(chosen);
    }
    if let Some(refusal) = blocked {
        return Err(refusal);
    }
    if !read.is_empty() {
        let brands: Vec<&str> = read.iter().map(|p| p.brand()).collect();
        return Err(refused(
            "no_roster_match",
            format!(
                "No athlete on your {} roster has {name}'s verified Dravr email: the email your \
                 platform lists for them must be the one they verified on Dravr.",
                brands.join(" or ")
            ),
        ));
    }
    // No platform at all was a candidate: nothing is connected.
    Err(not_read.unwrap_or_else(|| {
        refused(
            Refusal::NotConnected.as_str(),
            "Connect the coaching platform your athletes are on first.".to_owned(),
        )
    }))
}

/// The refusal for a member query that resolved to no one member.
fn miss_refusal(miss: &MemberMiss, query: &str) -> ToolResult {
    let reason = match miss {
        MemberMiss::Requester => Refusal::MemberIsCoach.as_str(),
        MemberMiss::NoMatch => "member_not_found",
        MemberMiss::Ambiguous(_) => "member_ambiguous",
    };
    refusal(
        reason,
        &miss.message(query, "you cannot be linked as your own athlete."),
    )
}

/// Ask a member of a group the caller coaches to confirm a coaching-platform
/// link, from chat.
pub struct ProposeAthleteLinkTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ProposeAthleteLinkTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "athlete".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "The member's name, as Dravr shows it, in a group you coach.".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Optional coaching platform to link on (`intervals_icu`, `trainingpeaks`). \
                     Omit to use the one roster that lists the member."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["athlete".to_owned()]));

        answers_with::<ProposeAthleteLinkResult>(tool_definition(
            "propose_athlete_link",
            "Propose linking a member of a group you coach to their entry on your coaching \
             platform roster (Intervals.icu, TrainingPeaks), so their workouts there read \
             through your account. Name the member; the roster entry is the one whose email is \
             their verified Dravr email. The member is asked to confirm in the Dravr app, and \
             nothing is read until they do. Use `get_provider_roster` to see who is \
             `proposable`. Refused in a group's room, for a member who stopped sharing with you, \
             and when the roster does not list their email.",
            schema,
            Some(ToolAnnotations {
                read_only_hint: Some(false),
                destructive_hint: Some(false),
                // A second call for the same member is refused: the first
                // proposal holds them.
                idempotent_hint: Some(false),
                ..ToolAnnotations::default()
            }),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let context = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async {
            let Some(query) = args
                .get("athlete")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                return Ok(refusal(
                    "athlete_required",
                    "Name the member to link with `athlete`.",
                ));
            };
            let tenant = TenantId::from_uuid(context.require_tenant()?);
            if let Some(refused) = room_refusal(&context, tenant, "Link an athlete").await? {
                return Ok(refused);
            }

            // The member is Dravr data, matched before any roster is read.
            let matched =
                match match_coached_member(&context.resources.data(), context.user_id, query)
                    .await?
                {
                    Ok(matched) => matched,
                    Err(miss) => return Ok(miss_refusal(&miss, query)),
                };
            let name = matched.display_name.as_str();
            if !matched.consented {
                return Ok(refusal(
                    "member_not_sharing",
                    &format!(
                        "{name} has stopped sharing their data with you, so they are not asked \
                         to link. They can share it again with `/group consent coach yes`."
                    ),
                ));
            }

            let requested = args
                .get("provider")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let services = delegation_services(context.resources.as_ref());
            let platforms = match roster_platforms(
                services.repos,
                services.registry,
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
            let mut readings = Vec::with_capacity(platforms.len());
            for platform in &platforms {
                let platform = platform.as_ref();
                let reading =
                    read_platform(&context, platform, tenant, matched.user_id, name).await?;
                readings.push((platform, reading));
            }
            let (platform, athlete_id, group) = match choose(readings, name) {
                Ok(chosen) => chosen,
                Err(r) => return Ok(refusal(&r.reason, &r.error)),
            };

            let link = match propose(
                services,
                &group,
                context.user_id,
                tenant,
                Proposal {
                    provider: platform.backend(),
                    provider_athlete_id: &athlete_id,
                    member_user_id: matched.user_id,
                },
            )
            .await
            {
                Ok(link) => link,
                Err(e) => {
                    return match platform_refusal(&e) {
                        Some(r) => Ok(refusal(&r.reason, &r.error)),
                        None => Err(e),
                    };
                }
            };
            info!(
                link_id = %link.id,
                group_id = %group.id,
                "propose_athlete_link: link proposed from chat"
            );
            ok_typed(
                "propose_athlete_link",
                ProposeAthleteLinkResult {
                    member: matched.display_name.clone(),
                    group: group.name.clone(),
                    provider: platform.user_facing().to_owned(),
                    next_step: format!(
                        "{name} is asked to confirm the link in the Dravr app; nothing is read \
                         until they do."
                    ),
                },
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// It stores a proposal and notifies the member; the member's confirm is the
// consent, so it is not destructive. Its answer echoes the member's Dravr
// display name and the group name, peer-written text the roster tools label
// untrusted, so this one does too.
crate::declare_security!(ProposeAthleteLinkTool => UNTRUSTED_OUTPUT);

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use pierre_core::models::groups::{GroupDigestMode, GroupRespondMode};
    use pierre_core::models::RosterAthlete;
    use pierre_services::delegated_link_states::CoachedMember;

    use super::*;

    fn group() -> CoachingGroup {
        let now = Utc::now();
        CoachingGroup {
            id: Uuid::new_v4(),
            tenant_id: Uuid::new_v4().to_string(),
            name: "Squad".to_owned(),
            description: None,
            agent_id: Uuid::new_v4().to_string(),
            owner_id: Uuid::new_v4(),
            coach_user_id: None,
            peer_data_sharing: false,
            respond_mode: GroupRespondMode::default(),
            digest_mode: GroupDigestMode::Off,
            max_members: 10,
            is_active: true,
            channel_type: None,
            channel_chat_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn entry(id: &str, state: LinkState) -> RosterLinkState {
        RosterLinkState {
            athlete: RosterAthlete::new(id, None, None),
            state,
        }
    }

    fn member(user_id: Uuid, group: &CoachingGroup) -> CoachedMember {
        CoachedMember {
            user_id,
            group: group.clone(),
        }
    }

    #[test]
    fn the_one_proposable_entry_binds_the_member_in_its_group() {
        let (alex, sam, squad) = (Uuid::new_v4(), Uuid::new_v4(), group());
        let states = [
            entry("a1", LinkState::NotInYourGroups),
            entry(
                "a2",
                LinkState::Proposable {
                    member: member(sam, &squad),
                },
            ),
            entry(
                "a3",
                LinkState::Proposable {
                    member: member(alex, &squad),
                },
            ),
        ];
        match standing_on(&states, alex, "Alex", "Intervals.icu") {
            Standing::Bindable { athlete_id, group } => {
                assert_eq!(athlete_id, "a3");
                assert_eq!(group.id, squad.id);
            }
            _ => panic!("Alex binds a3"),
        }
    }

    #[test]
    fn two_entries_under_one_email_are_refused_and_an_absent_member_is_absent() {
        let (alex, squad) = (Uuid::new_v4(), group());
        let twice = [
            entry(
                "a1",
                LinkState::Proposable {
                    member: member(alex, &squad),
                },
            ),
            entry(
                "a2",
                LinkState::Proposable {
                    member: member(alex, &squad),
                },
            ),
        ];
        match standing_on(&twice, alex, "Alex", "Intervals.icu") {
            Standing::Blocked(refusal) => assert_eq!(refusal.reason, "roster_email_shared"),
            _ => panic!("two entries share Alex's email"),
        }
        let nobody = [entry("a1", LinkState::RosterEmailMissing)];
        assert!(matches!(
            standing_on(&nobody, alex, "Alex", "Intervals.icu"),
            Standing::Absent
        ));
    }

    #[test]
    fn a_blocked_member_is_refused_with_the_state_s_reason() {
        let (alex, squad) = (Uuid::new_v4(), group());
        for (state, reason) in [
            (
                LinkState::MemberEmailUnverified {
                    member: member(alex, &squad),
                },
                "member_email_unverified",
            ),
            (
                LinkState::NoAccess {
                    member: member(alex, &squad),
                },
                "athlete_no_access",
            ),
            (
                LinkState::MemberOwnConnection {
                    member: member(alex, &squad),
                },
                "own_connection",
            ),
        ] {
            match standing_on(&[entry("a1", state)], alex, "Alex", "Intervals.icu") {
                Standing::Blocked(refusal) => assert_eq!(refusal.reason, reason),
                _ => panic!("{reason} blocks"),
            }
        }
    }
}
