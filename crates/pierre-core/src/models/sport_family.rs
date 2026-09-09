// ABOUTME: The coarse sport family a provider sport type belongs to — running, cycling, swimming, other
// ABOUTME: One mapping, read by the season walk's multi-sport condition and the flavour rule's sport mix
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::sport_type_alias::sport_family_head;
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
    ///
    /// Which disciplines belong to a family is not decided here. It is decided
    /// once, by [`sport_family_head`], and this resolves through it: a sport
    /// with a head takes the head's family, and a sport that is its own head
    /// answers for itself. So a terrain or virtual variant added to a family
    /// folds in here without this function being touched.
    ///
    /// It used to carry its own membership list, which drifted the moment the
    /// two were edited apart — `TrailRunning` was folded into `Run` by the
    /// pairing and left in `Other` by this copy, so a trail runner reached the
    /// flavour rule as `SportMix::Mixed` (carnet#418).
    #[must_use]
    pub fn of(sport: &SportType) -> Self {
        let head = sport_family_head(sport);
        match head.as_ref().unwrap_or(sport) {
            SportType::Run => Self::Running,
            SportType::Ride => Self::Cycling,
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
    use super::{sport_family_head, SportFamily};
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
    fn a_trail_run_is_running() {
        // carnet#418: this arm was missing while the cycling terrain variants
        // beside it were present, so a trail runner's primary sport reached
        // the flavour rule as Mixed and their sessions read as multi-sport.
        assert_eq!(
            SportFamily::of(&SportType::TrailRunning),
            SportFamily::Running
        );
        assert_eq!(
            SportFamily::distinct(&[SportType::Run, SportType::TrailRunning]),
            1,
            "road and trail sessions are one athlete training one sport"
        );
    }

    #[test]
    fn every_sport_with_a_head_shares_that_head_s_family() {
        // The invariant that makes the membership list single-source: this
        // holds by construction now, and would fail loudly if `of` ever grew
        // its own list again.
        for (sport, head) in [
            (SportType::TrailRunning, SportType::Run),
            (SportType::VirtualRun, SportType::Run),
            (SportType::MountainBike, SportType::Ride),
            (SportType::GravelRide, SportType::Ride),
            (SportType::EbikeRide, SportType::Ride),
            (SportType::VirtualRide, SportType::Ride),
        ] {
            assert_eq!(
                sport_family_head(&sport),
                Some(head.clone()),
                "{sport:?} folds into {head:?}"
            );
            assert_eq!(
                SportFamily::of(&sport),
                SportFamily::of(&head),
                "{sport:?} must land in the same family as its head {head:?}"
            );
        }
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
