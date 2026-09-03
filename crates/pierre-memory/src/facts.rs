// ABOUTME: UserFact — a structured claim the harness has extracted about a user
// ABOUTME: Distilled from conversation turns and activities by the memory extraction worker
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use pierre_core::models::Pillar;
use serde::{Deserialize, Serialize};

use crate::scope::MemoryScope;

/// Semantic category of a [`UserFact`].
///
/// Bounded to a small enum so the extraction prompt and the retrieval filter
/// can share a stable vocabulary. New kinds should be additive only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactKind {
    /// Long-lived identity/preference (sport, goal, coach style preference).
    Preference,
    /// Physiological or training-state claim ("resting HR around 52").
    Physiology,
    /// Injury, pain, or health constraint the coach should remember.
    Injury,
    /// Goal the user committed to, with or without a deadline.
    Goal,
    /// Schedule / availability constraint ("can only run Tue/Thu/Sat").
    Schedule,
    /// Equipment or environment ("trains on a Wahoo Kickr indoors in winter").
    Equipment,
    /// A core life motivation orienting the pillars ("be present for my kids").
    /// One to three per user; the layer above the pillars.
    NorthStar,
    /// Medical / pre-participation (PAR-Q) flag the coach must heed. Kept
    /// distinct from [`Self::Injury`] so it can be redacted/gated separately.
    Medical,
    /// Catch-all for semantically meaningful facts that don't fit elsewhere.
    Other,
}

impl FactKind {
    /// Stable string identifier used in DB serialization and extraction prompts.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::Physiology => "physiology",
            Self::Injury => "injury",
            Self::Goal => "goal",
            Self::Schedule => "schedule",
            Self::Equipment => "equipment",
            Self::NorthStar => "north_star",
            Self::Medical => "medical",
            Self::Other => "other",
        }
    }

    /// Parse from the DB string form. Unknown values fall back to [`Self::Other`]
    /// so older rows don't panic during migration.
    #[must_use]
    pub fn parse_lenient(s: &str) -> Self {
        match s {
            "preference" => Self::Preference,
            "physiology" => Self::Physiology,
            "injury" => Self::Injury,
            "goal" => Self::Goal,
            "schedule" => Self::Schedule,
            "equipment" => Self::Equipment,
            "north_star" => Self::NorthStar,
            "medical" => Self::Medical,
            _ => Self::Other,
        }
    }
}

/// Provenance of a [`UserFact`] — where the claim came from.
///
/// Lets retrieval and the conversation-update loop reason about trust and
/// supersession (e.g. a fresh `Onboarding` answer overrides a stale
/// `Conversation` inference). New sources should be additive only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactSource {
    /// Captured during the conversational pillar onboarding flow.
    Onboarding,
    /// Inferred by the background extraction worker from ongoing chat.
    Conversation,
    /// Pre-filled from connected device/provider data.
    Device,
    /// Written intentionally by a coach tool.
    Coach,
}

impl FactSource {
    /// Stable string identifier used in DB serialization.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Onboarding => "onboarding",
            Self::Conversation => "conversation",
            Self::Device => "device",
            Self::Coach => "coach",
        }
    }

    /// Parse from the DB string form. Unknown values fall back to
    /// [`Self::Conversation`] — the column is `NOT NULL DEFAULT 'conversation'`,
    /// so a decode must never panic on an unexpected value.
    #[must_use]
    pub fn parse_lenient(s: &str) -> Self {
        match s {
            "onboarding" => Self::Onboarding,
            "device" => Self::Device,
            "coach" => Self::Coach,
            _ => Self::Conversation,
        }
    }
}

/// The closed vocabulary of what a fact says about the athlete.
///
/// A fact used to carry a free-text `predicate` the extraction LLM wrote in
/// English, and every renderer glued it to the object as a sentence — so a
/// French athlete read "are training for un ultra de 26 km" in her own
/// memory screen and her coach's prompt. The predicate is now one of these
/// codes, the object is the athlete's own words, and the sentence is rendered
/// once per locale from the string catalogue (`messaging.memory.predicate.<code>`,
/// `{0}` = object). [`Self::States`] is the honest catch-all on every kind: its
/// sentence is the object alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PredicateCode {
    /// Goal: a race or event the athlete is preparing for.
    TrainingFor,
    /// Goal: a target the athlete named without an event (onboarding "goal").
    WorkingToward,
    /// Goal: the target race a saved training plan converges on.
    TargetRace,
    /// Preference: something the athlete wants more of.
    Prefer,
    /// Preference: something the athlete wants none of.
    Avoid,
    /// Preference: the athlete's main sport (onboarding).
    PrimarilyTrain,
    /// Physiology: a baseline number (resting HR, weight, FTP).
    HaveBaseline,
    /// Injury: a current injury, pain or constraint.
    Have,
    /// Injury: something the athlete is coming back from.
    RecoveringFrom,
    /// Schedule: a day or slot the athlete can train.
    CanTrainOn,
    /// Schedule: a day or slot the athlete cannot train.
    CannotTrainOn,
    /// Schedule: a session that must fall on a given day.
    NeedSessionOn,
    /// Schedule: a blackout period.
    Unavailable,
    /// Equipment: kit the athlete owns.
    Own,
    /// Equipment: what the athlete trains on.
    TrainOn,
    /// North star: why the athlete trains at all (onboarding).
    TrainBecause,
    /// Medical: a "yes" on a PAR-Q question; the object is the question.
    ParqYes,
    /// Medical: a flag a coach tool raised.
    Flagged,
    /// Any kind: the athlete's own words, with no verb of ours in front.
    States,
}

impl PredicateCode {
    /// Every code, in a stable order (the order the catalogue lists them).
    pub const ALL: [Self; 19] = [
        Self::TrainingFor,
        Self::WorkingToward,
        Self::TargetRace,
        Self::Prefer,
        Self::Avoid,
        Self::PrimarilyTrain,
        Self::HaveBaseline,
        Self::Have,
        Self::RecoveringFrom,
        Self::CanTrainOn,
        Self::CannotTrainOn,
        Self::NeedSessionOn,
        Self::Unavailable,
        Self::Own,
        Self::TrainOn,
        Self::TrainBecause,
        Self::ParqYes,
        Self::Flagged,
        Self::States,
    ];

    /// Stable string identifier used in the database, the wire and the
    /// extraction prompt.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TrainingFor => "training_for",
            Self::WorkingToward => "working_toward",
            Self::TargetRace => "target_race",
            Self::Prefer => "prefer",
            Self::Avoid => "avoid",
            Self::PrimarilyTrain => "primarily_train",
            Self::HaveBaseline => "have_baseline",
            Self::Have => "have",
            Self::RecoveringFrom => "recovering_from",
            Self::CanTrainOn => "can_train_on",
            Self::CannotTrainOn => "cannot_train_on",
            Self::NeedSessionOn => "need_session_on",
            Self::Unavailable => "unavailable",
            Self::Own => "own",
            Self::TrainOn => "train_on",
            Self::TrainBecause => "train_because",
            Self::ParqYes => "parq_yes",
            Self::Flagged => "flagged",
            Self::States => "states",
        }
    }

    /// Parse the stable identifier. Strict: an unknown code is `None`, never
    /// silently `States` — a tool or a row naming a code we do not have is a
    /// bug to surface, not a fact to keep.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|code| code.as_str() == s)
    }

    /// Whether this code may be written under `kind`. [`Self::States`] is
    /// allowed on every kind; every other code belongs to exactly one.
    #[must_use]
    pub const fn allowed_for(self, kind: FactKind) -> bool {
        matches!(
            (self, kind),
            (Self::States, _)
                | (
                    Self::TrainingFor | Self::WorkingToward | Self::TargetRace,
                    FactKind::Goal
                )
                | (
                    Self::Prefer | Self::Avoid | Self::PrimarilyTrain,
                    FactKind::Preference
                )
                | (Self::HaveBaseline, FactKind::Physiology)
                | (Self::Have | Self::RecoveringFrom, FactKind::Injury)
                | (
                    Self::CanTrainOn
                        | Self::CannotTrainOn
                        | Self::NeedSessionOn
                        | Self::Unavailable,
                    FactKind::Schedule
                )
                | (Self::Own | Self::TrainOn, FactKind::Equipment)
                | (Self::TrainBecause, FactKind::NorthStar)
                | (Self::ParqYes | Self::Flagged, FactKind::Medical)
        )
    }

    /// What the code means, in the words the extraction prompt shows the
    /// model. Lives beside the code so the prompt's vocabulary is generated
    /// from this enum and can never name a code the parser rejects.
    #[must_use]
    pub const fn gloss(self) -> &'static str {
        match self {
            Self::TrainingFor => "a race or event the athlete is preparing for",
            Self::WorkingToward => "a target the athlete named without an event",
            Self::TargetRace => "the target race a saved training plan converges on",
            Self::Prefer => "something the athlete wants more of",
            Self::Avoid => "something the athlete wants none of",
            Self::PrimarilyTrain => "the athlete's main sport",
            Self::HaveBaseline => "a baseline number: resting HR, weight, FTP",
            Self::Have => "a current injury, pain or constraint",
            Self::RecoveringFrom => "something the athlete is coming back from",
            Self::CanTrainOn => "a day or slot the athlete can train",
            Self::CannotTrainOn => "a day or slot the athlete cannot train",
            Self::NeedSessionOn => "a session that must fall on a given day",
            Self::Unavailable => "a blackout period",
            Self::Own => "kit the athlete owns",
            Self::TrainOn => "what the athlete trains on",
            Self::TrainBecause => "why the athlete trains at all",
            Self::ParqYes => "a yes on a PAR-Q question; the object is the question",
            Self::Flagged => "a flag a coach tool raised",
            Self::States => "the athlete's own words, when no other code fits",
        }
    }

    /// Whether the extraction model may pick this code. [`Self::TargetRace`]
    /// is minted by `save_training_plan` when a plan converges on a race,
    /// [`Self::ParqYes`] by the PAR-Q screen and [`Self::Flagged`] by coach
    /// tools; each passes [`Self::allowed_for`] on its kind, so without this
    /// gate the model could pass a chat remark off as a tool's work.
    #[must_use]
    pub const fn extractable(self) -> bool {
        !matches!(self, Self::TargetRace | Self::ParqYes | Self::Flagged)
    }

    /// The code an English predicate phrase from the pre-code era maps to.
    ///
    /// Only the seven phrases the server itself used to author are known; an
    /// extractor phrase from an old prompt is `None` and becomes
    /// [`Self::States`] with the phrase folded into the object, so nothing
    /// the athlete said is lost and nothing pretends to be structured.
    #[must_use]
    pub fn legacy_from_phrase(phrase: &str) -> Option<Self> {
        match phrase.trim() {
            "train because" => Some(Self::TrainBecause),
            "primarily train" => Some(Self::PrimarilyTrain),
            "are working toward" | "want" => Some(Self::WorkingToward),
            "answered yes (PAR-Q)" => Some(Self::ParqYes),
            "target race" => Some(Self::TargetRace),
            "flagged" => Some(Self::Flagged),
            _ => None,
        }
    }

    /// The catalogue key whose text renders this code as a sentence
    /// (`{0}` = the object).
    #[must_use]
    pub const fn catalogue_key(self) -> &'static str {
        match self {
            Self::TrainingFor => "messaging.memory.predicate.training_for",
            Self::WorkingToward => "messaging.memory.predicate.working_toward",
            Self::TargetRace => "messaging.memory.predicate.target_race",
            Self::Prefer => "messaging.memory.predicate.prefer",
            Self::Avoid => "messaging.memory.predicate.avoid",
            Self::PrimarilyTrain => "messaging.memory.predicate.primarily_train",
            Self::HaveBaseline => "messaging.memory.predicate.have_baseline",
            Self::Have => "messaging.memory.predicate.have",
            Self::RecoveringFrom => "messaging.memory.predicate.recovering_from",
            Self::CanTrainOn => "messaging.memory.predicate.can_train_on",
            Self::CannotTrainOn => "messaging.memory.predicate.cannot_train_on",
            Self::NeedSessionOn => "messaging.memory.predicate.need_session_on",
            Self::Unavailable => "messaging.memory.predicate.unavailable",
            Self::Own => "messaging.memory.predicate.own",
            Self::TrainOn => "messaging.memory.predicate.train_on",
            Self::TrainBecause => "messaging.memory.predicate.train_because",
            Self::ParqYes => "messaging.memory.predicate.parq_yes",
            Self::Flagged => "messaging.memory.predicate.flagged",
            Self::States => "messaging.memory.predicate.states",
        }
    }
}

/// A structured claim the harness has extracted about a user.
///
/// Facts are the unit of semantic memory. They are tenant-scoped, have
/// provenance back to a source message, and carry a confidence score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserFact {
    /// Stable identifier for this fact.
    pub id: String,
    /// Tenant that owns the fact.
    pub tenant_id: String,
    /// User the fact is about.
    pub user_id: String,
    /// Coach that the fact was collected for, if any. `None` means the fact
    /// is user-wide across coaches.
    pub coach_id: Option<String>,
    /// Scope bucket (conversation / user / tenant).
    pub scope: MemoryScope,
    /// Semantic category.
    pub kind: FactKind,
    /// Which of the six health pillars this fact belongs to. `None` for facts
    /// that are pillar-agnostic (e.g. North Star, medical flags, or older rows
    /// recorded before pillar tagging).
    pub pillar: Option<Pillar>,
    /// What the fact says about the athlete, as a closed code.
    pub predicate_code: PredicateCode,
    /// The athlete's own words for the value being asserted, in their language.
    pub object: String,
    /// Extractor confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// Where this fact came from (onboarding / conversation / device / coach).
    pub source: FactSource,
    /// Freshness horizon: after this instant the fact is considered stale and
    /// is demoted/flagged at render time. `None` means no expiry.
    pub valid_until: Option<DateTime<Utc>>,
    /// ID of the message the fact was extracted from, for provenance.
    pub source_msg_id: Option<String>,
    /// When the fact was first recorded.
    pub created_at: DateTime<Utc>,
    /// When the fact was last touched (updated / merged / confidence decayed).
    pub updated_at: DateTime<Utc>,
}

impl UserFact {
    /// Short-hand check to see if this fact is still confident enough to be
    /// injected into the next prompt. Callers pick the threshold; this
    /// helper just avoids duplicating the comparison.
    #[must_use]
    pub fn is_confident(&self, min: f32) -> bool {
        self.confidence >= min
    }
}

/// Aggregate health snapshot for the memory extraction worker.
///
/// The extraction worker is a fire-and-forget background task, so the health
/// of the pipeline is best observed by looking at the rows it has actually
/// produced in `user_facts` rather than instrumenting the worker itself.
///
/// Counters use `u64` so JSON marshaling matches the database column shape
/// (`COUNT(*)` returns an `i64` that we widen) and all-zero fields stay
/// readable in the UI without sentinel values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserFactMetrics {
    /// Total number of facts stored for the tenant across all users/coaches.
    pub total_facts: u64,
    /// Facts inserted or updated in the last 24 hours — the "recent activity"
    /// signal that tells operators the worker is still extracting.
    pub facts_last_24h: u64,
    /// Facts inserted or updated in the last 7 days — the "healthy baseline"
    /// signal that tells operators the worker produced anything this week.
    pub facts_last_7d: u64,
    /// Distinct users the tenant has facts for. Useful when comparing with the
    /// tenant's active-user count to spot extraction gaps.
    pub distinct_users: u64,
    /// Count per `FactKind` string (e.g. `preference`, `physiology`, ...).
    /// A `BTreeMap` keeps JSON serialization order stable across calls, which
    /// makes the admin tab table render deterministic.
    pub facts_by_kind: BTreeMap<String, u64>,
    /// Timestamp of the most recently updated fact, or `None` when the tenant
    /// has no facts yet. Drives the "last activity" badge in the admin UI.
    pub newest_updated_at: Option<DateTime<Utc>>,
}

impl UserFactMetrics {
    /// Convenience constructor for the empty snapshot — no facts yet.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            total_facts: 0,
            facts_last_24h: 0,
            facts_last_7d: 0,
            distinct_users: 0,
            facts_by_kind: BTreeMap::new(),
            newest_updated_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FactKind, FactSource, MemoryScope, PredicateCode, UserFact};
    use chrono::Utc;

    #[test]
    fn fact_kind_roundtrip() {
        for kind in [
            FactKind::Preference,
            FactKind::Physiology,
            FactKind::Injury,
            FactKind::Goal,
            FactKind::Schedule,
            FactKind::Equipment,
            FactKind::NorthStar,
            FactKind::Medical,
            FactKind::Other,
        ] {
            assert_eq!(FactKind::parse_lenient(kind.as_str()), kind);
        }
    }

    #[test]
    fn fact_source_roundtrip() {
        for source in [
            FactSource::Onboarding,
            FactSource::Conversation,
            FactSource::Device,
            FactSource::Coach,
        ] {
            assert_eq!(FactSource::parse_lenient(source.as_str()), source);
        }
        // Unknown values default to Conversation (NOT NULL DEFAULT in the DB).
        assert_eq!(
            FactSource::parse_lenient("garbage"),
            FactSource::Conversation
        );
    }

    #[test]
    fn unknown_kind_falls_back_to_other() {
        assert_eq!(FactKind::parse_lenient("hallucinated"), FactKind::Other);
    }

    #[test]
    fn is_confident_threshold() {
        let now = Utc::now();
        let fact = UserFact {
            id: "f1".into(),
            tenant_id: "t1".into(),
            user_id: "u1".into(),
            coach_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Goal,
            pillar: None,
            predicate_code: PredicateCode::WorkingToward,
            object: "sub-3 marathon".into(),
            confidence: 0.72,
            source: FactSource::Conversation,
            valid_until: None,
            source_msg_id: Some("m1".into()),
            created_at: now,
            updated_at: now,
        };
        assert!(fact.is_confident(0.7));
        assert!(!fact.is_confident(0.8));
    }
}
