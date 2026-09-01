// ABOUTME: Pillar — the canonical six fitness-adapted health dimensions Dravr coaches across
// ABOUTME: Single source of truth for per-user context, pillar-tagged facts, and the OKF bundle
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt;

use serde::{Deserialize, Serialize};

use super::onboarding::TopicVisibility;

/// One of the six fitness-adapted health dimensions Dravr coaches across.
///
/// This is the single source of truth for the pillar taxonomy: it backs the
/// `pillar` column on `user_facts`, the per-user context projection on `Dossier`,
/// and the rendered OKF bundle. Mobility folds into [`Pillar::TrainingAndMovement`];
/// it is a facet of training, not a seventh pillar. The marketplace `CoachCategory`
/// (which still carries a `mobility` value) is a deliberately separate taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pillar {
    /// Training & Movement — workouts, load, and mobility/flexibility.
    TrainingAndMovement,
    /// Fuelling — nutrition and hydration.
    Fuelling,
    /// Sleep & Recovery — sleep habits and rest.
    SleepAndRecovery,
    /// Mental Resilience — stress and psychological load.
    MentalResilience,
    /// Community & Connection — social support.
    CommunityAndConnection,
    /// Recovery Optimisation — substance use, framed around performance/recovery.
    RecoveryOptimisation,
}

impl Pillar {
    /// Every pillar, in canonical order, for iteration and enumeration endpoints.
    pub const ALL: [Self; 6] = [
        Self::TrainingAndMovement,
        Self::Fuelling,
        Self::SleepAndRecovery,
        Self::MentalResilience,
        Self::CommunityAndConnection,
        Self::RecoveryOptimisation,
    ];

    /// Short stable string identifier used in DB serialization and OKF frontmatter.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TrainingAndMovement => "training_and_movement",
            Self::Fuelling => "fuelling",
            Self::SleepAndRecovery => "sleep_and_recovery",
            Self::MentalResilience => "mental_resilience",
            Self::CommunityAndConnection => "community_and_connection",
            Self::RecoveryOptimisation => "recovery_optimisation",
        }
    }

    /// Human-readable display label.
    #[must_use]
    pub const fn display_label(self) -> &'static str {
        match self {
            Self::TrainingAndMovement => "Training & Movement",
            Self::Fuelling => "Fuelling",
            Self::SleepAndRecovery => "Sleep & Recovery",
            Self::MentalResilience => "Mental Resilience",
            Self::CommunityAndConnection => "Community & Connection",
            Self::RecoveryOptimisation => "Recovery Optimisation",
        }
    }

    /// Short, non-clinical hint guiding the onboarding coach on what to explore
    /// for this pillar. Steers the conversational probe; not shown verbatim.
    #[must_use]
    pub const fn probe_hint(self) -> &'static str {
        match self {
            Self::TrainingAndMovement => {
                "how they train and move — sports, weekly volume, mobility, what they enjoy or avoid"
            }
            Self::Fuelling => {
                "how they eat and hydrate — typical fuelling, dietary preferences or restrictions"
            }
            Self::SleepAndRecovery => {
                "their sleep and rest — typical hours, quality, how they recover between sessions"
            }
            Self::MentalResilience => {
                "stress and headspace — what drives stress, how they cope, mindset around training"
            }
            Self::CommunityAndConnection => {
                "their support network — who they train with, accountability, social motivation"
            }
            Self::RecoveryOptimisation => {
                "recovery habits framed around performance — alcohol/substances, non-judgementally"
            }
        }
    }

    /// Whether this pillar may be probed in a shared room the athlete chose
    /// to walk in.
    ///
    /// Mental Resilience probes stress and psychological load; Recovery
    /// Optimisation probes substance use. Both stay between the athlete and
    /// the coach — a room walk never surfaces them, and `/pillars` in a room
    /// points the athlete at a direct message for the two it leaves out.
    #[must_use]
    pub const fn visibility(self) -> TopicVisibility {
        match self {
            Self::TrainingAndMovement
            | Self::Fuelling
            | Self::SleepAndRecovery
            | Self::CommunityAndConnection => TopicVisibility::RoomSafe,
            Self::MentalResilience | Self::RecoveryOptimisation => TopicVisibility::DmOnly,
        }
    }

    /// Parse from the DB string form. Returns `None` on unknown values — the
    /// taxonomy is closed, so the legacy `activity`/`nutrition`/`recovery` slugs
    /// are intentionally rejected.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "training_and_movement" => Some(Self::TrainingAndMovement),
            "fuelling" => Some(Self::Fuelling),
            "sleep_and_recovery" => Some(Self::SleepAndRecovery),
            "mental_resilience" => Some(Self::MentalResilience),
            "community_and_connection" => Some(Self::CommunityAndConnection),
            "recovery_optimisation" => Some(Self::RecoveryOptimisation),
            _ => None,
        }
    }
}

impl fmt::Display for Pillar {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::Pillar;

    #[test]
    fn roundtrip_str() {
        for pillar in Pillar::ALL {
            assert_eq!(Pillar::parse(pillar.as_str()), Some(pillar));
        }
    }

    #[test]
    fn unknown_returns_none() {
        assert!(Pillar::parse("activity").is_none());
        assert!(Pillar::parse("nutrition").is_none());
        assert!(Pillar::parse("recovery").is_none());
        assert!(Pillar::parse("mobility").is_none());
    }

    #[test]
    fn serde_wire_form_is_snake_case() {
        assert_eq!(
            serde_json::to_string(&Pillar::TrainingAndMovement).unwrap_or_default(),
            "\"training_and_movement\""
        );
        assert_eq!(
            serde_json::to_string(&Pillar::RecoveryOptimisation).unwrap_or_default(),
            "\"recovery_optimisation\""
        );
    }
}
