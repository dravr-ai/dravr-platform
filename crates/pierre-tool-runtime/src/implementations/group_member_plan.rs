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
//! coach read of a plan runs ([`coached_plan_read_access`]). In a group's
//! room it reads that group's athletes alone, and a conversation that does
//! not resolve for the caller is refused rather than published into.

use crate::annotations::{annotations, Effect, Reach};
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
use pierre_services::coached_athlete::CoachedAthleteAccess;
use pierre_tools_core::ToolResult;

use super::coach_scope::refusal;
use super::group_roster::{call_room, CallRoom};
use super::plan_scope::{
    coached_plan_read_access, gate_message, match_coached_member, CoachedPlanRefusal, MemberMiss,
};
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, tool_definition, tool_result_to_response,
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
    let reason = match miss {
        MemberMiss::Requester => "self",
        MemberMiss::NoMatch => "no_match",
        MemberMiss::Ambiguous(_) => "ambiguous",
    };
    refusal(
        reason,
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
            &gate_message(gate, || MemberMiss::NoMatch.message(query, "")),
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
    if !ai_scope::admit_derived(plan.transport_policy) {
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
            annotations(Effect::ReadOnly, Reach::Internal),
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
            let coached: Vec<Uuid> = repos
                .groups
                .list_groups_coached_by(requester)
                .await?
                .into_iter()
                .filter(|group| group.is_active)
                .map(|group| group.id)
                .collect();
            if coached.is_empty() {
                return Ok(refusal(
                    "not_a_coach",
                    "You are not the human coach of any Dravr group, so no athlete's plan is \
                     looked up here.",
                ));
            }
            if within.is_some_and(|pin| !coached.contains(&pin)) {
                return Ok(not_this_groups_coach());
            }

            let found = match match_coached_member(repos, requester, query, within).await? {
                Ok(found) => found,
                Err(miss) => return Ok(miss_refusal(&miss, query)),
            };
            let access = match coached_plan_read_access(
                &context.resources,
                requester,
                found.user_id,
                found.group.id,
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
crate::declare_security!(GetGroupMemberPlanStatusTool => UNTRUSTED_OUTPUT);

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
