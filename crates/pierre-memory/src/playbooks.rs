// ABOUTME: Procedural coaching memory — playbooks (trigger -> intervention -> outcome) and pending advice
// ABOUTME: Pure types. The policy/learning layer over the capability state model (see ADR-007).
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Procedural / playbook coaching memory.
//!
//! Where [`crate::facts::UserFact`] remembers *what is true about a user*, a
//! [`Playbook`] remembers *what coaching actually works on them*: a structured
//! `trigger -> intervention` pair plus reinforcement counters fed by automatic
//! outcome labels derived from the activity & health data the platform already
//! syncs. [`PendingAdvice`] is the in-flight record between "the coach gave
//! advice" and "we observed whether it worked".
//!
//! These types are deliberately structured (enums + bands), not free text, so
//! the same vocabulary can be matched at retrieval time, hashed for the upsert
//! conflict key, and rendered into a prompt without re-parsing prose.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The situation that prompted a coaching intervention.
///
/// Unit variants only — the magnitude lives in [`TriggerPattern::magnitude`]
/// and the metric specifics in [`OutcomeMetric`], so this stays a stable,
/// hashable vocabulary shared by capture, retrieval, and the prompt renderer.
/// New kinds must be additive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerKind {
    /// Adherence/motivation is slipping (missed or skipped planned sessions).
    MotivationDip,
    /// Recovery is degrading (HRV trending down, resting HR up).
    HrvDrop,
    /// Training load is ramping fast (ACWR / ramp-rate elevated).
    LoadRamp,
    /// A target metric has stalled for an extended window.
    Plateau,
    /// A travel / routine-disruption window (data gap + context signal).
    Travel,
    /// A pre-planned key session or block boundary.
    PrePlanned,
    /// Catch-all for a meaningful trigger that doesn't fit elsewhere.
    Other,
}

impl TriggerKind {
    /// Stable string identifier for DB serialization and hashing.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MotivationDip => "motivation_dip",
            Self::HrvDrop => "hrv_drop",
            Self::LoadRamp => "load_ramp",
            Self::Plateau => "plateau",
            Self::Travel => "travel",
            Self::PrePlanned => "pre_planned",
            Self::Other => "other",
        }
    }

    /// Parse from the DB string form. Unknown values fall back to [`Self::Other`].
    #[must_use]
    pub fn parse_lenient(s: &str) -> Self {
        match s {
            "motivation_dip" => Self::MotivationDip,
            "hrv_drop" => Self::HrvDrop,
            "load_ramp" => Self::LoadRamp,
            "plateau" => Self::Plateau,
            "travel" => Self::Travel,
            "pre_planned" => Self::PrePlanned,
            _ => Self::Other,
        }
    }
}

/// Coarse magnitude band for a trigger or intervention.
///
/// Bands (not raw numbers) keep playbooks generalizable: "a *large* HRV drop"
/// matches across athletes where an exact millisecond value would not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Band {
    /// Mild deviation.
    Low,
    /// Moderate deviation.
    Moderate,
    /// Strong deviation.
    High,
}

impl Band {
    /// Stable string identifier for DB serialization and hashing.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Moderate => "moderate",
            Self::High => "high",
        }
    }

    /// Parse from the DB string form. Unknown values fall back to
    /// [`Self::Moderate`] — the neutral middle band.
    #[must_use]
    pub fn parse_lenient(s: &str) -> Self {
        match s {
            "low" => Self::Low,
            "high" => Self::High,
            _ => Self::Moderate,
        }
    }
}

/// The matchable context that prompted an intervention.
///
/// `sport` is a provider-agnostic sport slug (e.g. `"run"`), kept as a string
/// to avoid coupling `pierre-memory` to the activity model and to mirror the
/// `TEXT` storage column. `None` means sport-agnostic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerPattern {
    /// What kind of situation this is.
    pub kind: TriggerKind,
    /// Sport slug the trigger applies to, or `None` for any sport.
    pub sport: Option<String>,
    /// How pronounced the trigger was.
    pub magnitude: Band,
}

impl TriggerPattern {
    /// Deterministic stable key for this trigger, used as part of the
    /// `coaching_playbooks` upsert conflict key. Order-stable and allocation-
    /// light so equivalent triggers collapse onto the same playbook row.
    #[must_use]
    pub fn hash_key(&self) -> String {
        format!(
            "{}:{}:{}",
            self.kind.as_str(),
            self.sport.as_deref().unwrap_or("*"),
            self.magnitude.as_str()
        )
    }
}

/// The coaching action taken in response to a [`TriggerPattern`].
///
/// Unit variants only; numeric detail (days, sessions/week) lives in
/// [`Intervention::magnitude`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InterventionKind {
    /// Prescribe an easy/aerobic block instead of rest or intensity.
    EasyBlock,
    /// Add one or more tempo sessions per week.
    AddTempo,
    /// Add one or more threshold sessions per week.
    AddThreshold,
    /// Prescribe a "minimum viable" session to preserve the habit.
    MinimumViable,
    /// Cap or reduce weekly volume.
    ReduceVolume,
    /// Prescribe a full rest day.
    RestDay,
    /// Switch to a terse, encouragement-first communication style.
    CommStyleTerse,
    /// Switch to an analytical, physiology-forward communication style.
    CommStyleAnalytical,
    /// Catch-all intervention.
    Other,
}

impl InterventionKind {
    /// Stable string identifier for DB serialization and hashing.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EasyBlock => "easy_block",
            Self::AddTempo => "add_tempo",
            Self::AddThreshold => "add_threshold",
            Self::MinimumViable => "minimum_viable",
            Self::ReduceVolume => "reduce_volume",
            Self::RestDay => "rest_day",
            Self::CommStyleTerse => "comm_style_terse",
            Self::CommStyleAnalytical => "comm_style_analytical",
            Self::Other => "other",
        }
    }

    /// Parse from the DB string form. Unknown values fall back to [`Self::Other`].
    #[must_use]
    pub fn parse_lenient(s: &str) -> Self {
        match s {
            "easy_block" => Self::EasyBlock,
            "add_tempo" => Self::AddTempo,
            "add_threshold" => Self::AddThreshold,
            "minimum_viable" => Self::MinimumViable,
            "reduce_volume" => Self::ReduceVolume,
            "rest_day" => Self::RestDay,
            "comm_style_terse" => Self::CommStyleTerse,
            "comm_style_analytical" => Self::CommStyleAnalytical,
            _ => Self::Other,
        }
    }
}

/// A structured coaching action with an optional numeric magnitude.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Intervention {
    /// What action was taken.
    pub kind: InterventionKind,
    /// Numeric parameter interpreted per `kind` (e.g. days for `EasyBlock`,
    /// sessions/week for `AddTempo`). `None` when the kind needs no magnitude.
    pub magnitude: Option<i32>,
}

impl Intervention {
    /// Deterministic stable key for this intervention, used as part of the
    /// `coaching_playbooks` upsert conflict key.
    #[must_use]
    pub fn hash_key(&self) -> String {
        self.magnitude.map_or_else(
            || format!("{}:*", self.kind.as_str()),
            |m| format!("{}:{m}", self.kind.as_str()),
        )
    }
}

/// What the labeler measures to decide whether an intervention worked.
///
/// This is the pluggable seam between the playbook engine and the data layer:
/// v1 variants read what is persisted today (`cached_activities`,
/// `recovery_metrics`, `training_history`). When the [[ADR-007]] Capability
/// Engine ships, a `CapabilityDelta { id, window_days }` variant slots in here
/// as a first-class outcome source with no change to capture, storage, or
/// retrieval — that future variant is intentionally **not** added until a
/// capability score exists to read (no dead variants).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeMetric {
    /// Did a matching activity get completed within the window? Adherence signal.
    ActivityCompleted {
        /// How many days after the advice to look for the activity.
        window_days: u8,
        /// Sport slug the activity must match, or `None` for any.
        sport: Option<String>,
    },
    /// Did HRV improve over the window (from `recovery_metrics`)?
    HrvDelta {
        /// Observation window in days.
        window_days: u8,
    },
    /// Did training-stress balance (form) move favorably (from `training_history`)?
    TsbDelta {
        /// Observation window in days.
        window_days: u8,
    },
    /// Did the load ramp rate stay under a safe ceiling (injury-avoidance)?
    RampRateWithin {
        /// Maximum acceptable ramp rate.
        ceiling: f32,
    },
    /// Did training consistency (sessions/week) hold or improve?
    Consistency {
        /// Observation window in days.
        window_days: u8,
    },
}

impl OutcomeMetric {
    /// Stable discriminant string for DB indexing / logs.
    #[must_use]
    pub const fn kind_str(&self) -> &'static str {
        match self {
            Self::ActivityCompleted { .. } => "activity_completed",
            Self::HrvDelta { .. } => "hrv_delta",
            Self::TsbDelta { .. } => "tsb_delta",
            Self::RampRateWithin { .. } => "ramp_rate_within",
            Self::Consistency { .. } => "consistency",
        }
    }
}

/// Provenance anchor for a piece of advice: when its baseline was captured.
///
/// Delta metrics (HRV/TSB) recompute their own before/after endpoints from live
/// history at evaluation time rather than from a stored value, so only the
/// capture timestamp is retained.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricBaseline {
    /// When the baseline was captured.
    pub captured_at: DateTime<Utc>,
}

/// The verdict the labeler assigns to a piece of advice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeLabel {
    /// The intervention demonstrably worked (metric moved favorably / adhered).
    Success,
    /// The intervention did not work (metric worsened / not adhered).
    Failure,
    /// No clear signal either way — explicitly distinct from failure so a
    /// playbook can't win or lose by the user simply doing nothing.
    Neutral,
}

impl OutcomeLabel {
    /// Stable string identifier for DB serialization.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
            Self::Neutral => "neutral",
        }
    }

    /// Parse from the DB string form. Unknown values fall back to
    /// [`Self::Neutral`] — the safe, non-reinforcing default.
    #[must_use]
    pub fn parse_lenient(s: &str) -> Self {
        match s {
            "success" => Self::Success,
            "failure" => Self::Failure,
            _ => Self::Neutral,
        }
    }
}

/// Which mechanism produced an [`OutcomeLabel`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LabelSource {
    /// Labeled by deterministic data rules (free, reproducible).
    DataHeuristic,
    /// Labeled by the bounded LLM judge (ambiguous cases only).
    LlmJudge,
}

impl LabelSource {
    /// Stable string identifier for DB serialization.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DataHeuristic => "data_heuristic",
            Self::LlmJudge => "llm_judge",
        }
    }

    /// Parse from the DB string form. Unknown values fall back to
    /// [`Self::DataHeuristic`].
    #[must_use]
    pub fn parse_lenient(s: &str) -> Self {
        match s {
            "llm_judge" => Self::LlmJudge,
            _ => Self::DataHeuristic,
        }
    }
}

/// Lifecycle state of a [`PendingAdvice`] row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdviceStatus {
    /// Awaiting its observation window to close.
    Pending,
    /// Observed and labeled; rolled into a playbook's counters.
    Labeled,
    /// Window closed without enough data to label; not reinforcing.
    Expired,
}

impl AdviceStatus {
    /// Stable string identifier for DB serialization.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Labeled => "labeled",
            Self::Expired => "expired",
        }
    }

    /// Parse from the DB string form. Unknown values fall back to
    /// [`Self::Pending`].
    #[must_use]
    pub fn parse_lenient(s: &str) -> Self {
        match s {
            "labeled" => Self::Labeled,
            "expired" => Self::Expired,
            _ => Self::Pending,
        }
    }
}

/// Max length of an accepted sport slug — a real slug (`run`, `bike_ride`,
/// `cross_country_skiing`) fits comfortably; anything longer is not slug-shaped.
pub const MAX_SPORT_SLUG_LEN: usize = 32;

/// Narrow a caller-supplied sport to a bounded `[a-z0-9_]` slug, or `None`.
///
/// `sport` is the only free-text field in the trigger/outcome vocabulary, and
/// it reaches the coach's system prompt verbatim through the playbook and
/// commitment blocks. Constraining it here is why those renderers need no
/// further fencing — "Ignore prior instructions", `RUN`, `run!` and `trail run`
/// all reduce to sport-agnostic rather than reaching a prompt.
///
/// Shared by every writer of a stored sport slug so the guarantee holds at one
/// place instead of once per caller.
#[must_use]
pub fn sanitize_sport_slug(sport: Option<&str>) -> Option<String> {
    sport
        .map(str::trim)
        .filter(|s| {
            !s.is_empty()
                && s.len() <= MAX_SPORT_SLUG_LEN
                && s.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
        .map(str::to_owned)
}

/// z-score for a 95% one-sided Wilson confidence interval.
const WILSON_Z_95: f64 = 1.959_963_984_540_054;

/// Wilson score interval lower bound (one-sided, 95%) of the success rate over
/// `success + failure` decisive outcomes.
///
/// Returns `0.0` when there are none. Penalizes small samples so a
/// well-evidenced rate outranks a lucky one-off. Shared by [`Playbook`] and
/// [`ArchetypePrior`] so confidence is computed identically everywhere.
// The result is clamped to [0,1] before narrowing, so neither precision loss
// nor truncation is meaningful for this f64 -> f32 conversion. Counters are
// widened to f64 BEFORE summing so a corrupt counter cannot wrap the addition.
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
#[must_use]
pub fn wilson_lower_bound_95(success: u32, failure: u32) -> f32 {
    let n = f64::from(success) + f64::from(failure);
    if n == 0.0 {
        return 0.0;
    }
    let phat = f64::from(success) / n;
    let z = WILSON_Z_95;
    let z2 = z * z;
    let denom = 1.0 + z2 / n;
    let center = phat + z2 / (2.0 * n);
    let margin = z * (phat.mul_add(1.0 - phat, z2 / (4.0 * n)) / n).sqrt();
    let lower = (center - margin) / denom;
    lower.clamp(0.0, 1.0) as f32
}

/// A learned coaching playbook: a `trigger -> intervention` pair plus the
/// reinforcement counters and confidence that say how well it has worked.
///
/// Tenant-scoped and optionally coach-scoped. The `(tenant, user, coach,
/// trigger, intervention)` tuple is unique — repeated outcomes increment the
/// counters on the same row rather than inserting duplicates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Playbook {
    /// Stable identifier for this playbook.
    pub id: String,
    /// Tenant that owns the playbook.
    pub tenant_id: String,
    /// User the playbook is personalized for.
    pub user_id: String,
    /// Coach persona slug this playbook was learned under, or `None` for
    /// user-wide (coach-agnostic) playbooks.
    pub coach_slug: Option<String>,
    /// The situation the playbook responds to.
    pub trigger: TriggerPattern,
    /// The action the playbook prescribes.
    pub intervention: Intervention,
    /// What the labeler measures to score this playbook.
    pub outcome_metric: OutcomeMetric,
    /// Number of times the intervention worked.
    pub success_count: u32,
    /// Number of times it did not.
    pub failure_count: u32,
    /// Number of times the outcome was neutral (no clear signal).
    pub neutral_count: u32,
    /// Wilson lower-bound of the success rate — the value retrieval ranks on.
    /// Penalizes small samples so a 1/1 playbook never outranks an 18/20 one.
    pub confidence: f32,
    /// When the most recent outcome landed, or `None` if never labeled.
    pub last_outcome_at: Option<DateTime<Utc>>,
    /// When the playbook was first created.
    pub created_at: DateTime<Utc>,
    /// When the playbook was last updated (counter increment / recompute).
    pub updated_at: DateTime<Utc>,
}

impl Playbook {
    /// Total labeled outcomes (success + failure + neutral). Saturating so a
    /// corrupt counter read from storage can never wrap to a small total.
    #[must_use]
    pub const fn total_outcomes(&self) -> u32 {
        self.success_count
            .saturating_add(self.failure_count)
            .saturating_add(self.neutral_count)
    }

    /// Raw success rate over **decisive** outcomes (success + failure),
    /// ignoring neutrals. `0.0` when there are no decisive outcomes yet.
    // Counts stay well under 2^24, so these u32 -> f32 conversions are exact.
    #[allow(clippy::cast_precision_loss)]
    #[must_use]
    pub fn success_rate(&self) -> f32 {
        let decisive = self.success_count.saturating_add(self.failure_count);
        if decisive == 0 {
            0.0
        } else {
            self.success_count as f32 / decisive as f32
        }
    }

    /// Wilson score interval lower bound (one-sided, 95%) of the success rate
    /// over decisive outcomes. This is the principled "confidence" that lets a
    /// well-evidenced playbook outrank a lucky small-sample one. Returns `0.0`
    /// when there are no decisive outcomes.
    ///
    /// Neutrals are excluded from the denominator so a playbook is neither
    /// rewarded nor punished for the user doing nothing.
    #[must_use]
    pub fn wilson_lower_bound(&self) -> f32 {
        wilson_lower_bound_95(self.success_count, self.failure_count)
    }

    /// Apply a freshly-observed outcome: increment the matching counter,
    /// recompute [`Self::confidence`], and stamp `last_outcome_at`/`updated_at`.
    pub fn record_outcome(&mut self, label: OutcomeLabel, at: DateTime<Utc>) {
        match label {
            OutcomeLabel::Success => self.success_count += 1,
            OutcomeLabel::Failure => self.failure_count += 1,
            OutcomeLabel::Neutral => self.neutral_count += 1,
        }
        self.confidence = self.wilson_lower_bound();
        self.last_outcome_at = Some(at);
        self.updated_at = at;
    }
}

/// An in-flight piece of advice awaiting its outcome.
///
/// Created when the coach gives a concrete recommendation; resolved by the
/// outcome evaluator once `due_by` passes and the data window can be read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingAdvice {
    /// Stable identifier for this advice record.
    pub id: String,
    /// Tenant that owns the advice.
    pub tenant_id: String,
    /// User the advice was given to.
    pub user_id: String,
    /// Coach persona slug that gave the advice, or `None`.
    pub coach_slug: Option<String>,
    /// The playbook this advice instantiates, if it matched an existing one.
    /// `None` for novel `trigger -> intervention` pairs (a playbook is created
    /// on first labeling).
    pub playbook_id: Option<String>,
    /// The situation that prompted the advice.
    pub trigger: TriggerPattern,
    /// The action recommended.
    pub intervention: Intervention,
    /// What to measure to score it.
    pub outcome_metric: OutcomeMetric,
    /// Metric baseline captured at advice time.
    pub baseline: MetricBaseline,
    /// When the observation window closes and the evaluator may label it.
    pub due_by: DateTime<Utc>,
    /// Lifecycle state.
    pub status: AdviceStatus,
    /// The assigned label once observed; `None` while pending.
    pub label: Option<OutcomeLabel>,
    /// Which mechanism labeled it; `None` while pending.
    pub label_source: Option<LabelSource>,
    /// The assistant message the advice was extracted from, for provenance.
    pub source_msg_id: Option<String>,
    /// When the advice was captured.
    pub created_at: DateTime<Utc>,
}

/// A k-anonymous, non-tenant aggregate: how an intervention has worked across
/// many athletes of one archetype (e.g. all runners).
///
/// This is the cold-start prior a new athlete inherits until their own outcomes
/// accumulate. It holds **counts only** — never user or tenant identity — and is
/// only materialized for archetypes with at least `K` distinct contributing
/// users (enforced by the aggregation job). This is the sole non-tenant store in
/// the system; see the `archetype_priors` migration for the privacy rationale.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchetypePrior {
    /// Non-identifying archetype bucket (v1: the sport, or `"any"`).
    pub archetype_key: String,
    /// The situation this prior responds to.
    pub trigger: TriggerPattern,
    /// The intervention this prior is about.
    pub intervention: Intervention,
    /// Total successes summed across all contributing athletes.
    pub success_count: u32,
    /// Total failures summed across all contributing athletes.
    pub failure_count: u32,
    /// Number of distinct athletes who contributed — the k-anonymity guard.
    pub distinct_user_count: u32,
    /// Wilson lower-bound confidence, computed on read from the counts.
    pub confidence: f32,
}

impl ArchetypePrior {
    /// Wilson lower-bound confidence over the decisive aggregate outcomes.
    #[must_use]
    pub fn wilson_lower_bound(&self) -> f32 {
        wilson_lower_bound_95(self.success_count, self.failure_count)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AdviceStatus, Band, InterventionKind, LabelSource, OutcomeLabel, OutcomeMetric, Playbook,
        TriggerKind, TriggerPattern,
    };
    use chrono::Utc;

    #[test]
    fn enum_string_roundtrips() {
        for k in [
            TriggerKind::MotivationDip,
            TriggerKind::HrvDrop,
            TriggerKind::LoadRamp,
            TriggerKind::Plateau,
            TriggerKind::Travel,
            TriggerKind::PrePlanned,
            TriggerKind::Other,
        ] {
            assert_eq!(TriggerKind::parse_lenient(k.as_str()), k);
        }
        for b in [Band::Low, Band::Moderate, Band::High] {
            assert_eq!(Band::parse_lenient(b.as_str()), b);
        }
        for i in [
            InterventionKind::EasyBlock,
            InterventionKind::AddTempo,
            InterventionKind::AddThreshold,
            InterventionKind::MinimumViable,
            InterventionKind::ReduceVolume,
            InterventionKind::RestDay,
            InterventionKind::CommStyleTerse,
            InterventionKind::CommStyleAnalytical,
            InterventionKind::Other,
        ] {
            assert_eq!(InterventionKind::parse_lenient(i.as_str()), i);
        }
        for l in [
            OutcomeLabel::Success,
            OutcomeLabel::Failure,
            OutcomeLabel::Neutral,
        ] {
            assert_eq!(OutcomeLabel::parse_lenient(l.as_str()), l);
        }
        for s in [LabelSource::DataHeuristic, LabelSource::LlmJudge] {
            assert_eq!(LabelSource::parse_lenient(s.as_str()), s);
        }
        for st in [
            AdviceStatus::Pending,
            AdviceStatus::Labeled,
            AdviceStatus::Expired,
        ] {
            assert_eq!(AdviceStatus::parse_lenient(st.as_str()), st);
        }
    }

    #[test]
    fn unknown_enum_values_fall_back_safely() {
        assert_eq!(TriggerKind::parse_lenient("nope"), TriggerKind::Other);
        assert_eq!(Band::parse_lenient("nope"), Band::Moderate);
        assert_eq!(
            InterventionKind::parse_lenient("nope"),
            InterventionKind::Other
        );
        // The reinforcement-sensitive defaults must be the non-reinforcing ones.
        assert_eq!(OutcomeLabel::parse_lenient("nope"), OutcomeLabel::Neutral);
        assert_eq!(AdviceStatus::parse_lenient("nope"), AdviceStatus::Pending);
    }

    #[test]
    fn trigger_and_intervention_hash_keys_are_stable() {
        let t = TriggerPattern {
            kind: TriggerKind::HrvDrop,
            sport: Some("run".into()),
            magnitude: Band::High,
        };
        assert_eq!(t.hash_key(), "hrv_drop:run:high");
        let t_any = TriggerPattern {
            kind: TriggerKind::HrvDrop,
            sport: None,
            magnitude: Band::High,
        };
        assert_eq!(t_any.hash_key(), "hrv_drop:*:high");
    }

    #[test]
    fn outcome_metric_kind_str() {
        let m = OutcomeMetric::ActivityCompleted {
            window_days: 3,
            sport: Some("run".into()),
        };
        assert_eq!(m.kind_str(), "activity_completed");
        let r = OutcomeMetric::RampRateWithin { ceiling: 1.3 };
        assert_eq!(r.kind_str(), "ramp_rate_within");
    }

    #[test]
    fn outcome_metric_serde_roundtrip() {
        let m = OutcomeMetric::HrvDelta { window_days: 7 };
        let json = serde_json::to_string(&m).unwrap_or_default();
        assert!(!json.is_empty());
        let back = serde_json::from_str::<OutcomeMetric>(&json).ok();
        assert_eq!(back, Some(m));
    }

    fn sample_playbook() -> Playbook {
        let now = Utc::now();
        Playbook {
            id: "p1".into(),
            tenant_id: "t1".into(),
            user_id: "u1".into(),
            coach_slug: Some("trail".into()),
            trigger: TriggerPattern {
                kind: TriggerKind::MotivationDip,
                sport: Some("run".into()),
                magnitude: Band::Moderate,
            },
            intervention: super::Intervention {
                kind: InterventionKind::MinimumViable,
                magnitude: None,
            },
            outcome_metric: OutcomeMetric::ActivityCompleted {
                window_days: 2,
                sport: Some("run".into()),
            },
            success_count: 0,
            failure_count: 0,
            neutral_count: 0,
            confidence: 0.0,
            last_outcome_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn wilson_penalizes_small_samples() {
        let now = Utc::now();
        let mut lucky = sample_playbook();
        lucky.record_outcome(OutcomeLabel::Success, now);
        // 1/1 success — high raw rate but low confidence.
        assert!((lucky.success_rate() - 1.0).abs() < f32::EPSILON);
        let lucky_conf = lucky.confidence;

        let mut proven = sample_playbook();
        for _ in 0..18 {
            proven.record_outcome(OutcomeLabel::Success, now);
        }
        for _ in 0..2 {
            proven.record_outcome(OutcomeLabel::Failure, now);
        }
        // 18/20 = 0.90 raw, lower raw rate than 1/1 — but MORE confident.
        assert!(proven.success_rate() < lucky.success_rate());
        assert!(
            proven.confidence > lucky_conf,
            "18/20 ({}) should outrank 1/1 ({lucky_conf})",
            proven.confidence
        );
    }

    #[test]
    fn neutrals_do_not_move_confidence() {
        let now = Utc::now();
        let mut pb = sample_playbook();
        pb.record_outcome(OutcomeLabel::Success, now);
        let after_success = pb.confidence;
        pb.record_outcome(OutcomeLabel::Neutral, now);
        pb.record_outcome(OutcomeLabel::Neutral, now);
        // Neutrals bump the counter but leave the decisive-only confidence intact.
        assert_eq!(pb.neutral_count, 2);
        assert!((pb.confidence - after_success).abs() < f32::EPSILON);
    }

    #[test]
    fn empty_playbook_has_zero_confidence() {
        let pb = sample_playbook();
        assert_eq!(pb.total_outcomes(), 0);
        assert!((pb.wilson_lower_bound() - 0.0).abs() < f32::EPSILON);
    }
}
