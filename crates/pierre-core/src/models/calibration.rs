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

use super::onboarding::{TopicSlug, TopicVisibility, WalkAudience};
use super::Pillar;

/// One question in the calibration interview.
///
/// The variants are ordered as asked. Each carries a probe hint the agent
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

    /// What the agent should explore this turn, phrased naturally.
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
    /// agent-visible `injury` kind (distinct from the redacted `medical`);
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

    /// Whether this topic may be probed in a shared room the athlete chose
    /// to calibrate in.
    ///
    /// Every current topic is room-safe: calibration probes training capacity
    /// — load appetite, availability, recovery, what flares under load — the
    /// working vocabulary of any shared ride, and the athlete typing
    /// `/calibrate` in the room chose that room. Declared per topic all the
    /// same, so a future sensitive topic must decide rather than inherit.
    #[must_use]
    pub const fn visibility(self) -> TopicVisibility {
        match self {
            Self::ProgressionIntent
            | Self::BaselineConfirm
            | Self::Availability
            | Self::Injury
            | Self::RpeHeadroom
            | Self::RecoverySpeed
            | Self::Fueling
            | Self::EventDemand => TopicVisibility::RoomSafe,
        }
    }

    /// The next topic to probe: the first in ask order, audible to the walk's
    /// audience, that has not been delivered.
    ///
    /// Unlike the pillars walk there is no re-ask budget. Coverage cannot
    /// distinguish these topics from one another, so a second attempt could
    /// only be spent blindly; an answer that yielded no fact is caught by the
    /// completion check instead, which re-opens the safety-critical ones by
    /// name. `None` means every topic the audience may hear has been asked.
    #[must_use]
    pub fn next_target(
        probed: &[TopicSlug],
        conditions: CalibrationConditions,
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
