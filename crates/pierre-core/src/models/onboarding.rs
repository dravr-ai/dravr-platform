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
    /// The season walk — the race calendar and what it demands; topics come
    /// from a fixed list, like calibration.
    Season,
    /// The fortnight rail — the next two weeks of an existing plan.
    ///
    /// The odd one out in *where* its work happens: the platform half — read
    /// the plan, decide whether two weeks can be written — is over inside the
    /// `/fortnight` handler, before the athlete reads the reply. The marker is
    /// still written ACTIVE, because the athlete negotiates the weeks after
    /// they are drafted, and it is cleared once the rail's turns are spent.
    Fortnight,
}

impl GuidedFlow {
    /// Whether this flow is an interview — it asks the athlete questions and
    /// records the answers — rather than doing work on their behalf.
    ///
    /// One decision with four consumers, deliberately. An interview must not
    /// be handed the training-plan block, the progression guardrails, the
    /// visual contract, or `save_training_plan`: it is asking questions, and
    /// on 2026-07-24 a walk carrying that context turned into an unwanted
    /// 16-week plan on the athlete's first answer. A flow that exists to
    /// *write* a plan needs all four. Splitting the four predicates apart is
    /// how three of them end up agreeing and the fourth does not.
    #[must_use]
    pub const fn is_interview(self) -> bool {
        match self {
            Self::Pillars | Self::Calibration | Self::Intake | Self::Season => true,
            Self::Fortnight => false,
        }
    }
}

/// Whether a guided-interview topic may be probed where other people read
/// the exchange.
///
/// Declared per topic, never inferred, and the default is [`Self::DmOnly`]:
/// a topic that does not explicitly claim room safety is excluded from room
/// walks. The fixed flows declare theirs in code ([`Pillar::visibility`],
/// [`super::CalibrationTopic::visibility`]); an agent-authored questionnaire
/// will supply one per topic in its package, reviewed before publication.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopicVisibility {
    /// Probed only where the athlete is alone with the agent.
    #[default]
    DmOnly,
    /// May also be probed in a shared room whose walk the athlete started
    /// there — typing the command in the room is the consent.
    RoomSafe,
}

/// Who reads a guided walk's exchange.
///
/// Recorded once at activation and never derived later from the conversation
/// alone: the athlete's choice of where to start the walk is the consent
/// record, so it must survive exactly as granted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WalkAudience {
    /// A 1:1 conversation — every pre-feature row parses as this.
    #[default]
    Private,
    /// A shared room. Only [`TopicVisibility::RoomSafe`] topics are probed.
    Room,
}

/// The athlete's recent training load, computed once when a calibration
/// interview starts and reused by every later turn.
///
/// Held in the flow state rather than refetched per turn: the baseline-confirm
/// topic quotes it back for confirmation, and a figure that drifted mid-walk
/// would make the agent contradict itself.
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
    /// Distinct sport families (running, cycling, swimming, other) the window
    /// holds. Two or more is what makes an athlete multi-sport for the season
    /// walk's facility question. Defaults to zero on a snapshot stored before
    /// the field existed, which reads as single-sport — the shorter walk.
    #[serde(default)]
    pub sport_families: u32,
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
    /// The member this walk belongs to, as a UUID string. Set at activation;
    /// the turn resolver advances the walk only for this speaker, so in a
    /// shared thread every other member's message runs as ordinary coaching.
    /// `None` on rows written before the field existed and on auto-started
    /// walks, where the conversation's owner is the subject by construction.
    ///
    /// A string rather than a `Uuid` so a corrupt value degrades at the use
    /// site — mirroring how a corrupt `conversation.user_id` degrades the
    /// turn — instead of failing the whole state parse and killing the walk.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject_user_id: Option<String>,
    /// Who reads this walk's exchange. Absent from rows written before the
    /// field existed; those are all private walks — hence `serde(default)`.
    #[serde(default)]
    pub audience: WalkAudience,
    /// How many turns a topic-less flow has already taken.
    ///
    /// The interview walks end when they run out of topics to ask. A flow
    /// that asks nothing has no such bound, so it carries its own: without
    /// one, a fortnight rail that opened would own every later turn of the
    /// conversation forever. Absent from rows written before the field
    /// existed — hence `serde(default)`, which reads as zero.
    #[serde(default)]
    pub turns_owned: u8,
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
            subject_user_id: None,
            audience: WalkAudience::Private,
            turns_owned: 0,
        }
    }

    /// This state with one more owned turn counted against its budget.
    #[must_use]
    pub fn with_owned_turn(mut self) -> Self {
        self.turns_owned = self.turns_owned.saturating_add(1);
        self
    }

    /// This state marked finished, stamped `at` (RFC3339).
    ///
    /// The row is kept rather than deleted so the *next* turn can tell that an
    /// interview just ended. That matters because the interview's directive is
    /// the most forcefully worded block in the prompt — it "overrides every
    /// other instruction" and ends with "do not build, propose, or save a
    /// training plan" — and the athlete's transcript keeps its shape after the
    /// block itself disappears. On 2026-07-28 an agent spent the turn after a
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
        let Some((_, at)) = Self::retired(raw) else {
            return false;
        };
        now.signed_duration_since(at) < Duration::minutes(COMPLETION_RELEASE_WINDOW_MINUTES)
    }

    /// The flow a retired marker belonged to — the release directive names
    /// what the agent may now do, and that depends on which walk ended.
    /// `None` for an active flow, an absent or unparseable column, or a
    /// marker with no completion stamp.
    #[must_use]
    pub fn retired_flow(raw: Option<&str>) -> Option<GuidedFlow> {
        Self::retired(raw).map(|(flow, _)| flow)
    }

    /// A retired marker's flow and completion instant.
    fn retired(raw: Option<&str>) -> Option<(GuidedFlow, DateTime<Utc>)> {
        let state = raw.and_then(|s| serde_json::from_str::<Self>(s).ok())?;
        if state.active {
            return None;
        }
        let at = state
            .completed_at
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())?;
        Some((state.flow, at.with_timezone(&Utc)))
    }

    /// This state with the flow-start load snapshot attached.
    #[must_use]
    pub fn with_snapshot(mut self, snapshot: Option<LoadSnapshot>) -> Self {
        self.snapshot = snapshot;
        self
    }

    /// This state bound to the member it interviews.
    ///
    /// The turn resolver advances the walk only for this speaker; everyone
    /// else in the thread runs an ordinary coaching turn.
    #[must_use]
    pub fn with_subject(mut self, user_id: String) -> Self {
        self.subject_user_id = Some(user_id);
        self
    }

    /// This state with its audience recorded.
    #[must_use]
    pub const fn with_audience(mut self, audience: WalkAudience) -> Self {
        self.audience = audience;
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
                    GuidedFlow::Season => r#"{"active":true,"started_at":"","flow":"season"}"#,
                    GuidedFlow::Fortnight => {
                        r#"{"active":true,"started_at":"","flow":"fortnight"}"#
                    }
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

    /// Whether this topic may be probed in a shared room.
    ///
    /// The North Star — what the athlete trains *for* — is the kind of thing
    /// a training partner already knows; the pillars each declare their own.
    #[must_use]
    pub const fn visibility(self) -> TopicVisibility {
        match self {
            Self::NorthStar => TopicVisibility::RoomSafe,
            Self::Pillar(p) => p.visibility(),
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

    /// Every uncovered topic the audience may hear, North Star first then
    /// pillars in canonical order.
    ///
    /// A room walk EXCLUDES [`TopicVisibility::DmOnly`] topics from the set
    /// rather than skipping them turn by turn: `next_target` returning `None`
    /// is the walk's completion signal, so a merely-skipped topic would keep
    /// the walk alive with nothing left it may ask.
    fn uncovered(&self, audience: WalkAudience) -> Vec<CoverageTarget> {
        let audible = |t: &CoverageTarget| match audience {
            WalkAudience::Private => true,
            WalkAudience::Room => t.visibility() == TopicVisibility::RoomSafe,
        };
        let mut out = Vec::with_capacity(1 + Pillar::ALL.len());
        if !self.north_star_covered && audible(&CoverageTarget::NorthStar) {
            out.push(CoverageTarget::NorthStar);
        }
        out.extend(
            Pillar::ALL
                .into_iter()
                .filter(|p| !self.pillars.get(p).copied().unwrap_or(false))
                .map(CoverageTarget::Pillar)
                .filter(|t| audible(t)),
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
    /// `None` means the walk is over: every topic the audience may hear is
    /// either covered or out of attempts.
    #[must_use]
    pub fn next_target(
        &self,
        probed: &[TopicSlug],
        audience: WalkAudience,
    ) -> Option<CoverageTarget> {
        self.uncovered(audience)
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
