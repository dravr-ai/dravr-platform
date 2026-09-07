// ABOUTME: The coarse sport family a provider sport type belongs to — running, cycling, swimming, other
// ABOUTME: One mapping, read by the season walk's multi-sport condition and the flavour rule's sport mix
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::SportType;

/// The family a session belongs to, for questions that care whether an
/// athlete trains one discipline or several — not for anything that needs
/// the discipline itself.
///
/// Virtual and terrain variants fold into their family: a trainer ride is
/// cycling, a trail run is running. Everything without a family of its own
/// — walking, hiking, gym work, snow sports — is `Other`, so a runner who
/// also hikes is still single-sport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SportFamily {
    /// Running, on any surface, real or virtual.
    Running,
    /// Cycling, on any bike, real or virtual.
    Cycling,
    /// Swimming.
    Swimming,
    /// Anything without a family of its own.
    Other,
}

impl SportFamily {
    /// The family of a provider sport type.
    #[must_use]
    pub const fn of(sport: &SportType) -> Self {
        match sport {
            SportType::Run | SportType::VirtualRun => Self::Running,
            SportType::Ride
            | SportType::VirtualRide
            | SportType::EbikeRide
            | SportType::MountainBike
            | SportType::GravelRide => Self::Cycling,
            SportType::Swim => Self::Swimming,
            _ => Self::Other,
        }
    }

    /// How many distinct families a set of sessions spans. `Other` counts as
    /// a family of its own once, so run-plus-gym is two — the gym days still
    /// shape what a week can hold.
    #[must_use]
    pub fn distinct<'a>(sports: impl IntoIterator<Item = &'a SportType>) -> u32 {
        let mut seen: Vec<Self> = Vec::with_capacity(4);
        for sport in sports {
            let family = Self::of(sport);
            if !seen.contains(&family) {
                seen.push(family);
            }
        }
        u32::try_from(seen.len()).unwrap_or(u32::MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::SportFamily;
    use crate::models::SportType;

    #[test]
    fn variants_fold_into_their_family() {
        assert_eq!(
            SportFamily::of(&SportType::VirtualRide),
            SportFamily::Cycling
        );
        assert_eq!(
            SportFamily::of(&SportType::GravelRide),
            SportFamily::Cycling
        );
        assert_eq!(
            SportFamily::of(&SportType::VirtualRun),
            SportFamily::Running
        );
        assert_eq!(SportFamily::of(&SportType::Hike), SportFamily::Other);
    }

    #[test]
    fn a_runner_who_also_rides_the_trainer_spans_two_families() {
        let sports = [SportType::Run, SportType::VirtualRide, SportType::Run];
        assert_eq!(SportFamily::distinct(&sports), 2);
        assert_eq!(
            SportFamily::distinct(&[SportType::Run, SportType::VirtualRun]),
            1
        );
        assert_eq!(SportFamily::distinct(&[]), 0);
    }
}
