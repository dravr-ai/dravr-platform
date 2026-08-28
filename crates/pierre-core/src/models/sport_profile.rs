// ABOUTME: Sport-mix profile computed from a user's recent activities for coach matching
// ABOUTME: Pure aggregation over Activity slices — counts canonical sports, computes shares + overlap
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::models::{resolve_sport_type, sport_matches_family, Activity, SportType};

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

    /// The most frequently logged sport, as a canonical `snake_case` label.
    #[must_use]
    pub fn primary_sport(&self) -> Option<String> {
        self.sport_counts
            .iter()
            .max_by_key(|(_, count)| **count)
            .map(|(sport, _)| sport.clone())
    }

    /// The set of [`SportType`]s the user is actively training.
    ///
    /// A sport qualifies when it clears either an absolute floor
    /// (`min_activities`) or a relative share of total activities
    /// (`min_share`, a fraction 0.0..=1.0) — so a single cross-training ride
    /// during a running block doesn't surface cycling coaches. Both thresholds
    /// are caller-supplied (env-tunable via `CoachRecommendationConfig`).
    #[must_use]
    pub fn active_sports(&self, min_activities: u32, min_share: f32) -> HashSet<SportType> {
        #[allow(clippy::cast_precision_loss)] // counts are small; f32 is exact here
        let total = f32::from(u16::try_from(self.total_activities.max(1)).unwrap_or(u16::MAX));
        self.sport_counts
            .iter()
            .filter(|(_, &count)| {
                #[allow(clippy::cast_precision_loss)] // counts are small; f32 is exact here
                let share = f32::from(u16::try_from(count).unwrap_or(u16::MAX)) / total;
                count >= min_activities || share >= min_share
            })
            .filter_map(|(label, _)| resolve_sport_type(label))
            .collect()
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

    /// Whether the user actively trains at least one of the coach's required
    /// `activity_types`.
    #[must_use]
    pub fn matches_activity_types(
        &self,
        coach_activity_types: &[String],
        min_activities: u32,
        min_share: f32,
    ) -> bool {
        self.activity_type_overlap(coach_activity_types, min_activities, min_share) > 0.0
    }
}

/// Canonical `snake_case` label for an activity's sport type, matching the
/// serde representation of [`SportType`].
fn canonical_sport_label(activity: &Activity) -> String {
    serde_json::to_value(activity.sport_type())
        .ok()
        .and_then(|v| v.as_str().map(ToOwned::to_owned))
        .unwrap_or_else(|| format!("{:?}", activity.sport_type()))
}
