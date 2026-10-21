// ABOUTME: The rule table that picks an athlete's starting team beside the lead — goal, sleep/HRV source, strength work
// ABOUTME: Pure and keyed on catalogue handles; also the cold-start lead order and the LLM member-reason prompt + parser

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Team rules (carnet#1097).
//!
//! The onboarding proposal's lead comes from the activity scan and the LLM
//! re-rank. The agents beside it come from this table, which the LLM never
//! decides — it only writes each member's one-line reason:
//!
//! | Rule | Signal | Agent | Ticked |
//! |---|---|---|---|
//! | [`TeamRule::Goal`] / [`TeamRule::LongSessions`] | a goal fact, or a session ≥ [`LONG_SESSION_MIN`] | Fuelling | yes |
//! | [`TeamRule::SleepSource`] | a connected provider that sends sleep or recovery data | Recovery | yes |
//! | [`TeamRule::Strength`] | strength or crossfit sessions ≥ the sport floor | Mobility | no |
//!
//! "Long event" is the calibration interview's Fueling rule, read from the
//! same threshold, so the two never disagree about who trains long. Mental
//! resilience has no agent (carnet#1104) and community is Groups, not an
//! agent, so neither appears here.
//!
//! Keyed on catalogue handles, never on [`AgentCategory`]: the nutrition
//! category holds both Fuelling and RED-S.
//!
//! [`AgentCategory`]: pierre_core::models::agents::AgentCategory

use std::collections::{HashMap, HashSet};
use std::fmt::Write;
use std::hash::BuildHasher;

use pierre_contremaitre::messaging_strings::{
    KEY_AGENTS_TEAM_REASON_FUELLING, KEY_AGENTS_TEAM_REASON_MOBILITY,
    KEY_AGENTS_TEAM_REASON_RECOVERY,
};
use pierre_core::models::{
    resolve_sport_type, Dossier, Pillar, SportProfile, SportType, LONG_SESSION_MIN,
};
use pierre_memory::FactKind;
use serde::{Deserialize, Serialize};

use crate::agents::extract_json_array;
use crate::default_agent::{FUELLING_AGENT_HANDLE, MOBILITY_AGENT_HANDLE, RECOVERY_AGENT_HANDLE};

/// Catalogue handle of the RED-S energy-availability agent, which never leads
/// a cold start: it answers a clinical concern, not a first conversation.
pub const RED_S_AGENT_HANDLE: &str = "reds-energy-availability-agent";

/// Who leads an athlete the proposal knows nothing about, in order.
///
/// No provider and no activities: the first of these the athlete may be given
/// leads. Every other agent follows them, and [`RED_S_AGENT_HANDLE`] comes last.
pub const COLD_START_LEAD_ORDER: [&str; 3] = [
    FUELLING_AGENT_HANDLE,
    RECOVERY_AGENT_HANDLE,
    MOBILITY_AGENT_HANDLE,
];

/// Why the rule table put an agent on the team. The wire value is what the
/// clients key the member's source line on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TeamRule {
    /// The athlete named a goal: Fuelling.
    Goal,
    /// No goal, but a session of [`LONG_SESSION_MIN`] or more: Fuelling.
    LongSessions,
    /// A connected provider sends sleep or recovery data: Recovery.
    SleepSource,
    /// Strength or crossfit sessions in the window: Mobility.
    Strength,
}

impl TeamRule {
    /// The catalogue agent this rule puts on the team.
    #[must_use]
    pub const fn handle(self) -> &'static str {
        match self {
            Self::Goal | Self::LongSessions => FUELLING_AGENT_HANDLE,
            Self::SleepSource => RECOVERY_AGENT_HANDLE,
            Self::Strength => MOBILITY_AGENT_HANDLE,
        }
    }

    /// Whether the member arrives ticked. Mobility is offered, not chosen:
    /// strength work alone is a weak reason to add an agent.
    #[must_use]
    pub const fn preselected(self) -> bool {
        !matches!(self, Self::Strength)
    }

    /// The server-rendered contremaitre key for the reason shown when the LLM
    /// wrote none. The Recovery reason takes the provider's display name as
    /// `{0}`.
    #[must_use]
    pub const fn fallback_reason_key(self) -> &'static str {
        match self {
            Self::Goal | Self::LongSessions => KEY_AGENTS_TEAM_REASON_FUELLING,
            Self::SleepSource => KEY_AGENTS_TEAM_REASON_RECOVERY,
            Self::Strength => KEY_AGENTS_TEAM_REASON_MOBILITY,
        }
    }
}

/// What the rule table reads about the athlete.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TeamSignals<'a> {
    /// A non-stale goal fact exists.
    pub goal: bool,
    /// The longest session in the window, in minutes.
    pub longest_session_min: u32,
    /// Display name of a connected provider that sends sleep or recovery data.
    pub sleep_source: Option<&'a str>,
    /// Strength and crossfit sessions in the window.
    pub strength_sessions: u32,
}

impl<'a> TeamSignals<'a> {
    /// Read the signals from what the proposal already holds. No profile (a
    /// cold start) reads as no sessions; no dossier as no goal.
    #[must_use]
    pub fn read(
        profile: Option<&SportProfile>,
        dossier: Option<&Dossier>,
        sleep_source: Option<&'a str>,
    ) -> Self {
        Self {
            goal: dossier.is_some_and(has_goal),
            longest_session_min: profile.map_or(0, |p| p.longest_session_min),
            sleep_source,
            strength_sessions: profile.map_or(0, strength_sessions),
        }
    }
}

/// One agent the rule table puts on the team.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamPick {
    /// The rule that picked it.
    pub rule: TeamRule,
}

impl TeamPick {
    /// The catalogue handle of the agent picked.
    #[must_use]
    pub const fn handle(self) -> &'static str {
        self.rule.handle()
    }

    /// Whether it arrives ticked.
    #[must_use]
    pub const fn preselected(self) -> bool {
        self.rule.preselected()
    }
}

/// The team members the signals earn, in a fixed order: Fuelling, Recovery,
/// Mobility. `min_strength_sessions` is the sport floor the activity scan
/// already uses (`COACH_REC_MIN_ACTIVITIES_FOR_SPORT`).
#[must_use]
pub fn pick_members(signals: &TeamSignals<'_>, min_strength_sessions: u32) -> Vec<TeamPick> {
    let mut picks = Vec::with_capacity(3);
    if signals.goal {
        picks.push(TeamPick {
            rule: TeamRule::Goal,
        });
    } else if signals.longest_session_min >= LONG_SESSION_MIN {
        picks.push(TeamPick {
            rule: TeamRule::LongSessions,
        });
    }
    if signals.sleep_source.is_some() {
        picks.push(TeamPick {
            rule: TeamRule::SleepSource,
        });
    }
    if signals.strength_sessions > 0 && signals.strength_sessions >= min_strength_sessions {
        picks.push(TeamPick {
            rule: TeamRule::Strength,
        });
    }
    picks
}

/// The agent a newly covered pillar may offer the athlete.
///
/// Offered in "Finish your profile" ("Add X to my team · Not now"). `None`
/// for a pillar no agent
/// answers: mental resilience (carnet#1104), community (Groups), and recovery
/// optimisation, whose substance-use framing the Recovery agent does not own.
#[must_use]
pub const fn agent_for_pillar(pillar: Pillar) -> Option<&'static str> {
    match pillar {
        Pillar::Fuelling => Some(FUELLING_AGENT_HANDLE),
        Pillar::SleepAndRecovery => Some(RECOVERY_AGENT_HANDLE),
        Pillar::TrainingAndMovement => Some(MOBILITY_AGENT_HANDLE),
        Pillar::MentalResilience
        | Pillar::CommunityAndConnection
        | Pillar::RecoveryOptimisation => None,
    }
}

/// Where an agent sorts for a cold-start lead: its place in
/// [`COLD_START_LEAD_ORDER`], every other agent after those, RED-S last.
#[must_use]
pub fn cold_start_rank(handle: Option<&str>) -> usize {
    match handle {
        Some(RED_S_AGENT_HANDLE) => COLD_START_LEAD_ORDER.len() + 1,
        Some(handle) => COLD_START_LEAD_ORDER
            .iter()
            .position(|lead| *lead == handle)
            .unwrap_or(COLD_START_LEAD_ORDER.len()),
        None => COLD_START_LEAD_ORDER.len(),
    }
}

/// Whether an agent may lead a cold start. RED-S never does.
#[must_use]
pub fn may_lead_cold_start(handle: Option<&str>) -> bool {
    handle != Some(RED_S_AGENT_HANDLE)
}

/// Whether the dossier holds a live goal — the calibration interview's
/// `dated_goal` signal, read the same way.
#[must_use]
pub fn has_goal(dossier: &Dossier) -> bool {
    dossier
        .pillars
        .values()
        .flatten()
        .any(|fact| fact.kind == FactKind::Goal.as_str() && !fact.stale)
}

/// The athlete's first live goal, in pillar order, as they wrote it. The
/// caller sanitises it before it reaches a prompt or the wire.
#[must_use]
pub fn first_goal(dossier: &Dossier) -> Option<&str> {
    Pillar::ALL
        .iter()
        .filter_map(|pillar| dossier.pillars.get(pillar))
        .flatten()
        .find(|fact| fact.kind == FactKind::Goal.as_str() && !fact.stale)
        .map(|fact| fact.object.as_str())
        .filter(|goal| !goal.trim().is_empty())
}

/// Strength and crossfit sessions in the profile (weight training folds into
/// strength).
#[must_use]
pub fn strength_sessions(profile: &SportProfile) -> u32 {
    profile
        .sport_counts
        .iter()
        .filter(|(label, _)| {
            matches!(
                resolve_sport_type(label),
                Some(SportType::StrengthTraining | SportType::Crossfit)
            )
        })
        .map(|(_, &count)| count)
        .sum()
}

/// A team member the re-rank call writes a reason for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberContext {
    /// Agent id.
    pub id: String,
    /// Agent title, as the athlete reads it.
    pub title: String,
    /// What earned it a place, in plain words for the model.
    pub why: &'static str,
}

/// What the model is told earned each rule's member its place.
#[must_use]
pub const fn rule_context(rule: TeamRule) -> &'static str {
    match rule {
        TeamRule::Goal => "the athlete named a goal they are training toward",
        TeamRule::LongSessions => "the athlete trains sessions of three hours or more",
        TeamRule::SleepSource => "the athlete's connected provider sends sleep and HRV data",
        TeamRule::Strength => "the athlete does regular strength work",
    }
}

/// The members block appended to the re-rank user prompt.
///
/// The system prompt
/// (contremaitre `agent_rerank.md`) tells the model to add one item per
/// listed member to the same array and never to rank one. Empty when there
/// are no members, so the prompt is exactly what it was before.
#[must_use]
pub fn members_prompt_section(members: &[MemberContext]) -> String {
    if members.is_empty() {
        return String::new();
    }
    let mut section = String::from(
        "\n\nTeam members already chosen (do not rank them; write one reason each):\n",
    );
    for member in members {
        let _ = writeln!(
            section,
            "- id: {id}\n  title: {title}\n  why: {why}",
            id = member.id,
            title = member.title,
            why = member.why,
        );
    }
    section
}

#[derive(Debug, Deserialize)]
struct RawReason {
    id: String,
    reason: String,
}

/// The member reasons in the model's response, by member id.
///
/// Read from the same JSON array as the lead selections, against the member
/// ids alone, so a candidate's item never becomes a member reason. The first
/// non-empty reason per member wins; anything unparseable yields none and
/// the caller falls back to the rule's own reason.
#[must_use]
pub fn parse_member_reasons<S: BuildHasher>(
    content: &str,
    member_ids: &HashSet<String, S>,
) -> HashMap<String, String> {
    let Some(array) = extract_json_array(content) else {
        return HashMap::new();
    };
    let Ok(raw) = serde_json::from_str::<Vec<RawReason>>(array) else {
        return HashMap::new();
    };
    let mut reasons = HashMap::new();
    for item in raw {
        let reason = item.reason.trim();
        if member_ids.contains(&item.id) && !reason.is_empty() {
            reasons.entry(item.id).or_insert_with(|| reason.to_owned());
        }
    }
    reasons
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::models::DossierFact;

    const FLOOR: u32 = 3;

    fn rules(signals: &TeamSignals<'_>) -> Vec<TeamRule> {
        pick_members(signals, FLOOR)
            .into_iter()
            .map(|pick| pick.rule)
            .collect()
    }

    #[test]
    fn no_signal_picks_nobody() {
        assert_eq!(rules(&TeamSignals::default()), []);
    }

    #[test]
    fn a_goal_picks_fuelling_ticked() {
        let picks = pick_members(
            &TeamSignals {
                goal: true,
                ..TeamSignals::default()
            },
            FLOOR,
        );
        assert_eq!(picks.len(), 1);
        assert_eq!(picks[0].rule, TeamRule::Goal);
        assert_eq!(picks[0].handle(), FUELLING_AGENT_HANDLE);
        assert!(picks[0].preselected());
    }

    #[test]
    fn long_sessions_pick_fuelling_at_the_calibration_threshold() {
        let at = TeamSignals {
            longest_session_min: LONG_SESSION_MIN,
            ..TeamSignals::default()
        };
        assert_eq!(rules(&at), vec![TeamRule::LongSessions]);
        let under = TeamSignals {
            longest_session_min: LONG_SESSION_MIN - 1,
            ..TeamSignals::default()
        };
        assert_eq!(rules(&under), []);
    }

    #[test]
    fn a_goal_and_long_sessions_pick_fuelling_once_for_the_goal() {
        let both = TeamSignals {
            goal: true,
            longest_session_min: 300,
            ..TeamSignals::default()
        };
        assert_eq!(rules(&both), vec![TeamRule::Goal]);
    }

    #[test]
    fn a_sleep_source_picks_recovery_ticked() {
        let picks = pick_members(
            &TeamSignals {
                sleep_source: Some("Garmin"),
                ..TeamSignals::default()
            },
            FLOOR,
        );
        assert_eq!(picks.len(), 1);
        assert_eq!(picks[0].rule, TeamRule::SleepSource);
        assert_eq!(picks[0].handle(), RECOVERY_AGENT_HANDLE);
        assert!(picks[0].preselected());
    }

    #[test]
    fn strength_at_the_floor_offers_mobility_unticked() {
        let picks = pick_members(
            &TeamSignals {
                strength_sessions: FLOOR,
                ..TeamSignals::default()
            },
            FLOOR,
        );
        assert_eq!(picks.len(), 1);
        assert_eq!(picks[0].rule, TeamRule::Strength);
        assert_eq!(picks[0].handle(), MOBILITY_AGENT_HANDLE);
        assert!(!picks[0].preselected(), "Mobility is offered, not ticked");
        let under = TeamSignals {
            strength_sessions: FLOOR - 1,
            ..TeamSignals::default()
        };
        assert_eq!(rules(&under), []);
        let none_with_zero_floor = TeamSignals::default();
        assert_eq!(pick_members(&none_with_zero_floor, 0), []);
    }

    #[test]
    fn every_rule_fires_in_the_fixed_order() {
        let all = TeamSignals {
            goal: true,
            longest_session_min: 200,
            sleep_source: Some("Garmin"),
            strength_sessions: 5,
        };
        let picks = pick_members(&all, FLOOR);
        assert_eq!(
            picks.iter().map(|p| p.rule).collect::<Vec<_>>(),
            vec![TeamRule::Goal, TeamRule::SleepSource, TeamRule::Strength]
        );
        assert_eq!(
            picks.iter().filter(|p| p.preselected()).count(),
            2,
            "at most two members arrive ticked"
        );
    }

    #[test]
    fn pillars_offer_an_agent_only_where_one_answers() {
        assert_eq!(
            agent_for_pillar(Pillar::Fuelling),
            Some(FUELLING_AGENT_HANDLE)
        );
        assert_eq!(
            agent_for_pillar(Pillar::SleepAndRecovery),
            Some(RECOVERY_AGENT_HANDLE)
        );
        assert_eq!(
            agent_for_pillar(Pillar::TrainingAndMovement),
            Some(MOBILITY_AGENT_HANDLE)
        );
        assert_eq!(agent_for_pillar(Pillar::MentalResilience), None);
        assert_eq!(agent_for_pillar(Pillar::CommunityAndConnection), None);
        assert_eq!(agent_for_pillar(Pillar::RecoveryOptimisation), None);
    }

    #[test]
    fn a_cold_start_leads_fuelling_then_recovery_then_mobility_and_never_red_s() {
        let mut handles = [
            Some(RED_S_AGENT_HANDLE),
            Some("endurance-agent"),
            Some(MOBILITY_AGENT_HANDLE),
            None,
            Some(RECOVERY_AGENT_HANDLE),
            Some(FUELLING_AGENT_HANDLE),
        ];
        handles.sort_by_key(|h| cold_start_rank(*h));
        assert_eq!(
            &handles[..3],
            &[
                Some(FUELLING_AGENT_HANDLE),
                Some(RECOVERY_AGENT_HANDLE),
                Some(MOBILITY_AGENT_HANDLE)
            ]
        );
        assert_eq!(handles.last(), Some(&Some(RED_S_AGENT_HANDLE)));
        assert!(!may_lead_cold_start(Some(RED_S_AGENT_HANDLE)));
        assert!(may_lead_cold_start(Some(FUELLING_AGENT_HANDLE)));
        assert!(may_lead_cold_start(None));
    }

    #[test]
    fn rules_serialize_as_the_clients_source_keys() {
        let wire: Vec<String> = [
            TeamRule::Goal,
            TeamRule::LongSessions,
            TeamRule::SleepSource,
            TeamRule::Strength,
        ]
        .iter()
        .map(|rule| {
            serde_json::to_value(rule)
                .unwrap()
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
        assert_eq!(wire, ["goal", "long_sessions", "sleep_source", "strength"]);
    }

    fn fact(kind: &str, object: &str, stale: bool) -> DossierFact {
        DossierFact {
            kind: kind.to_owned(),
            predicate_code: "working_toward".to_owned(),
            object: object.to_owned(),
            confidence: 0.9,
            source: "onboarding".to_owned(),
            updated_at: chrono::Utc::now(),
            valid_until: None,
            stale,
        }
    }

    fn dossier_with(facts: Vec<DossierFact>) -> Dossier {
        let mut dossier = Dossier::empty(uuid::Uuid::nil(), uuid::Uuid::nil());
        dossier.pillars.insert(Pillar::TrainingAndMovement, facts);
        dossier
    }

    #[test]
    fn only_a_live_goal_counts() {
        let live = dossier_with(vec![fact("goal", "A gran fondo in June", false)]);
        assert!(has_goal(&live));
        assert_eq!(first_goal(&live), Some("A gran fondo in June"));
        let stale = dossier_with(vec![fact("goal", "An old marathon", true)]);
        assert!(!has_goal(&stale));
        assert_eq!(first_goal(&stale), None);
        let preference = dossier_with(vec![fact("preference", "Mornings", false)]);
        assert!(!has_goal(&preference));
    }

    #[test]
    fn signals_read_sessions_and_strength_from_the_profile() {
        let profile = SportProfile {
            total_activities: 10,
            window_days: 90,
            sport_counts: HashMap::from([
                ("ride".to_owned(), 5),
                ("strength_training".to_owned(), 2),
                ("crossfit".to_owned(), 1),
            ]),
            longest_session_min: 240,
        };
        let signals = TeamSignals::read(Some(&profile), None, Some("Garmin"));
        assert_eq!(signals.strength_sessions, 3);
        assert_eq!(signals.longest_session_min, 240);
        assert!(!signals.goal);
        assert_eq!(signals.sleep_source, Some("Garmin"));
        assert_eq!(TeamSignals::read(None, None, None), TeamSignals::default());
    }

    #[test]
    fn the_members_section_lists_each_member_and_is_empty_without_one() {
        assert_eq!(members_prompt_section(&[]), "");
        let section = members_prompt_section(&[MemberContext {
            id: "m1".to_owned(),
            title: "Fuelling".to_owned(),
            why: rule_context(TeamRule::Goal),
        }]);
        assert!(section.contains("Team members already chosen"));
        assert!(section.contains("- id: m1"));
        assert!(section.contains("title: Fuelling"));
    }

    #[test]
    fn member_reasons_are_read_against_member_ids_only() {
        let members: HashSet<String> = HashSet::from(["m1".to_owned(), "m2".to_owned()]);
        let content = r#"Sure: [
            {"id": "lead", "reason": "Fits your riding."},
            {"id": "m1", "reason": "  Plans what you eat.  "},
            {"id": "m1", "reason": "A second try."},
            {"id": "m2", "reason": "   "},
            {"id": "ghost", "reason": "Hallucinated."}
        ]"#;
        let reasons = parse_member_reasons(content, &members);
        assert_eq!(reasons.len(), 1);
        assert_eq!(reasons["m1"], "Plans what you eat.");
        assert!(parse_member_reasons("no json here", &members).is_empty());
        assert!(parse_member_reasons("[not json]", &members).is_empty());
    }
}
