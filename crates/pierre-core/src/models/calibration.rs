// ABOUTME: The difficulty-calibration interview's topic table and next-topic policy
// ABOUTME: Fixed topic list (core + conditionals), each with its probe hint and fact kind
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Difficulty-calibration interview topics.
//!
//! The interview asks a short, ordered set of questions whose answers land as
//! durable facts that steer later plan generation. Unlike the pillars walk,
//! whose next topic is derived from live Dossier coverage, calibration's topic
//! order is fixed and its completion authority is the delivered-probe ledger:
//! seven of its topics land as [`FactKind::Preference`] in the
//! [`Pillar::TrainingAndMovement`] bucket, so Dossier coverage cannot tell them
//! apart and could never report which ones are still outstanding.
//!
//! Order matters — progression intent is asked first because every later answer
//! is interpreted against it.

use serde::{Deserialize, Serialize};

use super::onboarding::TopicSlug;
use super::Pillar;

/// One question in the calibration interview.
///
/// The variants are ordered as asked. Each carries a probe hint the coach
/// phrases naturally (never read verbatim — same contract as the pillars walk)
/// and the fact kind its answer is stamped with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CalibrationTopic {
    /// How the athlete wants "harder" to be expressed. Asked first: ramping one
    /// knob at a time is what keeps the other answers safe to act on.
    ProgressionIntent,
    /// Confirm or correct the inferred recent-load baseline.
    BaselineConfirm,
    /// Hours realistically available and which days are protected.
    Availability,
    /// Anything in the last 12 months that flares under load.
    Injury,
    /// Whether the last few hard sessions left headroom.
    RpeHeadroom,
    /// How long recovery from a hard day actually takes.
    RecoverySpeed,
    /// Carbohydrate intake on long sessions. Conditional — see
    /// [`CalibrationConditions`].
    Fueling,
    /// What the goal event demands. Conditional — see
    /// [`CalibrationConditions`].
    EventDemand,
}

/// Raw observations about the athlete that decide which conditional topics
/// they are asked. Policy lives in [`CalibrationTopic::for_conditions`]; these
/// are the inputs, each meaning exactly what it says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CalibrationConditions {
    /// The recent window holds at least one session over three hours.
    pub long_sessions: bool,
    /// A goal fact exists, so "what does the event demand" has a referent.
    pub dated_goal: bool,
}

impl CalibrationTopic {
    /// The six topics every athlete is asked, in ask order.
    ///
    /// The interview is deliberately shorter than the research set: the
    /// seven-topic pillars walk already proved long guided flows shed athletes
    /// mid-way, and these six are the ones that most change a generated plan.
    /// The rest moved to the conditionals below or to a later phase.
    pub const CORE: [Self; 6] = [
        Self::ProgressionIntent,
        Self::BaselineConfirm,
        Self::Availability,
        Self::Injury,
        Self::RpeHeadroom,
        Self::RecoverySpeed,
    ];

    /// Every topic, core then conditional, in ask order.
    pub const ALL: [Self; 8] = [
        Self::ProgressionIntent,
        Self::BaselineConfirm,
        Self::Availability,
        Self::Injury,
        Self::RpeHeadroom,
        Self::RecoverySpeed,
        Self::Fueling,
        Self::EventDemand,
    ];

    /// Stable identifier recorded in the delivered-probe ledger.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProgressionIntent => "calibration_progression_intent",
            Self::BaselineConfirm => "calibration_baseline_confirm",
            Self::Availability => "calibration_availability",
            Self::Injury => "calibration_injury",
            Self::RpeHeadroom => "calibration_rpe_headroom",
            Self::RecoverySpeed => "calibration_recovery_speed",
            Self::Fueling => "calibration_fueling",
            Self::EventDemand => "calibration_event_demand",
        }
    }

    /// The slug as persisted in [`super::OnboardingState::probed`].
    ///
    /// Prefixed so a calibration entry can never collide with a pillar slug in
    /// the shared ledger.
    #[must_use]
    pub fn slug(self) -> TopicSlug {
        TopicSlug::new(self.as_str().to_owned())
    }

    /// Parse from a ledger slug. `None` for anything that is not a calibration
    /// topic — a pillar slug, or a topic from a later build.
    #[must_use]
    pub fn parse(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.as_str() == slug)
    }

    /// What the coach should explore this turn, phrased naturally.
    #[must_use]
    pub const fn probe_hint(self) -> &'static str {
        match self {
            Self::ProgressionIntent => {
                "which single lever they want turned up — more hours, more hard days, or longer \
                 long sessions — and make clear you will ramp one at a time, not all three"
            }
            Self::BaselineConfirm => {
                "whether the recent-load figures you just quoted are a fair baseline or a \
                 disrupted stretch (illness, travel, a break)"
            }
            Self::Availability => {
                "the hours they can realistically give in a normal week, and which days are \
                 protected or immovable"
            }
            Self::Injury => {
                "anything in the last twelve months that flares when they push — which sport, \
                 and what brings it on"
            }
            Self::RpeHeadroom => {
                "their last few hard sessions: could they have done one more interval set, or \
                 were they done"
            }
            Self::RecoverySpeed => {
                "how they feel the morning after a hard day — ready to go again, or needing one \
                 or two easy days first"
            }
            Self::Fueling => {
                "what they actually take in on a long session — grams of carbohydrate per hour, \
                 honestly, not what they intend to"
            }
            Self::EventDemand => {
                "what their goal event actually demands — long steady efforts, punchy climbs, or \
                 all-day rough terrain"
            }
        }
    }

    /// The fact kind this topic's answer is stamped with.
    ///
    /// Availability is a schedule constraint; the injury question uses the
    /// coach-visible `injury` kind (distinct from the redacted `medical`);
    /// event demand refines a goal; the rest are preferences.
    ///
    /// Recovery speed is `physiology` for two reasons that agree. "I need two
    /// easy days after a hard session" is a training-state claim about recovery
    /// capacity, not a taste — which is what `physiology` means. And being the
    /// only topic that writes that kind is what lets the completion check
    /// notice when its answer never landed: the other safety-critical topic
    /// (injury) owns `injury` the same way, whereas anything filed under
    /// `preference` is indistinguishable from its five siblings and from every
    /// preference the chat extractor ever wrote.
    ///
    /// Returned as the string form so this module stays free of a
    /// `pierre-memory` dependency — [`super::Pillar`] is already local. A
    /// pipeline test asserts every value here resolves to a real `FactKind`
    /// rather than falling through to `Other`.
    #[must_use]
    pub const fn fact_kind(self) -> &'static str {
        match self {
            Self::Availability => "schedule",
            Self::Injury => "injury",
            Self::EventDemand => "goal",
            Self::RecoverySpeed => "physiology",
            Self::ProgressionIntent | Self::BaselineConfirm | Self::RpeHeadroom | Self::Fueling => {
                "preference"
            }
        }
    }

    /// Every calibration answer belongs to the training pillar.
    #[must_use]
    pub const fn pillar(self) -> Pillar {
        Pillar::TrainingAndMovement
    }

    /// Whether a missing fact for this topic should re-open the interview
    /// rather than let it report success.
    ///
    /// Injury and recovery speed bound how hard a plan may get. A calibration
    /// that captured the athlete's appetite for more load but not the two
    /// answers that constrain it is worse than none, because the facts it did
    /// land all point one way.
    #[must_use]
    pub const fn is_safety_critical(self) -> bool {
        matches!(self, Self::Injury | Self::RecoverySpeed)
    }

    /// The topics this athlete will be asked, core plus whichever conditionals
    /// they qualify for.
    ///
    /// Fueling qualifies on *either* signal: long sessions already in the
    /// history, or a goal event to prepare for. Under-fuelling shows up as
    /// under-recovery, so an athlete building toward a long event needs the
    /// question before their sessions get long, not after.
    #[must_use]
    pub fn for_conditions(conditions: CalibrationConditions) -> Vec<Self> {
        let mut topics = Self::CORE.to_vec();
        if conditions.long_sessions || conditions.dated_goal {
            topics.push(Self::Fueling);
        }
        if conditions.dated_goal {
            topics.push(Self::EventDemand);
        }
        topics
    }

    /// The next topic to probe: the first in ask order that has not been
    /// delivered.
    ///
    /// Unlike the pillars walk there is no re-ask budget. Coverage cannot
    /// distinguish these topics from one another, so a second attempt could
    /// only be spent blindly; an answer that yielded no fact is caught by the
    /// completion check instead, which re-opens the safety-critical ones by
    /// name. `None` means every topic has been asked.
    #[must_use]
    pub fn next_target(probed: &[TopicSlug], conditions: CalibrationConditions) -> Option<Self> {
        Self::for_conditions(conditions)
            .into_iter()
            .find(|topic| !probed.iter().any(|slug| slug.as_str() == topic.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::{CalibrationConditions, CalibrationTopic};
    use crate::models::onboarding::TopicSlug;

    fn probed(topics: &[CalibrationTopic]) -> Vec<TopicSlug> {
        topics.iter().map(|t| t.slug()).collect()
    }

    #[test]
    fn intent_is_asked_first() {
        assert_eq!(
            CalibrationTopic::next_target(&[], CalibrationConditions::default()),
            Some(CalibrationTopic::ProgressionIntent),
            "every later answer is interpreted against the progression intent"
        );
    }

    #[test]
    fn topics_advance_in_ask_order_and_terminate() {
        let conditions = CalibrationConditions::default();
        let mut history = Vec::new();
        for expected in CalibrationTopic::CORE {
            assert_eq!(
                CalibrationTopic::next_target(&history, conditions),
                Some(expected)
            );
            history.push(expected.slug());
        }
        assert_eq!(
            CalibrationTopic::next_target(&history, conditions),
            None,
            "the core interview ends after its six topics — there is no re-ask budget"
        );
    }

    #[test]
    fn conditionals_only_appear_when_they_apply() {
        let core = CalibrationTopic::for_conditions(CalibrationConditions::default());
        assert_eq!(core.len(), 6);
        assert!(!core.contains(&CalibrationTopic::Fueling));

        let fueling_only = CalibrationTopic::for_conditions(CalibrationConditions {
            long_sessions: true,
            dated_goal: false,
        });
        assert_eq!(fueling_only.len(), 7);
        assert!(fueling_only.contains(&CalibrationTopic::Fueling));
        assert!(!fueling_only.contains(&CalibrationTopic::EventDemand));

        let both = CalibrationTopic::for_conditions(CalibrationConditions {
            long_sessions: true,
            dated_goal: true,
        });
        assert_eq!(both, CalibrationTopic::ALL.to_vec());
    }

    #[test]
    fn a_dated_goal_alone_earns_the_fueling_question() {
        // Either signal qualifies: an athlete building toward an event needs
        // the fueling answer before their sessions get long, not after.
        let goal_only = CalibrationTopic::for_conditions(CalibrationConditions {
            long_sessions: false,
            dated_goal: true,
        });
        assert!(goal_only.contains(&CalibrationTopic::Fueling));
        assert!(goal_only.contains(&CalibrationTopic::EventDemand));
        assert_eq!(goal_only.len(), 8);
    }

    #[test]
    fn a_qualifying_conditional_is_still_asked_after_the_core_six() {
        let conditions = CalibrationConditions {
            long_sessions: true,
            dated_goal: false,
        };
        let history = probed(&CalibrationTopic::CORE);
        assert_eq!(
            CalibrationTopic::next_target(&history, conditions),
            Some(CalibrationTopic::Fueling),
            "the core six are done but this athlete still owes the fueling answer"
        );
    }

    #[test]
    fn slugs_are_prefixed_so_they_never_collide_with_a_pillar() {
        for topic in CalibrationTopic::ALL {
            assert!(
                topic.as_str().starts_with("calibration_"),
                "{} shares the ledger with pillar slugs",
                topic.as_str()
            );
            assert_eq!(CalibrationTopic::parse(topic.as_str()), Some(topic));
        }
        assert_eq!(CalibrationTopic::parse("fuelling"), None);
        assert_eq!(CalibrationTopic::parse("north_star"), None);
    }

    #[test]
    fn a_pillar_slug_in_the_ledger_does_not_satisfy_a_calibration_topic() {
        // A conversation that ran a pillars walk before calibrating carries
        // pillar slugs in the shared ledger; none of them may mark a
        // calibration topic delivered.
        let stale = vec![
            TopicSlug::new("north_star".to_owned()),
            TopicSlug::new("training_and_movement".to_owned()),
            TopicSlug::new("fuelling".to_owned()),
        ];
        assert_eq!(
            CalibrationTopic::next_target(&stale, CalibrationConditions::default()),
            Some(CalibrationTopic::ProgressionIntent)
        );
    }

    #[test]
    fn safety_critical_topics_are_the_two_that_bound_load() {
        let critical: Vec<_> = CalibrationTopic::ALL
            .into_iter()
            .filter(|t| t.is_safety_critical())
            .collect();
        assert_eq!(
            critical,
            vec![CalibrationTopic::Injury, CalibrationTopic::RecoverySpeed]
        );
    }

    #[test]
    fn fact_kinds_match_the_topic_semantics() {
        assert_eq!(CalibrationTopic::Availability.fact_kind(), "schedule");
        assert_eq!(CalibrationTopic::Injury.fact_kind(), "injury");
        assert_eq!(CalibrationTopic::EventDemand.fact_kind(), "goal");
        assert_eq!(CalibrationTopic::RecoverySpeed.fact_kind(), "physiology");
        assert_eq!(
            CalibrationTopic::ProgressionIntent.fact_kind(),
            "preference"
        );
    }

    #[test]
    fn each_safety_critical_topic_owns_its_fact_kind_alone() {
        // The completion check notices a missing safety answer by looking for
        // that topic's kind among the facts the interview landed. That only
        // works while no other topic writes the same kind — otherwise a
        // sibling's answer would mask the gap and the interview would report
        // success having never learned whether the athlete is injured.
        for critical in CalibrationTopic::ALL
            .into_iter()
            .filter(|t| t.is_safety_critical())
        {
            let sharers: Vec<_> = CalibrationTopic::ALL
                .into_iter()
                .filter(|t| *t != critical && t.fact_kind() == critical.fact_kind())
                .collect();
            assert!(
                sharers.is_empty(),
                "{} shares kind '{}' with {sharers:?}, so its absence cannot be detected",
                critical.as_str(),
                critical.fact_kind()
            );
        }
    }

    #[test]
    fn every_topic_has_a_usable_probe_hint() {
        for topic in CalibrationTopic::ALL {
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
}
