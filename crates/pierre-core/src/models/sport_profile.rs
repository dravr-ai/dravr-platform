// ABOUTME: Sport-mix profile computed from a user's recent activities for coach matching
// ABOUTME: Pure aggregation over Activity slices — counts canonical sports, computes shares + overlap
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::models::{
    resolve_sport_type, sport_family_head, sport_matches_family, Activity, SportType,
};

/// A user's recent sport mix, derived from their connected providers'
/// activities. Drives personalized coach recommendations.
///
/// Keyed by the canonical `snake_case` sport label (e.g. `"run"`,
/// `"ride"`) — the same serialization [`SportType`] uses — so the profile
/// caches cleanly and resolves back to [`SportType`] for matching.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SportProfile {
    /// Total activities scanned to build this profile.
    pub total_activities: u32,
    /// Look-back window (days) the activities were drawn from.
    pub window_days: u32,
    /// Canonical `snake_case` sport label → activity count.
    pub sport_counts: HashMap<String, u32>,
}

impl SportProfile {
    /// Build a profile by counting sport types across `activities`.
    #[must_use]
    pub fn from_activities(activities: &[Activity], window_days: u32) -> Self {
        let mut sport_counts: HashMap<String, u32> = HashMap::new();
        for activity in activities {
            *sport_counts
                .entry(canonical_sport_label(activity))
                .or_insert(0) += 1;
        }
        Self {
            total_activities: u32::try_from(activities.len()).unwrap_or(u32::MAX),
            window_days,
            sport_counts,
        }
    }

    /// The sport the athlete mainly trains, as a canonical `snake_case` label.
    ///
    /// Decided in two steps: the winning FAMILY by total volume, then the
    /// dominant discipline WITHIN it. A bare `max_by_key` over labels answered
    /// the wrong question for anyone whose provider splits one sport across
    /// several — an athlete logging 20 runs against 12 mountain-bike, 11 gravel
    /// and 10 road rides came back `"run"` while 62% of his training was
    /// cycling, and the coach proposal greeted him with "Based on your recent
    /// Run training".
    ///
    /// The second step is why this does not simply return the family head:
    /// reporting `Run` for an athlete who only ever logs `TrailRunning` would
    /// trade one wrong label for another. A trail runner stays a trail runner; a
    /// rider split across three bikes gets the bike he rides most.
    ///
    /// Ties break on the label to keep the answer stable across runs — a
    /// `HashMap` iteration order must not decide what the athlete is told they
    /// do. Labels that resolve to no known sport still count for themselves, so
    /// a provider-specific `Other(...)` can win outright.
    #[must_use]
    pub fn primary_sport(&self) -> Option<String> {
        // Keyed by the head's canonical label rather than the enum: `SportType`
        // is not `Ord`, and the tie-break has to be deterministic or a `HashMap`
        // iteration order decides what the athlete is told they do.
        let mut family_totals: HashMap<String, u32> = HashMap::new();
        for (label, &count) in &self.sport_counts {
            let Some(sport) = resolve_sport_type(label) else {
                continue;
            };
            let head = sport_family_head(&sport).unwrap_or(sport);
            *family_totals.entry(sport_label(&head)).or_insert(0) += count;
        }

        let winner = family_totals
            .iter()
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .and_then(|(head, _)| resolve_sport_type(head));

        let Some(winner) = winner else {
            // Nothing resolved to a known sport — fall back to raw label volume
            // so a purely `Other(...)` profile still answers.
            return self
                .sport_counts
                .iter()
                .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
                .map(|(sport, _)| sport.clone());
        };

        self.sport_counts
            .iter()
            .filter(|(label, _)| {
                resolve_sport_type(label).is_some_and(|s| sport_matches_family(&s, &winner))
            })
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(sport, _)| sport.clone())
    }

    /// The set of [`SportType`]s the user is actively training.
    ///
    /// A sport qualifies when it clears either an absolute floor
    /// (`min_activities`) or a relative share of total activities
    /// (`min_share`, a fraction 0.0..=1.0) — so a single cross-training ride
    /// during a running block doesn't surface cycling coaches. Both thresholds
    /// are caller-supplied (env-tunable via `CoachRecommendationConfig`).
    ///
    /// The thresholds are applied to a FAMILY as well as to each discipline,
    /// because the unit an athlete trains in is not the unit a provider tags in.
    /// Six rides split two road, two mountain, two gravel clear no single label
    /// — each is 2 against a floor of 3, and 6.7% against a floor of 15% — while
    /// being a fifth of that athlete's training. Counting the family too is what
    /// makes eligibility independent of how granularly the provider happened to
    /// label the same six rides; the same athlete whose six rides all say `ride`
    /// already qualified.
    ///
    /// The bar is not lowered, only applied to the right unit: a family head is
    /// inserted ONLY when the family clears and no member cleared on its own, so
    /// one cross-training ride in thirty still resolves to nothing.
    ///
    /// The result is a MATCHING input, not a display surface. It may contain a
    /// head standing in for disciplines the athlete logged individually, which
    /// is exactly what [`Self::activity_type_overlap`] needs and exactly what a
    /// human-facing label must not do — [`Self::primary_sport`] stays at the
    /// label level for that reason.
    #[must_use]
    pub fn active_sports(&self, min_activities: u32, min_share: f32) -> HashSet<SportType> {
        #[allow(clippy::cast_precision_loss)] // counts are small; f32 is exact here
        let total = f32::from(u16::try_from(self.total_activities.max(1)).unwrap_or(u16::MAX));
        let clears = |count: u32| {
            #[allow(clippy::cast_precision_loss)] // counts are small; f32 is exact here
            let share = f32::from(u16::try_from(count).unwrap_or(u16::MAX)) / total;
            count >= min_activities || share >= min_share
        };

        let mut active: HashSet<SportType> = self
            .sport_counts
            .iter()
            .filter(|(_, &count)| clears(count))
            .filter_map(|(label, _)| resolve_sport_type(label))
            .collect();

        // Second pass over the families the athlete's labels belong to. Summed
        // per head so a rider tagged across three bikes is measured as a rider.
        let mut family_totals: HashMap<SportType, u32> = HashMap::new();
        for (label, &count) in &self.sport_counts {
            if let Some(head) = resolve_sport_type(label)
                .as_ref()
                .and_then(sport_family_head)
            {
                *family_totals.entry(head).or_insert(0) += count;
            }
        }
        for (head, family_count) in family_totals {
            // Only when nothing in the family stood on its own — inserting a
            // head beside a member that already qualified adds no matches and
            // costs the set its minimality.
            let member_already_active = active
                .iter()
                .any(|logged| sport_matches_family(logged, &head));
            if !member_already_active && clears(family_count) {
                active.insert(head);
            }
        }

        active
    }

    /// Fraction (0.0..=1.0) of a coach's required `activity_types` that match
    /// a sport the user is actively training.
    ///
    /// Returns 0.0 when the coach lists no activity types or the user does
    /// none of them. `min_activities` / `min_share` define what counts as an
    /// active sport (see [`Self::active_sports`]).
    #[must_use]
    pub fn activity_type_overlap(
        &self,
        coach_activity_types: &[String],
        min_activities: u32,
        min_share: f32,
    ) -> f32 {
        if coach_activity_types.is_empty() {
            return 0.0;
        }
        let active = self.active_sports(min_activities, min_share);
        if active.is_empty() {
            return 0.0;
        }
        // Family-aware, not exact. A coach asking for `Run` is asking for the
        // on-foot family, and an athlete who logs almost everything as
        // `TrailRunning` is a runner; a coach asking for `Ride` wants the
        // cyclist whose rides are tagged `MountainBike` and `GravelRide`.
        // Exact equality made this the mirror of the 2026-08-27 grounding
        // defect: the same athlete whose 22 mountain-bike sessions were hidden
        // from his coach also scored 0.0 against every cycling coach, so none
        // was ever eligible to recommend. A coach naming a specific discipline
        // still matches only that one — see `sport_matches_family`.
        let matches = coach_activity_types
            .iter()
            .filter_map(|t| resolve_sport_type(t))
            .filter(|sport| {
                active
                    .iter()
                    .any(|logged| sport_matches_family(logged, sport))
            })
            .count();
        #[allow(clippy::cast_precision_loss)] // small counts; exact in f32
        let overlap = f32::from(u16::try_from(matches).unwrap_or(u16::MAX))
            / f32::from(u16::try_from(coach_activity_types.len()).unwrap_or(u16::MAX));
        overlap
    }
}

/// Canonical `snake_case` label for a [`SportType`], matching its serde form.
///
/// Falls back to the `Debug` rendering for a variant serde cannot express as a
/// bare string, which keeps the label total rather than optional — callers use
/// it as a map key and a tie-break, neither of which tolerates a gap.
fn sport_label(sport: &SportType) -> String {
    serde_json::to_value(sport)
        .ok()
        .and_then(|v| v.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{sport:?}"))
}

/// Canonical `snake_case` label for an activity's sport type, matching the
/// serde representation of [`SportType`].
fn canonical_sport_label(activity: &Activity) -> String {
    serde_json::to_value(activity.sport_type())
        .ok()
        .and_then(|v| v.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{:?}", activity.sport_type()))
}
