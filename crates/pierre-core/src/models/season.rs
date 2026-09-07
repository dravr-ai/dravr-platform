// ABOUTME: The /season walk's topic table and next-topic policy — the calendar and what it demands
// ABOUTME: Fixed topic list (core + one conditional), each with its probe hint, fact kind and room visibility
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Season-intake topics.
//!
//! `/season` captures what the flavour rule and the season layout need and no
//! other flow asks: the race calendar and which event actually matters, what
//! "good" looks like on two horizons, the best performances the athlete can
//! point at, how long they have been in the sport, what they can steer
//! intensity by, and what they want from coaching. Availability, injury and
//! recovery are `/calibrate`'s and are quoted back, never re-asked.
//!
//! Like calibration the order is fixed and the completion authority is the
//! delivered-probe ledger: several topics land as the same kind in the same
//! pillar, so Dossier coverage could never say which are outstanding.
//!
//! Order matters — the calendar is asked first because every later answer is
//! read against the event it points at.

use serde::{Deserialize, Serialize};

use super::onboarding::{TopicSlug, TopicVisibility, WalkAudience};
use super::Pillar;

/// One question in the season walk.
///
/// The variants are ordered as asked. Each carries a probe hint the coach
/// phrases naturally (never read verbatim — same contract as the pillars walk)
/// and the fact kind its answer is stamped with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeasonTopic {
    /// The events they are pointing at, when, and which one actually matters.
    /// Asked first: every later answer is read against it. The turn also
    /// quotes back `/calibrate`'s availability for correction.
    RaceCalendar,
    /// What "good" looks like this season versus in a few years — two
    /// horizons, two facts, so a later edit to one never clobbers the other.
    GoalHorizon,
    /// Best times or finishes and how recent they are.
    PerformanceBaseline,
    /// How long in the sport, and what they came from.
    Background,
    /// What they can steer intensity by — lactate meter, power meter, HR
    /// strap, pace, or effort alone. The one input no other flow asks, and
    /// the lactate-guided flavours are ineligible without it.
    MeasurementTools,
    /// Prior coaching, what worked and what did not, and what they want from
    /// coaching now. Asked of everyone: "never had one" is a one-line answer,
    /// and the athlete who has had one is exactly who has an opinion.
    CoachingFit,
    /// Pool, gym and trainer access — days and durations. Conditional — see
    /// [`SeasonConditions`].
    FacilityAccess,
}

/// Raw observations about the athlete that decide which conditional topics
/// they are asked. Policy lives in [`SeasonTopic::for_conditions`]; these are
/// the inputs, each meaning exactly what it says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SeasonConditions {
    /// The recent window holds sessions in more than one sport family
    /// (running, cycling, swimming, other), so facility access — pool days,
    /// trainer time — shapes what a week can hold.
    pub multi_sport: bool,
}

impl SeasonTopic {
    /// The six topics every athlete is asked, in ask order.
    ///
    /// Six is the same ceiling calibration set: the seven-topic pillars walk
    /// proved long guided flows shed athletes mid-way, and availability,
    /// injury and recovery already have a home there.
    pub const CORE: [Self; 6] = [
        Self::RaceCalendar,
        Self::GoalHorizon,
        Self::PerformanceBaseline,
        Self::Background,
        Self::MeasurementTools,
        Self::CoachingFit,
    ];

    /// Every topic, core then conditional, in ask order.
    pub const ALL: [Self; 7] = [
        Self::RaceCalendar,
        Self::GoalHorizon,
        Self::PerformanceBaseline,
        Self::Background,
        Self::MeasurementTools,
        Self::CoachingFit,
        Self::FacilityAccess,
    ];

    /// Stable identifier recorded in the delivered-probe ledger.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RaceCalendar => "season_race_calendar",
            Self::GoalHorizon => "season_goal_horizon",
            Self::PerformanceBaseline => "season_performance_baseline",
            Self::Background => "season_background",
            Self::MeasurementTools => "season_measurement_tools",
            Self::CoachingFit => "season_coaching_fit",
            Self::FacilityAccess => "season_facility_access",
        }
    }

    /// The slug as persisted in [`super::OnboardingState::probed`].
    ///
    /// Prefixed so a season entry can never collide with a pillar or
    /// calibration slug in the shared ledger.
    #[must_use]
    pub fn slug(self) -> TopicSlug {
        TopicSlug::new(self.as_str().to_owned())
    }

    /// Parse from a ledger slug. `None` for anything that is not a season
    /// topic — a pillar slug, a calibration slug, or a topic from a later build.
    #[must_use]
    pub fn parse(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == slug)
    }

    /// What the coach should explore this turn, phrased naturally.
    #[must_use]
    pub const fn probe_hint(self) -> &'static str {
        match self {
            Self::RaceCalendar => {
                "the events on their calendar — which ones, roughly when, and which single one \
                 actually matters this season; the rest are training days with a number on"
            }
            Self::GoalHorizon => {
                "what a good season would look like to them — a time, a finish, a feeling — and, \
                 separately, where they want to be in a few years; keep the two apart"
            }
            Self::PerformanceBaseline => {
                "the best they have done recently — a race time, a finish, a benchmark effort — \
                 and how long ago it was, because a ten-year-old best is history, not a baseline"
            }
            Self::Background => {
                "how many years they have trained with any structure, and what they came from — \
                 another sport, a long break, nothing at all"
            }
            Self::MeasurementTools => {
                "what they can steer a hard session by — a lactate meter, a power meter, a heart \
                 rate strap, pace on a watch, or feel alone — and which of those they actually use"
            }
            Self::CoachingFit => {
                "whether they have worked with a coach before and, if so, what worked and what \
                 did not; either way, what they want from coaching now"
            }
            Self::FacilityAccess => {
                "which days they can get to a pool, a gym or a trainer, and for how long — the \
                 constraints that decide what a multi-sport week can hold"
            }
        }
    }

    /// The fact kind this topic's answer is stamped with, or `None` where the
    /// extractor chooses.
    ///
    /// The calendar turn is the one left open: it also carries the quoted-back
    /// availability for correction, and a correction is a `schedule` fact
    /// while the races are `goal` facts — forcing either would mis-file the
    /// other. A best performance is a `physiology` baseline the same way FTP
    /// is; tools and facilities are `equipment`; background and coaching fit
    /// are `preference`.
    ///
    /// Returned as the string form so this module stays free of a
    /// `pierre-memory` dependency, exactly as [`super::CalibrationTopic`] does.
    #[must_use]
    pub const fn fact_kind(self) -> Option<&'static str> {
        match self {
            Self::RaceCalendar => None,
            Self::GoalHorizon => Some("goal"),
            Self::PerformanceBaseline => Some("physiology"),
            Self::Background | Self::CoachingFit => Some("preference"),
            Self::MeasurementTools | Self::FacilityAccess => Some("equipment"),
        }
    }

    /// The kind whose presence among the landed facts counts this topic as
    /// answered. For the calendar that is a goal — the races — whichever
    /// other kinds the turn also produced.
    #[must_use]
    pub const fn landed_kind(self) -> &'static str {
        match self.fact_kind() {
            Some(kind) => kind,
            None => "goal",
        }
    }

    /// Every season answer belongs to the training pillar.
    #[must_use]
    pub const fn pillar(self) -> Pillar {
        Pillar::TrainingAndMovement
    }

    /// The topics this athlete will be asked, core plus whichever conditionals
    /// they qualify for.
    #[must_use]
    pub fn for_conditions(conditions: SeasonConditions) -> Vec<Self> {
        let mut topics = Self::CORE.to_vec();
        if conditions.multi_sport {
            topics.push(Self::FacilityAccess);
        }
        topics
    }

    /// Whether this topic may be probed in a shared room the athlete chose
    /// to run the walk in.
    ///
    /// Races, goals, bests, background, tools and facilities are the working
    /// vocabulary of any shared ride. Coaching fit is not: "what my last
    /// coach got wrong" is said to a coach alone, so a room walk skips it.
    #[must_use]
    pub const fn visibility(self) -> TopicVisibility {
        match self {
            Self::RaceCalendar
            | Self::GoalHorizon
            | Self::PerformanceBaseline
            | Self::Background
            | Self::MeasurementTools
            | Self::FacilityAccess => TopicVisibility::RoomSafe,
            Self::CoachingFit => TopicVisibility::DmOnly,
        }
    }

    /// The next topic to probe: the first in ask order, audible to the walk's
    /// audience, that has not been delivered. `None` means every topic the
    /// audience may hear has been asked. No re-ask budget, for calibration's
    /// reason: coverage cannot tell these topics apart.
    #[must_use]
    pub fn next_target(
        probed: &[TopicSlug],
        conditions: SeasonConditions,
        audience: WalkAudience,
    ) -> Option<Self> {
        Self::for_conditions(conditions)
            .into_iter()
            .filter(|topic| match audience {
                WalkAudience::Private => true,
                WalkAudience::Room => topic.visibility() == TopicVisibility::RoomSafe,
            })
            .find(|topic| !probed.iter().any(|slug| slug.as_str() == topic.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::{SeasonConditions, SeasonTopic};
    use crate::models::onboarding::{TopicSlug, TopicVisibility, WalkAudience};

    fn probed(topics: &[SeasonTopic]) -> Vec<TopicSlug> {
        topics.iter().map(|t| t.slug()).collect()
    }

    #[test]
    fn the_calendar_is_asked_first() {
        assert_eq!(
            SeasonTopic::next_target(&[], SeasonConditions::default(), WalkAudience::Private),
            Some(SeasonTopic::RaceCalendar),
            "every later answer is read against the event it points at"
        );
    }

    #[test]
    fn topics_advance_in_ask_order_and_terminate() {
        let conditions = SeasonConditions::default();
        let mut history = Vec::new();
        for expected in SeasonTopic::CORE {
            assert_eq!(
                SeasonTopic::next_target(&history, conditions, WalkAudience::Private),
                Some(expected)
            );
            history.push(expected.slug());
        }
        assert_eq!(
            SeasonTopic::next_target(&history, conditions, WalkAudience::Private),
            None,
            "the walk ends after its six core topics — there is no re-ask budget"
        );
    }

    #[test]
    fn facility_access_is_asked_only_of_a_multi_sport_athlete() {
        let single = SeasonTopic::for_conditions(SeasonConditions::default());
        assert_eq!(single.len(), 6);
        assert!(!single.contains(&SeasonTopic::FacilityAccess));

        let multi = SeasonTopic::for_conditions(SeasonConditions { multi_sport: true });
        assert_eq!(multi, SeasonTopic::ALL.to_vec());
    }

    #[test]
    fn a_qualifying_conditional_is_still_asked_after_the_core_six() {
        let history = probed(&SeasonTopic::CORE);
        assert_eq!(
            SeasonTopic::next_target(
                &history,
                SeasonConditions { multi_sport: true },
                WalkAudience::Private
            ),
            Some(SeasonTopic::FacilityAccess)
        );
    }

    #[test]
    fn slugs_are_prefixed_so_they_never_collide_with_another_flow() {
        for topic in SeasonTopic::ALL {
            assert!(
                topic.as_str().starts_with("season_"),
                "{} shares the ledger with pillar and calibration slugs",
                topic.as_str()
            );
            assert_eq!(SeasonTopic::parse(topic.as_str()), Some(topic));
        }
        assert_eq!(SeasonTopic::parse("calibration_availability"), None);
        assert_eq!(SeasonTopic::parse("training_and_movement"), None);
    }

    #[test]
    fn a_foreign_slug_in_the_ledger_does_not_satisfy_a_season_topic() {
        let stale = vec![
            TopicSlug::new("north_star".to_owned()),
            TopicSlug::new("calibration_availability".to_owned()),
            TopicSlug::new("fuelling".to_owned()),
        ];
        assert_eq!(
            SeasonTopic::next_target(&stale, SeasonConditions::default(), WalkAudience::Private),
            Some(SeasonTopic::RaceCalendar)
        );
    }

    #[test]
    fn the_calendar_turn_leaves_the_kind_to_the_extractor() {
        // It carries the quoted-back availability too: a correction is a
        // schedule fact while the races are goals, so forcing either would
        // mis-file the other.
        assert_eq!(SeasonTopic::RaceCalendar.fact_kind(), None);
        assert_eq!(SeasonTopic::RaceCalendar.landed_kind(), "goal");
        assert_eq!(SeasonTopic::GoalHorizon.fact_kind(), Some("goal"));
        assert_eq!(
            SeasonTopic::PerformanceBaseline.fact_kind(),
            Some("physiology")
        );
        assert_eq!(SeasonTopic::MeasurementTools.fact_kind(), Some("equipment"));
        assert_eq!(SeasonTopic::FacilityAccess.fact_kind(), Some("equipment"));
        assert_eq!(SeasonTopic::Background.fact_kind(), Some("preference"));
        assert_eq!(SeasonTopic::CoachingFit.fact_kind(), Some("preference"));
    }

    #[test]
    fn every_topic_has_a_usable_probe_hint() {
        for topic in SeasonTopic::ALL {
            let hint = topic.probe_hint();
            assert!(
                hint.len() > 40,
                "{} has no usable probe hint",
                topic.as_str()
            );
            assert!(
                !hint.ends_with('?'),
                "{} reads as a verbatim question; hints describe what to explore",
                topic.as_str()
            );
        }
    }

    #[test]
    fn a_room_walk_skips_coaching_fit_and_nothing_else() {
        let private: Vec<_> = SeasonTopic::for_conditions(SeasonConditions { multi_sport: true });
        let mut history = Vec::new();
        let mut asked_in_room = Vec::new();
        while let Some(next) = SeasonTopic::next_target(
            &history,
            SeasonConditions { multi_sport: true },
            WalkAudience::Room,
        ) {
            asked_in_room.push(next);
            history.push(next.slug());
        }
        let expected: Vec<_> = private
            .into_iter()
            .filter(|t| *t != SeasonTopic::CoachingFit)
            .collect();
        assert_eq!(asked_in_room, expected);
        assert_eq!(
            SeasonTopic::CoachingFit.visibility(),
            TopicVisibility::DmOnly
        );
    }
}
