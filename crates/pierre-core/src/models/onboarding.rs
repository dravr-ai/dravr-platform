// ABOUTME: Pillar-onboarding flow state + coverage map derived from the Dossier
// ABOUTME: Drives the guided multi-turn onboarding: which pillar to probe next, when done
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{Dossier, Pillar};

/// Maximum times a single topic is probed before the walk moves past it.
///
/// Coverage is derived from the Dossier, so a probe the athlete answers
/// unextractably ("dunno", or an extraction miss) never flips its topic to
/// covered — and [`CoverageMap::next_target`] would hand back that same topic
/// forever. Two delivered probes is the budget (one ask plus one rephrase),
/// after which the walk treats the topic as settled and advances. The Dossier
/// still reports it uncovered, so `/pillars <pillar>` can re-screen it later.
pub const MAX_PROBE_ATTEMPTS: usize = 2;

/// Stable string identifier for an onboarding topic — `north_star` or a
/// [`Pillar`] slug — as persisted in [`OnboardingState::probed`].
///
/// Serialized transparently, so the stored column holds a plain JSON string
/// array. A slug this build does not recognize matches no topic rather than
/// failing the parse, keeping rows written by other builds readable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TopicSlug(String);

impl TopicSlug {
    /// Wrap a stable topic identifier.
    #[must_use]
    pub const fn new(slug: String) -> Self {
        Self(slug)
    }

    /// The slug as a string.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Conversation-scoped state of a guided pillar-onboarding walk.
///
/// Serialized into `chat_conversations.onboarding_state`. Which topic is
/// *covered* is NOT stored here — coverage is re-derived from the live Dossier
/// every turn via [`CoverageMap`], so the flow stays self-healing. What is
/// stored is the ask history, which coverage cannot express: it lets the walk
/// advance before an answer's fact lands and gives every topic a bounded
/// number of attempts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingState {
    /// Whether the guided walk is currently active.
    pub active: bool,
    /// RFC3339 timestamp when the walk was (re)started, for observability.
    pub started_at: String,
    /// One entry per topic probe that actually reached the athlete, in ask
    /// order — so an entry's multiplicity is that topic's attempt count. A
    /// withheld reply is never recorded: the athlete saw a marker, not the
    /// question. Absent from rows written before this field existed, hence
    /// `serde(default)`.
    #[serde(default)]
    pub probed: Vec<TopicSlug>,
}

impl OnboardingState {
    /// Start a fresh onboarding walk.
    #[must_use]
    pub fn start(started_at: String) -> Self {
        Self {
            active: true,
            started_at,
            probed: Vec::new(),
        }
    }

    /// This state with one more delivered probe of `target` recorded.
    #[must_use]
    pub fn with_delivered_probe(mut self, target: CoverageTarget) -> Self {
        self.probed.push(target.slug());
        self
    }

    /// Serialize back into the `chat_conversations.onboarding_state` column.
    ///
    /// # Errors
    /// Returns the `serde_json` error when the state cannot be serialized.
    pub fn to_column(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Start a fresh walk stamped now, serialized to the JSON stored in
    /// `chat_conversations.onboarding_state`. Convenience for callers (e.g. the
    /// `/pillars` handler) that lack a `chrono`/`serde_json` dependency.
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

impl CoverageTarget {
    /// The stable slug recorded in [`OnboardingState::probed`].
    #[must_use]
    pub fn slug(self) -> TopicSlug {
        match self {
            Self::NorthStar => TopicSlug::new("north_star".to_owned()),
            Self::Pillar(p) => TopicSlug::new(p.as_str().to_owned()),
        }
    }
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

    /// Every uncovered topic, North Star first then pillars in canonical order.
    fn uncovered(&self) -> Vec<CoverageTarget> {
        let mut out = Vec::with_capacity(1 + Pillar::ALL.len());
        if !self.north_star_covered {
            out.push(CoverageTarget::NorthStar);
        }
        out.extend(
            Pillar::ALL
                .into_iter()
                .filter(|p| !self.pillars.get(p).copied().unwrap_or(false))
                .map(CoverageTarget::Pillar),
        );
        out
    }

    /// How many delivered probes `probed` records for `target`.
    fn attempts(target: CoverageTarget, probed: &[TopicSlug]) -> usize {
        let slug = target.slug();
        probed.iter().filter(|s| **s == slug).count()
    }

    /// The next topic to probe, given the walk's delivered-probe history.
    ///
    /// Uncovered topics are ordered by how many times they have already been
    /// asked, ties broken by canonical order — so the walk sweeps every topic
    /// once before it comes back to one whose answer produced no fact, instead
    /// of stalling on it while extraction lags. A topic that has burned
    /// [`MAX_PROBE_ATTEMPTS`] is skipped for the rest of the walk.
    ///
    /// `None` means the walk is over: every topic is either covered or out of
    /// attempts.
    #[must_use]
    pub fn next_target(&self, probed: &[TopicSlug]) -> Option<CoverageTarget> {
        self.uncovered()
            .into_iter()
            .filter(|t| Self::attempts(*t, probed) < MAX_PROBE_ATTEMPTS)
            .min_by_key(|t| Self::attempts(*t, probed))
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
    use super::{CoverageMap, CoverageTarget, OnboardingState, TopicSlug, MAX_PROBE_ATTEMPTS};
    use crate::models::{Dossier, DossierFact, Pillar};
    use uuid::Uuid;

    /// Delivered-probe history for the given targets, in ask order.
    fn probed(targets: &[CoverageTarget]) -> Vec<TopicSlug> {
        targets.iter().map(|t| t.slug()).collect()
    }

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
        assert_eq!(cov.next_target(&[]), Some(CoverageTarget::NorthStar));
        assert!(!cov.is_complete());
        assert_eq!(cov.covered_count(), 0);
    }

    #[test]
    fn north_star_then_first_uncovered_pillar() {
        let mut d = Dossier::empty(Uuid::nil(), Uuid::nil());
        d.north_star = vec![covered_fact()];
        let cov = CoverageMap::from_dossier(&d);
        assert_eq!(
            cov.next_target(&[]),
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
        assert_eq!(cov.next_target(&[]), None);
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

    #[test]
    fn probed_north_star_advances_before_its_fact_lands() {
        // The athlete answered the North Star but extraction has not landed:
        // coverage still says uncovered. One delivered probe is enough to move
        // the walk to Training & Movement instead of re-asking.
        let d = Dossier::empty(Uuid::nil(), Uuid::nil());
        let cov = CoverageMap::from_dossier(&d);
        assert_eq!(
            cov.next_target(&probed(&[CoverageTarget::NorthStar])),
            Some(CoverageTarget::Pillar(Pillar::TrainingAndMovement))
        );
    }

    #[test]
    fn unprobed_topics_are_swept_before_a_second_attempt() {
        let d = Dossier::empty(Uuid::nil(), Uuid::nil());
        let cov = CoverageMap::from_dossier(&d);
        // Every topic asked exactly once, none extracted: the sweep restarts at
        // the North Star for attempt two rather than stopping.
        let mut history = vec![CoverageTarget::NorthStar];
        history.extend(Pillar::ALL.map(CoverageTarget::Pillar));
        assert_eq!(
            cov.next_target(&probed(&history)),
            Some(CoverageTarget::NorthStar)
        );
    }

    #[test]
    fn walk_terminates_once_every_topic_burns_its_attempts() {
        let d = Dossier::empty(Uuid::nil(), Uuid::nil());
        let cov = CoverageMap::from_dossier(&d);
        let mut history = Vec::new();
        for _ in 0..MAX_PROBE_ATTEMPTS {
            history.push(CoverageTarget::NorthStar);
            history.extend(Pillar::ALL.map(CoverageTarget::Pillar));
        }
        assert_eq!(
            cov.next_target(&probed(&history)),
            None,
            "nothing covered, but every topic is out of attempts — walk must end"
        );
        // Coverage is still the honest 0/7: /pillars can re-screen later.
        assert!(!cov.is_complete());
        assert_eq!(cov.covered_count(), 0);
    }

    #[test]
    fn covered_topic_is_skipped_even_with_attempts_left() {
        let mut d = Dossier::empty(Uuid::nil(), Uuid::nil());
        d.north_star = vec![covered_fact()];
        let cov = CoverageMap::from_dossier(&d);
        assert_eq!(
            cov.next_target(&probed(&[CoverageTarget::NorthStar])),
            Some(CoverageTarget::Pillar(Pillar::TrainingAndMovement))
        );
    }

    #[test]
    fn state_without_probed_field_parses_and_records_probes() {
        // Rows written before `probed` existed must keep loading.
        let legacy = r#"{"active":true,"started_at":"2026-06-17T00:00:00Z"}"#;
        let parsed = OnboardingState::from_column(Some(legacy));
        assert!(
            parsed.is_some(),
            "a row written before `probed` existed must still parse"
        );
        assert_eq!(parsed.as_ref().map(|s| s.probed.len()), Some(0));

        let recorded = parsed
            .map(|s| s.with_delivered_probe(CoverageTarget::Pillar(Pillar::Fuelling)))
            .and_then(|s| s.to_column().ok())
            .and_then(|column| OnboardingState::from_column(Some(&column)));
        assert_eq!(
            recorded.map(|s| s.probed),
            Some(probed(&[CoverageTarget::Pillar(Pillar::Fuelling)])),
            "a recorded probe must survive the column round trip"
        );
        assert_eq!(
            CoverageTarget::Pillar(Pillar::Fuelling).slug().as_str(),
            "fuelling"
        );
    }

    #[test]
    fn unknown_slug_in_probed_matches_no_topic() {
        let d = Dossier::empty(Uuid::nil(), Uuid::nil());
        let cov = CoverageMap::from_dossier(&d);
        let foreign = vec![TopicSlug::new("topic_from_a_later_build".to_owned())];
        assert_eq!(cov.next_target(&foreign), Some(CoverageTarget::NorthStar));
    }
}
