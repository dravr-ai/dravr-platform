// ABOUTME: Pillar-onboarding flow state + coverage map derived from the Dossier
// ABOUTME: Drives the guided multi-turn onboarding: which pillar to probe next, when done
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{Dossier, Pillar};

/// Conversation-scoped marker that a guided pillar-onboarding walk is active.
///
/// Serialized into `chat_conversations.onboarding_state`. The "next pillar to
/// probe" is NOT stored here — it is re-derived from the live Dossier each turn
/// via [`CoverageMap`], so the flow is self-healing and stateless beyond this
/// active marker.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingState {
    /// Whether the guided walk is currently active.
    pub active: bool,
    /// RFC3339 timestamp when the walk was (re)started, for observability.
    pub started_at: String,
}

impl OnboardingState {
    /// Start a fresh onboarding walk.
    #[must_use]
    pub fn start(started_at: String) -> Self {
        Self {
            active: true,
            started_at,
        }
    }

    /// Start a fresh walk stamped now, serialized to the JSON stored in
    /// `chat_conversations.onboarding_state`. Convenience for callers (e.g. the
    /// `/context` handler) that lack a `chrono`/`serde_json` dependency.
    #[must_use]
    pub fn start_now_column() -> String {
        serde_json::to_string(&Self::start(chrono::Utc::now().to_rfc3339()))
            .unwrap_or_else(|_| r#"{"active":true,"started_at":""}"#.to_owned())
    }

    /// Parse from the stored JSON column. Returns `None` on absent/invalid.
    #[must_use]
    pub fn from_column(raw: Option<&str>) -> Option<Self> {
        raw.and_then(|s| serde_json::from_str::<Self>(s).ok())
            .filter(|s| s.active)
    }
}

/// What to capture next in the onboarding walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoverageTarget {
    /// The North Star (life motivations) — captured first.
    NorthStar,
    /// A specific health pillar.
    Pillar(Pillar),
}

/// Read-time projection of which onboarding topics the user has covered.
///
/// "Covered" means the user has at least one **non-stale** fact in that bucket
/// (any source). Derived purely from the [`Dossier`] — never stored — so it can
/// never drift from the facts.
#[derive(Debug, Clone)]
pub struct CoverageMap {
    /// Whether the North Star is covered.
    pub north_star_covered: bool,
    /// Per-pillar coverage, keyed in canonical order.
    pub pillars: BTreeMap<Pillar, bool>,
}

impl CoverageMap {
    /// Derive coverage from the composed dossier.
    #[must_use]
    pub fn from_dossier(dossier: &Dossier) -> Self {
        let north_star_covered = dossier.north_star.iter().any(|f| !f.stale);
        let mut pillars = BTreeMap::new();
        for pillar in Pillar::ALL {
            let covered = dossier
                .pillars
                .get(&pillar)
                .is_some_and(|facts| facts.iter().any(|f| !f.stale));
            pillars.insert(pillar, covered);
        }
        Self {
            north_star_covered,
            pillars,
        }
    }

    /// The next topic to probe: North Star first, then the first uncovered
    /// pillar in canonical order. `None` once everything is covered.
    #[must_use]
    pub fn next_target(&self) -> Option<CoverageTarget> {
        if !self.north_star_covered {
            return Some(CoverageTarget::NorthStar);
        }
        Pillar::ALL
            .into_iter()
            .find(|p| !self.pillars.get(p).copied().unwrap_or(false))
            .map(CoverageTarget::Pillar)
    }

    /// Onboarding is complete once the North Star and all six pillars are
    /// covered (the depth chosen for v1).
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.north_star_covered && self.pillars.values().all(|covered| *covered)
    }

    /// Count of covered topics out of the 7 (North Star + 6 pillars), for
    /// progress display.
    #[must_use]
    pub fn covered_count(&self) -> usize {
        usize::from(self.north_star_covered) + self.pillars.values().filter(|c| **c).count()
    }
}

#[cfg(test)]
mod tests {
    use super::{CoverageMap, CoverageTarget, OnboardingState};
    use crate::models::{Dossier, DossierFact, Pillar};
    use uuid::Uuid;

    fn covered_fact() -> DossierFact {
        DossierFact {
            kind: "goal".to_owned(),
            subject: "you".to_owned(),
            predicate: "want".to_owned(),
            object: "x".to_owned(),
            confidence: 0.9,
            source: "onboarding".to_owned(),
            updated_at: chrono::DateTime::from_timestamp(0, 0).unwrap_or_default(),
            valid_until: None,
            stale: false,
        }
    }

    #[test]
    fn empty_dossier_targets_north_star_first() {
        let d = Dossier::empty(Uuid::nil(), Uuid::nil());
        let cov = CoverageMap::from_dossier(&d);
        assert_eq!(cov.next_target(), Some(CoverageTarget::NorthStar));
        assert!(!cov.is_complete());
        assert_eq!(cov.covered_count(), 0);
    }

    #[test]
    fn north_star_then_first_uncovered_pillar() {
        let mut d = Dossier::empty(Uuid::nil(), Uuid::nil());
        d.north_star = vec![covered_fact()];
        let cov = CoverageMap::from_dossier(&d);
        assert_eq!(
            cov.next_target(),
            Some(CoverageTarget::Pillar(Pillar::TrainingAndMovement))
        );
    }

    #[test]
    fn stale_fact_does_not_count_as_covered() {
        let mut d = Dossier::empty(Uuid::nil(), Uuid::nil());
        let mut f = covered_fact();
        f.stale = true;
        d.north_star = vec![f];
        let cov = CoverageMap::from_dossier(&d);
        assert!(!cov.north_star_covered);
    }

    #[test]
    fn complete_when_all_seven_covered() {
        let mut d = Dossier::empty(Uuid::nil(), Uuid::nil());
        d.north_star = vec![covered_fact()];
        for pillar in Pillar::ALL {
            d.pillars.insert(pillar, vec![covered_fact()]);
        }
        let cov = CoverageMap::from_dossier(&d);
        assert!(cov.is_complete());
        assert_eq!(cov.next_target(), None);
        assert_eq!(cov.covered_count(), 7);
    }

    #[test]
    fn onboarding_state_roundtrip() {
        let s = OnboardingState::start("2026-06-17T00:00:00Z".to_owned());
        let json = serde_json::to_string(&s).unwrap_or_default();
        let back = OnboardingState::from_column(Some(&json));
        assert!(back.is_some());
        assert!(OnboardingState::from_column(None).is_none());
        assert!(OnboardingState::from_column(Some("not json")).is_none());
    }
}
