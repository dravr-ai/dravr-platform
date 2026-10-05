// ABOUTME: Coach roster overview — every athlete the caller coaches, each consent-gated.
// ABOUTME: Read from the coach's own thread; inside a group's room it is pinned to that group.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Roster overview
//!
//! [`GetRosterOverviewTool`] answers the roster review a coach asks for in
//! their own thread — who trained, who missed sessions, who needs attention —
//! across every group they coach, in one call (registre#748).
//! [`super::groups::GetGroupMemberActivitiesTool`] reads one athlete's full
//! session history; this tool reads the whole roster's snapshot.
//!
//! Four rules decide what comes back:
//!
//! 1. **The roster is the coach attachment.** The groups are the active ones
//!    whose `coach_user_id` is the caller, and the athletes their live members
//!    — the set `list_athletes_coached_by` returns. The tenant-isolation
//!    conformance stage redacts any athlete outside that same set, so the two
//!    must never drift: an owner who is not the group's coach does not see it
//!    here.
//! 2. **A room is pinned to its group.** A reply in a group's room is read by
//!    every member, so a call surfaced from one covers that group alone, and
//!    a room whose group the caller does not coach — or a conversation that
//!    does not resolve for them — is refused rather than guessed.
//! 3. **Consent is the athlete's coach consent.** The rule
//!    `get_group_member_activities` and the group context apply to a group's
//!    coach: the member's own `coach_sharing_consent`, granted by joining
//!    (ADR-002) and revoked with `/group consent coach no`. The group's peer
//!    switch governs what members see of each other, never what their coach
//!    sees, so it plays no part here. Only a `shared` athlete carries
//!    training data; one who revoked is named with that status, so the coach
//!    knows why, and nothing is inferred.
//! 4. **Bounded.** At most [`MAX_ROSTER_ATHLETES`] snapshots are fetched per
//!    call, and `truncated` says when the roster was cut.

use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::info;
use uuid::Uuid;

use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_core::civil_time::{local_date, resolve_zone};
use pierre_core::errors::AppResult;
use pierre_core::models::groups::{CoachingGroup, GroupMember, MemberFitnessSnapshot};
use pierre_core::models::TenantId;
use pierre_core::untrusted::{display_line, ACTIVITY_NAME_MAX_CHARS};
use pierre_mcp_schema::{PropertySchema, ToolAnnotations};
use pierre_tools_core::ToolResult;

use crate::athlete_display_name::fetch_user_display_name;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, task_capable, tool_definition, tool_result_to_response,
};
use crate::group_fitness::fetch_member_snapshots;
use crate::runtime::ToolRuntime;

/// Most athletes one call fetches training for, and most it names without.
/// Each snapshot is a provider read, so the cap bounds both the turn's latency
/// and the reply's token cost.
const MAX_ROSTER_ATHLETES: usize = 40;

/// Recent sessions listed per athlete. The weekly totals carry the volume;
/// these name what the sessions were.
const RECENT_ACTIVITIES_PER_ATHLETE: usize = 3;

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

/// One session in an athlete's recent list.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RosterOverviewActivity {
    /// The day the athlete trained, on their own clock (`YYYY-MM-DD`).
    pub date: String,
    /// That day's weekday, on the athlete's clock.
    pub weekday: String,
    /// Sport, as the provider classified it.
    pub sport: String,
    /// The session's title, when the provider gave one.
    pub name: String,
    /// Moving time in whole minutes.
    pub duration_minutes: i64,
    /// Distance in kilometres; absent for a session with no distance.
    pub distance_km: Option<f64>,
}

/// A shared athlete's training, as the group roster card carries it.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RosterTraining {
    /// Days since their last session; absent with none in the lookback.
    pub days_since_last_activity: Option<i32>,
    /// Sessions this week.
    pub sessions_this_week: i32,
    /// Active minutes this week, which covers sources with no distance.
    pub minutes_this_week: i64,
    /// Kilometres this week.
    pub km_this_week: f64,
    /// Kilometres the week before, for the trend.
    pub km_previous_week: Option<f64>,
    /// The sport they log most.
    pub primary_sport: Option<String>,
    /// Chronic training load — fitness; absent when it could not be computed.
    pub ctl: Option<f64>,
    /// Form, read as a share of this athlete's own fitness with its band.
    /// Never compare it across athletes as an absolute number.
    pub form: Option<String>,
    /// Their latest sessions, newest first.
    pub recent_activities: Vec<RosterOverviewActivity>,
    /// The day each connected source last delivered a session.
    pub last_activity_per_source: BTreeMap<String, String>,
    /// Sources whose connection died and must be reconnected by the athlete.
    pub needs_reconnect: Vec<String>,
    /// True when the data came from a stale cache that could not be refreshed;
    /// quiet recent days are then not evidence they stopped training.
    pub stale: bool,
}

/// One athlete on the coach's roster.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RosterOverviewAthlete {
    /// Display name — never their account identifier.
    pub name: String,
    /// The groups, held by the coach, the athlete belongs to.
    pub groups: Vec<String>,
    /// Whether their training is listed, and why not when it is not.
    pub status: RosterAthleteStatus,
    /// Their training; present only when `status` is `shared`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub training: Option<RosterTraining>,
}

/// One group the overview covers.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RosterOverviewGroup {
    /// The group's name.
    pub name: String,
    /// Live athletes in the group.
    pub athlete_count: usize,
}

/// What `get_roster_overview` answers with.
#[derive(Debug, Serialize, schemars::JsonSchema)]
pub struct RosterOverviewResult {
    /// The groups covered: every one the caller coaches, or the room's own.
    pub groups: Vec<RosterOverviewGroup>,
    /// Shared athletes first, the longest without a session at the top; then
    /// the athletes whose training is not listed, by name.
    pub athletes: Vec<RosterOverviewAthlete>,
    /// True when the roster held more athletes than one call lists.
    pub truncated: bool,
}

/// An athlete gathered across the coach's groups, before any data is read.
struct RosterEntry {
    user_id: Uuid,
    groups: Vec<String>,
    status: RosterAthleteStatus,
}

/// A refusal in the tool's error shape, with a machine-readable reason.
fn refusal(reason: &str, message: &str) -> ToolResult {
    ToolResult::error(json!({ "reason": reason, "error": message }))
}

/// The groups the call covers, or the refusal that explains why it covers none.
///
/// The outer `Err` is a repository failure; the inner `Err` is the refusal.
async fn covered_groups(
    context: &ToolExecutionContext,
    arg_group: Option<&str>,
) -> AppResult<Result<Vec<CoachingGroup>, ToolResult>> {
    let requester = context.user_id;
    let repos = context.resources.repos();
    let mut groups: Vec<CoachingGroup> = repos
        .groups
        .list_groups_coached_by(requester)
        .await?
        .into_iter()
        .filter(|g| g.is_active)
        .collect();
    if groups.is_empty() {
        return Ok(Err(refusal(
            "not_a_coach",
            "You are not the coach of any group, so there is no roster to review. A coach joins \
             a group by redeeming its coach invite.",
        )));
    }

    if let Some(conversation_id) = context.conversation_id.as_deref() {
        let tenant = context
            .conversation_ref()
            .map(|(_, tenant)| tenant)
            .or_else(|| context.tenant_id.map(TenantId::from_uuid));
        let conversation = match tenant {
            Some(tenant) => {
                repos
                    .chat
                    .get_conversation(conversation_id, &requester.to_string(), tenant)
                    .await?
            }
            None => None,
        };
        // A conversation the caller cannot resolve may be any room: fail
        // closed rather than publish a roster into it.
        let Some(conversation) = conversation else {
            return Ok(Err(refusal(
                "room_unresolved",
                "Review your roster from your own conversation with Dravr — this conversation \
                 could not be confirmed as private to you.",
            )));
        };
        if let Some(room_group) = conversation.group_id.as_deref() {
            groups.retain(|g| g.id.to_string() == room_group);
            if groups.is_empty() {
                return Ok(Err(refusal(
                    "not_this_groups_coach",
                    "You are not this group's coach, so its roster is not reviewed here. Review \
                     the groups you coach from your own conversation with Dravr.",
                )));
            }
        }
    }

    if let Some(pin) = arg_group {
        groups.retain(|g| g.id.to_string() == pin);
        if groups.is_empty() {
            return Ok(Err(refusal(
                "not_your_group",
                "That group is not one you coach here. Omit `group_id` to review every athlete \
                 you can see from this conversation.",
            )));
        }
    }
    Ok(Ok(groups))
}

/// Gather each live athlete of `groups` once, with the most useful status any
/// of their groups grants, and the per-group summary.
async fn gather_roster(
    context: &ToolExecutionContext,
    groups: &[CoachingGroup],
) -> AppResult<(Vec<RosterOverviewGroup>, Vec<RosterEntry>)> {
    let repos = context.resources.repos();
    let mut summaries = Vec::with_capacity(groups.len());
    let mut entries: Vec<RosterEntry> = Vec::new();
    let mut index: HashMap<Uuid, usize> = HashMap::new();
    for group in groups {
        let members = repos.groups.list_members(&group.id.to_string()).await?;
        let mut athlete_count = 0;
        for member in members.iter().filter(|m| m.left_at.is_none()) {
            if member.user_id == context.user_id {
                continue;
            }
            athlete_count += 1;
            let status = RosterAthleteStatus::of_member(member);
            if let Some(&at) = index.get(&member.user_id) {
                // Shared through any one group is shared: the consent is per
                // membership, and each grants the same coach the same read.
                let entry = &mut entries[at];
                entry.groups.push(group.name.clone());
                if status == RosterAthleteStatus::Shared {
                    entry.status = status;
                }
            } else {
                index.insert(member.user_id, entries.len());
                entries.push(RosterEntry {
                    user_id: member.user_id,
                    groups: vec![group.name.clone()],
                    status,
                });
            }
        }
        summaries.push(RosterOverviewGroup {
            name: group.name.clone(),
            athlete_count,
        });
    }
    Ok((summaries, entries))
}

/// Project a snapshot to the training the overview lists.
fn project_training(snapshot: &MemberFitnessSnapshot) -> RosterTraining {
    let zone = resolve_zone(snapshot.timezone.as_deref());
    let recent_activities = snapshot
        .recent_activities
        .iter()
        .take(RECENT_ACTIVITIES_PER_ATHLETE)
        .map(|a| {
            let day = local_date(a.start, zone);
            RosterOverviewActivity {
                date: day.format("%Y-%m-%d").to_string(),
                weekday: day.format("%A").to_string(),
                sport: a.sport.clone(),
                name: display_line(&a.name, ACTIVITY_NAME_MAX_CHARS),
                duration_minutes: a.duration_minutes,
                distance_km: a.distance_km,
            }
        })
        .collect();
    RosterTraining {
        days_since_last_activity: snapshot.days_since_last_activity,
        sessions_this_week: snapshot.weekly_activity_count,
        minutes_this_week: snapshot.weekly_duration_seconds / 60,
        km_this_week: snapshot.weekly_volume_km,
        km_previous_week: snapshot.previous_week_volume_km,
        primary_sport: snapshot.primary_sport.clone(),
        ctl: snapshot.ctl.map(f64::round),
        form: snapshot.form_reading().map(|reading| reading.inline()),
        recent_activities,
        last_activity_per_source: snapshot
            .last_activity_per_provider
            .iter()
            .map(|(source, at)| {
                (
                    source.clone(),
                    local_date(*at, zone).format("%Y-%m-%d").to_string(),
                )
            })
            .collect(),
        needs_reconnect: snapshot.needs_reauth_providers.clone(),
        stale: snapshot.served_stale,
    }
}

/// Order shared athletes for a "who needs attention" read: no session in the
/// lookback first, then the longest since their last one.
fn attention_order(a: &RosterOverviewAthlete, b: &RosterOverviewAthlete) -> Ordering {
    let days = |athlete: &RosterOverviewAthlete| {
        athlete
            .training
            .as_ref()
            .map_or(Some(i32::MAX), |t| t.days_since_last_activity)
            .unwrap_or(i32::MAX)
    };
    days(b).cmp(&days(a)).then_with(|| a.name.cmp(&b.name))
}

/// Build the overview for the athletes of `groups`.
async fn build_overview(
    context: &ToolExecutionContext,
    groups: &[CoachingGroup],
) -> AppResult<RosterOverviewResult> {
    let runtime = &context.resources;
    let data = runtime.data();
    let (summaries, entries) = gather_roster(context, groups).await?;
    let total = entries.len();

    let (sharing, withheld): (Vec<RosterEntry>, Vec<RosterEntry>) = entries
        .into_iter()
        .partition(|e| e.status == RosterAthleteStatus::Shared);

    // Split the sharers by whether any training source is connected: one
    // without a source has nothing to read, and saying so beats an empty card.
    let mut readable = Vec::new();
    let mut sourceless = Vec::new();
    for entry in sharing {
        let connections = data
            .repos()
            .provider_connections
            .get_for_user(entry.user_id, None)
            .await?;
        if connections.is_empty() {
            sourceless.push(RosterEntry {
                status: RosterAthleteStatus::NoSource,
                ..entry
            });
        } else {
            readable.push(entry);
        }
    }
    let truncated = readable.len() > MAX_ROSTER_ATHLETES
        || withheld.len() + sourceless.len() > MAX_ROSTER_ATHLETES;
    readable.truncate(MAX_ROSTER_ATHLETES);

    let fallback_tenant = TenantId::from_uuid(context.require_tenant()?);
    let ids: Vec<Uuid> = readable.iter().map(|e| e.user_id).collect();
    // One snapshot per id, in the order asked: `join_all` keeps it.
    let snapshots = fetch_member_snapshots(runtime, &ids, fallback_tenant).await;

    let mut shared: Vec<RosterOverviewAthlete> = readable
        .into_iter()
        .zip(snapshots)
        .map(|(entry, snapshot)| RosterOverviewAthlete {
            training: Some(project_training(&snapshot)),
            name: snapshot.display_name,
            groups: entry.groups,
            status: RosterAthleteStatus::Shared,
        })
        .collect();
    shared.sort_by(attention_order);

    let mut others = Vec::new();
    for entry in sourceless.into_iter().chain(withheld) {
        others.push(RosterOverviewAthlete {
            name: fetch_user_display_name(&data, entry.user_id).await,
            groups: entry.groups,
            status: entry.status,
            training: None,
        });
    }
    others.sort_by(|a, b| a.name.cmp(&b.name));
    others.truncate(MAX_ROSTER_ATHLETES);

    info!(
        requester = %context.user_id,
        groups = summaries.len(),
        athletes = total,
        shared = shared.len(),
        truncated,
        "get_roster_overview: consent-gated roster read"
    );
    shared.extend(others);
    Ok(RosterOverviewResult {
        groups: summaries,
        athletes: shared,
        truncated,
    })
}

/// Review every athlete the caller coaches, consent-gated, in one call.
pub struct GetRosterOverviewTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetRosterOverviewTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "group_id".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Optional id of one group you coach, to review only its athletes. Omit to \
                     review every athlete you coach."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );
        let schema = object_schema(properties, None);

        answers_with::<RosterOverviewResult>(task_capable(tool_definition(
            "get_roster_overview",
            "Review the athletes you COACH across your groups in one call: who trained this week, \
             who has gone quiet, weekly volume, fitness and form, recent sessions and broken \
             sources. Each athlete carries a `status`; only `shared` athletes carry `training`. \
             `no_coach_consent` means the athlete stopped sharing with you — they can share again \
             with `/group consent coach yes`; never infer their numbers. In a group's room the \
             review covers that group only. Use `get_group_member_activities` for one athlete's \
             full session history.",
            schema,
            Some(ToolAnnotations {
                read_only_hint: Some(true),
                destructive_hint: Some(false),
                idempotent_hint: Some(true),
                ..ToolAnnotations::default()
            }),
        )))
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
            let arg_group = args
                .get("group_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let groups = match covered_groups(&context, arg_group).await? {
                Ok(groups) => groups,
                Err(refused) => return Ok(refused),
            };
            ok_typed(
                "get_roster_overview",
                build_overview(&context, &groups).await?,
            )
        }
        .await;
        tool_result_to_response(result)
    }
}

// Athlete display names and session titles are third-party text.
crate::declare_security!(GetRosterOverviewTool => UNTRUSTED_OUTPUT);

#[cfg(test)]
mod tests {
    use super::*;

    /// A shared athlete; `has_snapshot` false models one with no `training`
    /// listed, `days` the days since their last session.
    fn athlete(name: &str, has_snapshot: bool, days: Option<i32>) -> RosterOverviewAthlete {
        RosterOverviewAthlete {
            name: name.to_owned(),
            groups: Vec::new(),
            status: RosterAthleteStatus::Shared,
            training: has_snapshot.then(|| RosterTraining {
                days_since_last_activity: days,
                sessions_this_week: 0,
                minutes_this_week: 0,
                km_this_week: 0.0,
                km_previous_week: None,
                primary_sport: None,
                ctl: None,
                form: None,
                recent_activities: Vec::new(),
                last_activity_per_source: BTreeMap::new(),
                needs_reconnect: Vec::new(),
                stale: false,
            }),
        }
    }

    #[test]
    fn attention_order_puts_the_quietest_athlete_first() {
        let mut roster = [
            athlete("Recent", true, Some(1)),
            athlete("NoSession", true, None),
            athlete("Week", true, Some(8)),
            athlete("NoSnapshot", false, None),
        ];
        roster.sort_by(attention_order);
        let names: Vec<&str> = roster.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["NoSession", "NoSnapshot", "Week", "Recent"]);
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
            RosterAthleteStatus::Shared
        );
        assert_eq!(
            RosterAthleteStatus::of_member(&json(false)),
            RosterAthleteStatus::NoCoachConsent
        );
    }
}
