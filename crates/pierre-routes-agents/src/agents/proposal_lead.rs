// ABOUTME: Orders an onboarding agent proposal and picks the agent that leads it
// ABOUTME: Cold-start lead rules: declared sport first, never a coach tool, never RED-S

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::cmp::Ordering;
use std::collections::HashSet;

use pierre_core::models::agents::AgentListItem;
use pierre_services::team_rules;

/// The deterministic proposal order: best score first, then (on a cold
/// start) the cold-start lead order and the fewest activities asked, then the
/// title — never an id, which would make the lead an accident of UUID
/// generation.
pub(super) fn proposal_order(
    a: &(AgentListItem, f32),
    b: &(AgentListItem, f32),
    cold_start: bool,
) -> Ordering {
    let rank = |item: &AgentListItem| {
        if cold_start {
            team_rules::cold_start_rank(item.agent.handle.as_deref())
        } else {
            0
        }
    };
    // On a cold start an agent asking fewer activities fits an athlete who
    // has synced none better: the road-running agent before the ultra one.
    let floor = |item: &AgentListItem| {
        if cold_start {
            item.agent.prerequisites.min_activities
        } else {
            0
        }
    };
    b.1.partial_cmp(&a.1)
        .unwrap_or(Ordering::Equal)
        .then_with(|| rank(&a.0).cmp(&rank(&b.0)))
        .then_with(|| floor(&a.0).cmp(&floor(&b.0)))
        .then_with(|| a.0.agent.title.cmp(&b.0.agent.title))
        .then_with(|| a.0.agent.handle.cmp(&b.0.agent.handle))
}

/// The best agent for the sport the athlete declared among `pool`, members
/// excluded, in the deterministic cold-start order.
pub(super) fn best_declared_sport_agent<'pool>(
    pool: impl Iterator<Item = &'pool (AgentListItem, f32)>,
    member_ids: &HashSet<String>,
) -> Option<String> {
    pool.filter(|(item, _)| {
        declared_sport_lead(item) && !member_ids.contains(&item.agent.id.to_string())
    })
    .min_by(|a, b| proposal_order(a, b, true))
    .map(|(item, _)| item.agent.id.to_string())
}

/// Rotate the proposal's lead to the front, keeping the rest in order.
///
/// The lead answers the person's own thread about their own training, so it
/// is never a coach tool, whoever ranked it first: a coach who also trains was
/// led by the roster agent, whose whole premise is a coach with no training
/// of their own (carnet#1149). A cold start never leads with RED-S either, and
/// leads with the agent for the sport the athlete declared when there is one
/// (carnet#1151). Coach tools stay in the list, to add beside the team.
pub(super) fn put_lead_first(out: &mut [(AgentListItem, f32, String)], cold_start: bool) {
    let may_lead = |item: &AgentListItem| {
        !item.agent.is_coach_facing()
            && (!cold_start || team_rules::may_lead_cold_start(item.agent.handle.as_deref()))
    };
    let lead = cold_start
        .then(|| {
            out.iter()
                .position(|(item, _, _)| declared_sport_lead(item))
        })
        .flatten()
        .or_else(|| out.iter().position(|(item, _, _)| may_lead(item)));
    if let Some(lead) = lead {
        out[..=lead].rotate_right(1);
    }
}

/// Whether `item` is, on a cold start, an agent for the sport the athlete
/// declared: a cold start admits a sport-specific agent only through that
/// sport ([`pierre_services::agents::score_declared_sport`]). Never a coach tool.
pub(super) fn declared_sport_lead(item: &AgentListItem) -> bool {
    !item.agent.prerequisites.activity_types.is_empty() && !item.agent.is_coach_facing()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use uuid::Uuid;

    /// A catalogue agent as the proposal reads it: `types` are its activity
    /// prerequisites, `tags` its tags.
    fn item(handle: &str, types: &[&str], min_activities: u32, tags: &[&str]) -> AgentListItem {
        serde_json::from_value(json!({
            "id": Uuid::new_v4(),
            "user_id": "00000000-0000-0000-0000-000000000001",
            "tenant_id": "00000000-0000-0000-0000-000000000002",
            "title": handle,
            "description": null,
            "system_prompt": "Test.",
            "category": "training",
            "tags": tags,
            "token_count": 0,
            "created_at": "2026-10-23T00:00:00Z",
            "updated_at": "2026-10-23T00:00:00Z",
            "handle": handle,
            "prerequisites": {
                "providers": if types.is_empty() { json!([]) } else { json!(["strava"]) },
                "min_activities": min_activities,
                "activity_types": types,
            },
            "is_assigned": false,
            "is_favorite": false,
            "is_active": false,
            "use_count": 0,
            "last_used_at": null,
        }))
        .unwrap()
    }

    fn handles(out: &[(AgentListItem, f32, String)]) -> Vec<&str> {
        out.iter()
            .map(|(item, _, _)| item.agent.handle.as_deref().unwrap_or_default())
            .collect()
    }

    fn ranked(items: Vec<AgentListItem>) -> Vec<(AgentListItem, f32, String)> {
        items
            .into_iter()
            .map(|item| (item, 0.5, String::new()))
            .collect()
    }

    /// carnet#1151: a runner with a connected watch and no synced run was led
    /// by Mobility even with the road-running agent in the proposal.
    #[test]
    fn a_cold_start_leads_with_the_agent_for_the_declared_sport() {
        let mut out = ranked(vec![
            item("mobility-agent", &[], 0, &[]),
            item("fuelling-agent", &[], 0, &[]),
            item("road-running-agent", &["Run"], 5, &[]),
        ]);
        put_lead_first(&mut out, true);
        assert_eq!(
            handles(&out),
            ["road-running-agent", "mobility-agent", "fuelling-agent"],
            "bug: the declared sport's agent must lead a cold start, the rest keeping their order"
        );
    }

    /// carnet#1149: a coach who also trains was led by the roster agent.
    #[test]
    fn no_proposal_leads_with_a_coach_tool() {
        for cold_start in [true, false] {
            let mut out = ranked(vec![
                item("roster-agent", &[], 0, &["coach-tool"]),
                item("recovery-agent", &[], 0, &[]),
            ]);
            put_lead_first(&mut out, cold_start);
            assert_eq!(
                handles(&out),
                ["recovery-agent", "roster-agent"],
                "bug: a coach tool led (cold_start = {cold_start})"
            );
        }
        let mut builders = ranked(vec![
            item("taper-builder-agent", &["Run", "Ride"], 14, &["coach-tool"]),
            item("mobility-agent", &[], 0, &[]),
        ]);
        put_lead_first(&mut builders, true);
        assert_eq!(
            handles(&builders),
            ["mobility-agent", "taper-builder-agent"],
            "a coach-tool builder is not the declared sport's lead"
        );
    }

    #[test]
    fn a_cold_start_never_leads_with_red_s_and_a_profiled_one_keeps_the_rank() {
        let mut cold = ranked(vec![
            item(team_rules::RED_S_AGENT_HANDLE, &[], 0, &[]),
            item("fuelling-agent", &[], 0, &[]),
        ]);
        put_lead_first(&mut cold, true);
        assert_eq!(
            handles(&cold)[0],
            "fuelling-agent",
            "bug: RED-S led a cold start"
        );

        let mut profiled = ranked(vec![
            item("mobility-agent", &[], 0, &[]),
            item("road-running-agent", &["Run"], 5, &[]),
        ]);
        put_lead_first(&mut profiled, false);
        assert_eq!(
            handles(&profiled),
            ["mobility-agent", "road-running-agent"],
            "with a profile the ranking decides"
        );
    }

    #[test]
    fn the_declared_sport_pick_prefers_the_single_sport_agent_asking_fewest_activities() {
        let pool = [
            (item("ultra-trail-agent", &["Run"], 14, &[]), 1.0),
            (item("endurance-agent", &["Run", "Ride"], 14, &[]), 0.5),
            (item("road-running-agent", &["Run"], 5, &[]), 1.0),
            (item("mobility-agent", &[], 0, &[]), 0.5),
        ];
        let road = pool[2].0.agent.id.to_string();
        assert_eq!(
            best_declared_sport_agent(pool.iter(), &HashSet::new()),
            Some(road.clone()),
            "bug: the single-sport agent asking fewest activities was not picked"
        );
        let members = HashSet::from([road]);
        assert_eq!(
            best_declared_sport_agent(pool.iter(), &members),
            Some(pool[0].0.agent.id.to_string()),
            "a member is never picked to lead"
        );
    }
}
