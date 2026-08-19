// ABOUTME: Guided-flow state + the two next-topic policies (Dossier coverage, calibration list)
// ABOUTME: Drives the guided multi-turn interviews: which topic to probe next, and when done
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
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

/// Which guided interview a conversation is running.
///
/// One conversation runs at most one flow at a time. Persisted in
/// [`OnboardingState::flow`]; rows written before the field existed are
/// pillars walks, which is what [`Default`] returns.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuidedFlow {
    /// The six-pillar profile walk — topics come from Dossier coverage.
    #[default]
    Pillars,
    /// The difficulty-calibration interview — topics come from a fixed list.
    Calibration,
    /// The messaging intake — profile type, then the seven PAR-Q+ questions.
    ///
    /// Unlike the other two, the platform asks and parses this one itself: the
    /// PAR-Q+ is a standardised instrument whose wording is the instrument, and
    /// a paraphrased question with an inferred yes/no is a different screen. So
    /// an intake turn never reaches the model, and the guided-turn resolver
    /// treats this flow as inactive.
    Intake,
}

/// The athlete's recent training load, computed once when a calibration
/// interview starts and reused by every later turn.
///
/// Held in the flow state rather than refetched per turn: the baseline-confirm
/// topic quotes it back for confirmation, and a figure that drifted mid-walk
/// would make the coach contradict itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoadSnapshot {
    /// Mean training hours per week over the window.
    pub weekly_hours: f64,
    /// Mean sessions per week over the window.
    pub sessions_per_week: f64,
    /// Longest single session in the window, in minutes.
    pub longest_session_min: u32,
    /// How many weeks the means were taken over.
    pub weeks: u32,
}

/// Conversation-scoped state of a guided interview.
///
/// Serialized into `chat_conversations.onboarding_state`. For the pillars walk,
/// which topic is *covered* is NOT stored here — coverage is re-derived from the
/// live Dossier every turn via [`CoverageMap`], so that flow stays self-healing.
/// What is stored is the ask history, which coverage cannot express: it lets the
/// walk advance before an answer's fact lands and gives every topic a bounded
/// number of attempts. Calibration has no Dossier-derived coverage — seven of
/// its topics land as the same kind in the same pillar — so for that flow the
/// ask history is the whole completion authority.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnboardingState {
    /// Whether the guided walk is currently active.
    pub active: bool,
    /// RFC3339 timestamp when the walk was (re)started, for observability and
    /// for scoping a re-run's supersession window.
    pub started_at: String,
    /// One entry per topic probe that actually reached the athlete, in ask
    /// order — so an entry's multiplicity is that topic's attempt count. A
    /// withheld reply is never recorded: the athlete saw a marker, not the
    /// question. Absent from rows written before this field existed, hence
    /// `serde(default)`.
    #[serde(default)]
    pub probed: Vec<TopicSlug>,
    /// Which interview this is. Absent from rows written before the field
    /// existed, and those are all pillars walks — hence `serde(default)`.
    #[serde(default)]
    pub flow: GuidedFlow,
    /// Recent-load figures computed at calibration start, or `None` for a
    /// pillars walk and for an athlete with no connected provider.
    #[serde(default)]
    pub snapshot: Option<LoadSnapshot>,
    /// RFC3339 timestamp when the interview finished, set once `active` goes
    /// false. `None` while the flow runs, and absent from rows written before
    /// the field existed — hence `serde(default)`.
    #[serde(default)]
    pub completed_at: Option<String>,
}

/// How long after a guided interview ends its release directive keeps firing.
///
/// The directive is cleared the turn it fires, so this window only matters
/// when that clear fails to write. Bounding it stops a stuck marker from
/// asserting "the interview just finished" days later, and 30 minutes is far
/// wider than the gap that mattered in the incident it exists for — the
/// athlete's next message came 48 seconds after the wrap-up.
pub const COMPLETION_RELEASE_WINDOW_MINUTES: i64 = 30;

impl OnboardingState {
    /// Start a fresh guided walk for `flow`.
    #[must_use]
    pub fn start(started_at: String, flow: GuidedFlow) -> Self {
        Self {
            active: true,
            started_at,
            probed: Vec::new(),
            flow,
            snapshot: None,
            completed_at: None,
        }
    }

    /// This state marked finished, stamped `at` (RFC3339).
    ///
    /// The row is kept rather than deleted so the *next* turn can tell that an
    /// interview just ended. That matters because the interview's directive is
    /// the most forcefully worded block in the prompt — it "overrides every
    /// other instruction" and ends with "do not build, propose, or save a
    /// training plan" — and the athlete's transcript keeps its shape after the
    /// block itself disappears. On 2026-07-28 a coach spent the turn after a
    /// completed calibration telling the athlete it could not save his plan,
    /// having never called the tool, which was callable on that turn.
    #[must_use]
    pub fn completed(mut self, at: String) -> Self {
        self.active = false;
        self.completed_at = Some(at);
        self
    }

    /// Whether the stored column is a guided interview that finished recently
    /// enough to still warrant the release directive.
    ///
    /// `false` for an active flow — that is [`Self::from_column`]'s job — for
    /// an absent, unparseable or un-stamped column, and for a completion older
    /// than [`COMPLETION_RELEASE_WINDOW_MINUTES`].
    #[must_use]
    pub fn just_completed(raw: Option<&str>, now: DateTime<Utc>) -> bool {
        let Some(state) = raw.and_then(|s| serde_json::from_str::<Self>(s).ok()) else {
            return false;
        };
        if state.active {
            return false;
        }
        let Some(at) = state
            .completed_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        else {
            return false;
        };
        now.signed_duration_since(at.with_timezone(&Utc))
            < Duration::minutes(COMPLETION_RELEASE_WINDOW_MINUTES)
    }

    /// This state with the flow-start load snapshot attached.
    #[must_use]
    pub fn with_snapshot(mut self, snapshot: Option<LoadSnapshot>) -> Self {
        self.snapshot = snapshot;
        self
    }

    /// This state with one more delivered probe of `topic` recorded.
    ///
    /// Takes the slug rather than a flow-specific target type so both
    /// interviews append to the one ledger.
    #[must_use]
    pub fn with_delivered_probe(mut self, topic: TopicSlug) -> Self {
        self.probed.push(topic);
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
    /// `/pillars` and `/calibrate` handlers) that lack a `chrono`/`serde_json`
    /// dependency.
    ///
    /// The serialization-failure fallback carries the flow. It used to be a
    /// single flow-less literal, which — once `flow` defaults to `Pillars` —
    /// would have turned a calibration start into a pillars walk silently, with
    /// the athlete reading a calibration opener and then being asked about
    /// sleep hygiene.
    #[must_use]
    pub fn start_now_column(flow: GuidedFlow) -> String {
        serde_json::to_string(&Self::start(chrono::Utc::now().to_rfc3339(), flow)).unwrap_or_else(
            |_| {
                match flow {
                    GuidedFlow::Pillars => r#"{"active":true,"started_at":"","flow":"pillars"}"#,
                    GuidedFlow::Calibration => {
                        r#"{"active":true,"started_at":"","flow":"calibration"}"#
                    }
                    GuidedFlow::Intake => r#"{"active":true,"started_at":"","flow":"intake"}"#,
                }
                .to_owned()
            },
        )
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
    use super::{
        CoverageMap, CoverageTarget, GuidedFlow, OnboardingState, TopicSlug, MAX_PROBE_ATTEMPTS,
    };
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
        let s = OnboardingState::start("2026-06-17T00:00:00Z".to_owned(), GuidedFlow::Pillars);
        let json = serde_json::to_string(&s).unwrap_or_default();
        let back = OnboardingState::from_column(Some(&json));
        assert!(back.is_some());
        assert!(OnboardingState::from_column(None).is_none());
        assert!(OnboardingState::from_column(Some("not json")).is_none());
    }

    #[test]
    fn flow_survives_the_column_round_trip() {
        let json = OnboardingState::start_now_column(GuidedFlow::Calibration);
        assert_eq!(
            OnboardingState::from_column(Some(&json)).map(|s| s.flow),
            Some(GuidedFlow::Calibration),
            "a calibration start that came back as a pillars walk would ask the wrong questions"
        );
        let json = OnboardingState::start_now_column(GuidedFlow::Pillars);
        assert_eq!(
            OnboardingState::from_column(Some(&json)).map(|s| s.flow),
            Some(GuidedFlow::Pillars)
        );
    }

    #[test]
    fn the_serialization_fallback_literals_carry_their_flow() {
        // `start_now_column` degrades to a hand-written literal if serde ever
        // fails. Both literals must parse back to the flow that produced them —
        // a flow-less fallback would silently default a calibration start to
        // the pillars walk.
        for (literal, expected) in [
            (
                r#"{"active":true,"started_at":"","flow":"pillars"}"#,
                GuidedFlow::Pillars,
            ),
            (
                r#"{"active":true,"started_at":"","flow":"calibration"}"#,
                GuidedFlow::Calibration,
            ),
        ] {
            assert_eq!(
                OnboardingState::from_column(Some(literal)).map(|s| s.flow),
                Some(expected),
                "fallback literal {literal} did not round-trip"
            );
        }
    }

    #[test]
    fn a_row_written_before_flow_existed_is_a_pillars_walk() {
        // Live rows predate both `probed` and `flow`. They must keep parsing,
        // and they are all pillars walks — calibration did not exist.
        let legacy = r#"{"active":true,"started_at":"2026-06-17T00:00:00Z"}"#;
        let parsed = OnboardingState::from_column(Some(legacy));
        assert_eq!(parsed.as_ref().map(|s| s.flow), Some(GuidedFlow::Pillars));
        assert_eq!(parsed.as_ref().map(|s| s.probed.len()), Some(0));
        assert!(parsed.is_some_and(|s| s.snapshot.is_none()));
    }

    #[test]
    fn a_row_written_with_probed_but_no_flow_still_parses() {
        // Rows written by the pillars Phase A build: `probed` present, `flow`
        // absent. Both defaults must apply independently.
        let mid = r#"{"active":true,"started_at":"2026-07-26T00:00:00Z","probed":["north_star"]}"#;
        let parsed = OnboardingState::from_column(Some(mid));
        assert_eq!(parsed.as_ref().map(|s| s.flow), Some(GuidedFlow::Pillars));
        assert_eq!(parsed.map(|s| s.probed.len()), Some(1));
    }

    #[test]
    fn the_snapshot_survives_the_column_round_trip() {
        let snapshot = super::LoadSnapshot {
            weekly_hours: 7.5,
            sessions_per_week: 4.0,
            longest_session_min: 195,
            weeks: 6,
        };
        let state =
            OnboardingState::start("2026-07-28T00:00:00Z".to_owned(), GuidedFlow::Calibration)
                .with_snapshot(Some(snapshot.clone()));
        let column = state.to_column().unwrap_or_default();
        assert_eq!(
            OnboardingState::from_column(Some(&column)).and_then(|s| s.snapshot),
            Some(snapshot),
            "later turns quote the baseline back; a lost snapshot would refetch and drift"
        );
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
            .map(|s| s.with_delivered_probe(CoverageTarget::Pillar(Pillar::Fuelling).slug()))
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
