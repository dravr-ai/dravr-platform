// ABOUTME: A coached athlete's plan status — published or not, its goal race, season and current phase — readable in the coach's room
// ABOUTME: Never a week or a session: a room reads every reply, so the plan itself is pointed at, not posted
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A coached athlete's plan status
//!
//! In a group the sender coaches, the plan tools are withheld (the coach
//! seat, carnet#741) and an athlete's plan is never posted: every member
//! reads the reply. A coach asking there for an athlete's plan is answered
//! by [`GetGroupMemberPlanStatusTool`] (carnet#1244): whether the athlete
//! has a published plan, its goal race, season dates and the phase the
//! athlete is in — never a week or a session — and where their coach reads
//! the plan itself: Group info's read-only view, `/plan <name>` answered to
//! the coach alone, the coach's own conversation with Dravr, and the plan
//! thread where it is changed.
//!
//! It reads only as the athlete's human coach, through the gates every
//! coach read of a plan runs ([`coached_plan_read_access`]), and looks no
//! name up for a caller the coach rule does not grant ([`roster_access`]).
//! In a group's room it reads that group's athletes alone, and a
//! conversation that does not resolve for the caller is refused rather than
//! published into. The room is asked before the caller's coach access: a
//! caller attached to a group whose coach access is off is told so in their
//! own conversation and in that group's room, and in any other group's room
//! that they are not its coach, as anyone is.

use crate::annotations::{annotations, Effect, Reach};
use crate::tool_data::ToolData;
use dravr_barrage::Stream;
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
use pierre_database::RepositoryRegistry;
use pierre_mcp_schema::PropertySchema;
use pierre_memory::training_plans;
use pierre_providers::ai_scope;
use pierre_services::athlete_clock;
use pierre_services::coach_roster::{roster_access, RosterAccess, NO_COACH_ACCESS};
use pierre_services::coached_athlete::CoachedAthleteAccess;
use pierre_tools_core::ToolResult;

use super::call_room::{call_room, CallRoom};
use super::coach_scope::refusal;
use super::plan_scope::{coached_plan_read_access, CoachedPlanRefusal};
use crate::coach_assistant;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, tool_definition, tool_result_to_response,
};
use crate::read_subject::{
    gate_message, match_coached_member, GateObject, MatchAudience, MemberMiss, COACH_ACCESS_OFF,
};
use crate::runtime::ToolRuntime;

/// The goal race a published season is aimed at.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct PlanGoalRace {
    /// The race's name.
    pub name: String,
    /// The race day, `YYYY-MM-DD`.
    pub date: String,
    /// Days from the athlete's today to the race; absent when the date does
    /// not parse.
    pub days_out: Option<i64>,
}

/// What `get_group_member_plan_status` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct GroupMemberPlanStatus {
    /// The athlete, by roster name — never their account identifier.
    pub member: String,
    /// The group the caller coaches them in.
    pub group: String,
    /// Whether the athlete has a published plan, the one they see.
    pub published: bool,
    /// The race the season is aimed at; absent without a published plan.
    pub goal_race: Option<PlanGoalRace>,
    /// First day of the season, when the plan states one.
    pub season_start: Option<String>,
    /// Day after the season's last, when the plan states one.
    pub season_end: Option<String>,
    /// The phase the athlete is in today, when one covers it.
    pub current_phase: Option<String>,
    /// Where their coach reads the plan, changes it, or builds one: relay it.
    pub where_to_read: String,
}

/// Where the coach reads a published plan: never in the group.
fn published_pointer(name: &str) -> String {
    format!(
        "{name}'s plan is never posted in a group: every member would read it. As their coach \
         you read it in Group info in the Dravr app — 'View {name}'s plan' shows it read-only, \
         'Work on {name}'s plan' opens the thread where you change it — by typing \
         `/plan {name}`, which answers you alone, on a messaging app too, or by asking for it \
         in your own conversation with Dravr."
    )
}

/// Where the coach builds a plan the athlete does not have yet.
fn unpublished_pointer(name: &str) -> String {
    format!(
        "{name} has no published plan. As their coach you build one from 'Work on {name}'s plan' \
         in Group info in the Dravr app, or in your own conversation with Dravr."
    )
}

/// The refusal for a group the caller does not coach.
fn not_this_groups_coach() -> ToolResult {
    refusal(
        "not_this_groups_coach",
        "You are not this group's coach, so its athletes' plans are not looked up here.",
    )
}

/// The one group the call reads athletes of: the room's own, or none outside
/// a room. The outer `Err` is a repository failure; the inner `Err` the
/// refusal.
async fn room_pin(context: &ToolExecutionContext) -> AppResult<Result<Option<Uuid>, ToolResult>> {
    Ok(match call_room(context).await? {
        CallRoom::Private => Ok(None),
        // A group id that is no UUID names no group the caller coaches.
        CallRoom::Group(group) => {
            Uuid::parse_str(&group).map_or_else(|_| Err(not_this_groups_coach()), |id| Ok(Some(id)))
        }
        CallRoom::Unresolved => Err(refusal(
            "room_unresolved",
            "Look up an athlete's plan from your own conversation with Dravr — this \
             conversation could not be confirmed as private to you or as a group you coach.",
        )),
    })
}

/// The refusal for a name that resolved to no one athlete.
fn miss_refusal(miss: &MemberMiss, query: &str) -> ToolResult {
    refusal(
        miss.reason(),
        &miss.message(
            query,
            "your own plan is never read in a group you coach: tell them to ask in their own \
             conversation with Dravr.",
        ),
    )
}

/// The refusal for an athlete the coach may not read the plan of.
fn read_refusal(refused: &CoachedPlanRefusal, query: &str) -> ToolResult {
    match refused {
        // Only a membership that changed since the match fails the first two
        // gates, and that reads as no match.
        CoachedPlanRefusal::Coaching(gate) => refusal(
            gate.reason(),
            &gate_message(gate, GateObject::Plan, || {
                MemberMiss::NoMatch.message(query, "")
            }),
        ),
        CoachedPlanRefusal::PlansDisabled { athlete } => refusal(
            "plans_disabled",
            &format!("Training plans are turned off in {athlete}'s workspace, so their plan is not read."),
        ),
    }
}

/// The status of `access`'s athlete's published plan: goal, season and phase,
/// never a week.
async fn plan_status(
    repos: &RepositoryRegistry,
    access: &CoachedAthleteAccess,
) -> AppResult<ToolResult> {
    let name = access.display_name.clone();
    let group = access.group.name.clone();
    let athlete = access.athlete_user_id;
    let Some(plan) = repos
        .training_plans
        .get_active_plan(&access.tenant.to_string(), &athlete.to_string())
        .await?
    else {
        return ok_typed(
            "get_group_member_plan_status",
            GroupMemberPlanStatus {
                where_to_read: unpublished_pointer(&name),
                member: name,
                group,
                published: false,
                goal_race: None,
                season_start: None,
                season_end: None,
                current_phase: None,
            },
        );
    };
    // A plan derived from first-party-only data is not described over an
    // external transport, and not reported as absent either (carnet#769).
    // Described, it is labelled as what an agent wrote with the athlete.
    if !ai_scope::admit_agent_written(plan.transport_policy) {
        return Ok(refusal(
            "unavailable_over_transport",
            &format!("{name}'s plan is not available over this interface."),
        ));
    }
    let today = athlete_clock::athlete_today(repos, athlete).await;
    let days_out =
        training_plans::parse_plan_date(&plan.goal_race.date).map(|race| (race - today).num_days());
    let current_phase: Option<String> = plan
        .phases
        .iter()
        .find(|phase| phase.covers(today))
        .map(|phase| phase.kind.as_str().to_owned());
    ok_typed(
        "get_group_member_plan_status",
        GroupMemberPlanStatus {
            where_to_read: published_pointer(&name),
            member: name,
            group,
            published: true,
            goal_race: Some(PlanGoalRace {
                name: plan.goal_race.name,
                date: plan.goal_race.date,
                days_out,
            }),
            season_start: plan.season_start,
            season_end: plan.season_end,
            current_phase,
        },
    )
}

/// Whether a coached athlete has a published plan, and where their coach
/// reads it.
#[derive(Debug)]
pub struct GetGroupMemberPlanStatusTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetGroupMemberPlanStatusTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "member".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some("The athlete's roster name.".to_owned()),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, Some(vec!["member".to_owned()]));

        answers_with::<GroupMemberPlanStatus>(tool_definition(
            "get_group_member_plan_status",
            "Whether an athlete you COACH has a published plan: goal race, season, phase, and \
             `where_to_read`. Never the weeks: relay `where_to_read`.",
            schema,
            annotations(
                "Get group member plan status",
                Effect::ReadOnly,
                Reach::Internal,
            ),
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
            let Some(query) = args
                .get("member")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                return Ok(refusal(
                    "missing_member",
                    "Missing required 'member' argument (the athlete's roster display name).",
                ));
            };
            let within = match room_pin(&context).await? {
                Ok(within) => within,
                Err(refused) => return Ok(refused),
            };

            let requester = context.user_id;
            let repos = context.resources.repos();
            let (attached, holds_coach_access) = match roster_access(repos, requester).await? {
                RosterAccess::Granted(groups) => (groups, true),
                RosterAccess::NoCoachAccess(groups) => (groups, false),
                // An admin's assistant is listed this tool with or without a
                // group (carnet#1511, Q24): it answers no match, and how an
                // admin sees a coach's members.
                RosterAccess::NotACoach
                    if ctx.is_admin && coach_assistant::serving_coach_assistant() =>
                {
                    return Ok(refusal(
                        "no_match",
                        &format!(
                            "{} {}",
                            coach_assistant::not_included_message(query),
                            coach_assistant::ADMIN_NO_GROUP_NOTE
                        ),
                    ));
                }
                RosterAccess::NotACoach => {
                    return Ok(refusal(
                        "not_a_coach",
                        "You are not the human coach of any Dravr group, so no athlete's plan \
                         is looked up here.",
                    ));
                }
            };
            // The room is asked first: every member of it reads the reply,
            // so the room of a group the caller is not attached to is told
            // that alone, never that their coach access is off.
            if within.is_some_and(|pin| attached.iter().all(|group| group.id != pin)) {
                return Ok(not_this_groups_coach());
            }
            if !holds_coach_access {
                return Ok(refusal(NO_COACH_ACCESS, COACH_ACCESS_OFF));
            }

            let found = match match_coached_member(
                repos,
                requester,
                query,
                within,
                MatchAudience::current(),
            )
            .await?
            {
                Ok(found) => found,
                Err(miss) => return Ok(miss_refusal(&miss, query)),
            };
            // The AI assistant a coach connects is shown a member who allowed
            // it there (carnet#1511), asked before the gates below record the
            // read, so a member left out is never logged as read.
            let included = repos
                .groups
                .get_member(&found.group.id.to_string(), found.user_id)
                .await?
                .is_some_and(|membership| coach_assistant::includes(&membership));
            if !included {
                return Ok(refusal(
                    "no_match",
                    &coach_assistant::not_included_message(query),
                ));
            }
            // The gates record the read on the athlete's own record before
            // their plan is read (carnet#1445).
            let access = match coached_plan_read_access(
                &context.resources,
                requester,
                found.user_id,
                found.group.id,
                "get_group_member_plan_status",
            )
            .await?
            {
                Ok(access) => access,
                Err(refused) => return Ok(read_refusal(&refused, query)),
            };
            info!(
                requester = %requester,
                athlete = %access.athlete_user_id,
                group_id = %access.group.id,
                "get_group_member_plan_status: coach read an athlete's plan status"
            );
            plan_status(repos, &access).await
        }
        .await;
        tool_result_to_response(result)
    }
}

// The goal race's name is athlete-authored text.
// A coach's tool over other members' data: listed and run over every
// transport for a human coach or an admin (`crate::tool_access`), and
// externally it shows only the members who allowed their coach's assistant
// (`crate::coach_assistant`).
crate::declare_security!(
    GetGroupMemberPlanStatusTool => empty; coach,
    reads: ToolData::own(&[Stream::Workout]),
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_published_pointer_names_every_place_the_coach_reads_the_plan() {
        let pointer = published_pointer("JF");
        assert!(
            pointer.contains("never posted in a group"),
            "the pointer says why the plan is not in the room: {pointer}"
        );
        assert!(
            pointer.contains("'View JF's plan'"),
            "the pointer names Group info's read-only view: {pointer}"
        );
        assert!(
            pointer.contains("'Work on JF's plan'"),
            "the pointer names the plan thread: {pointer}"
        );
        assert!(
            pointer.contains("`/plan JF`"),
            "the pointer names the private /plan reply: {pointer}"
        );
        assert!(
            pointer.contains("your own conversation with Dravr"),
            "the pointer names the coach's own conversation: {pointer}"
        );
    }

    #[test]
    fn the_unpublished_pointer_says_where_a_plan_is_built() {
        let pointer = unpublished_pointer("JF");
        assert!(
            pointer.starts_with("JF has no published plan."),
            "the pointer opens on the athlete having no plan: {pointer}"
        );
        assert!(
            pointer.contains("'Work on JF's plan'"),
            "the pointer names the plan thread a plan is built in: {pointer}"
        );
        assert!(
            pointer.contains("your own conversation with Dravr"),
            "the pointer names the coach's own conversation: {pointer}"
        );
    }
}
