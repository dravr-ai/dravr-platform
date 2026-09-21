// ABOUTME: Claim verdicts emitted by the claim-verification (bullshit detector) pipeline
// ABOUTME: Pure types — persistence in pierre-database, pipeline in pierre-evals
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Category a claim belongs to. Each category has its own deterministic
/// bounds, evidence retrieval filters, and Evidence Strength thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimCategory {
    /// Physiology claims — HR, `VO2max`, lactate thresholds, substrate use.
    Physiological,
    /// Training prescription — volume, intensity, periodization.
    TrainingPrescription,
    /// Nutrition — macros, hydration, fuelling timing.
    Nutrition,
    /// Recovery — sleep, HRV, heat/cold, active recovery.
    Recovery,
    /// Supplement — ergogenic aids and dosing.
    Supplement,
    /// Injury rehabilitation — return-to-play timelines and protocols.
    InjuryRehab,
    /// A claim about *this athlete's own* records — what they did, when, how
    /// far, how much they slept.
    ///
    /// The other six are propositions about human physiology, answerable from a
    /// literature corpus. This one is answerable only from the athlete's data,
    /// which is why it needs its own category and its own layer: "your longest
    /// run last month was 21 km" is true or false in the database, and no
    /// amount of sports-science evidence bears on it.
    AthleteData,
}

impl ClaimCategory {
    /// Every variant, in declaration order — the vocabulary a filter offers
    /// and a rejection message lists.
    pub const ALL: &'static [Self] = &[
        Self::Physiological,
        Self::TrainingPrescription,
        Self::Nutrition,
        Self::Recovery,
        Self::Supplement,
        Self::InjuryRehab,
        Self::AthleteData,
    ];

    /// Stable `snake_case` string used in the database `CHECK` constraint.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Physiological => "physiological",
            Self::TrainingPrescription => "training_prescription",
            Self::Nutrition => "nutrition",
            Self::Recovery => "recovery",
            Self::Supplement => "supplement",
            Self::InjuryRehab => "injury_rehab",
            Self::AthleteData => "athlete_data",
        }
    }

    /// Parse a category from its stable stringified form.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "physiological" => Some(Self::Physiological),
            "training_prescription" => Some(Self::TrainingPrescription),
            "nutrition" => Some(Self::Nutrition),
            "recovery" => Some(Self::Recovery),
            "supplement" => Some(Self::Supplement),
            "injury_rehab" => Some(Self::InjuryRehab),
            "athlete_data" => Some(Self::AthleteData),
            _ => None,
        }
    }
}

impl fmt::Display for ClaimCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Verdict status for a claim — what the pipeline concluded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaimStatus {
    /// Claim is backed by evidence at or above the configured strength.
    Supported,
    /// Claim has no evidence meeting the strength threshold.
    Unsupported,
    /// Claim is directly contradicted by evidence or by a deterministic check.
    Contradicted,
    /// Claim is a rhetorical flourish, question, or motivational phrase — not
    /// a factual proposition. Fast path, no retrieval fired.
    Rhetorical,
    /// Pipeline could not reach a confident verdict (e.g. no evidence found,
    /// no matching deterministic rule, judge tied).
    Unverifiable,
}

impl ClaimStatus {
    /// Every variant, in declaration order — the vocabulary a filter offers
    /// and a rejection message lists.
    pub const ALL: &'static [Self] = &[
        Self::Supported,
        Self::Unsupported,
        Self::Contradicted,
        Self::Rhetorical,
        Self::Unverifiable,
    ];

    /// Stable `snake_case` string used in the database `CHECK` constraint.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Unsupported => "unsupported",
            Self::Contradicted => "contradicted",
            Self::Rhetorical => "rhetorical",
            Self::Unverifiable => "unverifiable",
        }
    }

    /// Parse a status from its stable stringified form.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "supported" => Some(Self::Supported),
            "unsupported" => Some(Self::Unsupported),
            "contradicted" => Some(Self::Contradicted),
            "rhetorical" => Some(Self::Rhetorical),
            "unverifiable" => Some(Self::Unverifiable),
            _ => None,
        }
    }
}

/// Strength of the evidence backing a supported claim.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStrength {
    /// No evidence was retrieved — the claim is rhetorical or unverifiable.
    None,
    /// A single observational study or correlational data.
    Weak,
    /// Multiple studies with some disagreement, or a single RCT.
    ///
    /// `moderate` is accepted as a deserialization synonym because the
    /// dravr-contremaitre `evidence/*.md` authors use the natural-English
    /// term in frontmatter; the canonical form serialized back out is still
    /// `mixed`. This mirrors the alias in [`EvidenceStrength::parse`].
    #[default]
    #[serde(alias = "moderate")]
    Mixed,
    /// Peer-reviewed meta-analysis, position stand, or RCT + guideline.
    Strong,
}

impl EvidenceStrength {
    /// Stable `snake_case` string used in the database `CHECK` constraint.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Weak => "weak",
            Self::Mixed => "mixed",
            Self::Strong => "strong",
        }
    }

    /// Parse an evidence strength from its stable stringified form.
    ///
    /// `moderate` is accepted as a synonym for `mixed` because
    /// the dravr-contremaitre `evidence/*.md` content authors use the
    /// natural-English term; the canonical storage form is still
    /// `mixed` so the round-trip through [`Self::as_str`] normalizes.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "none" => Some(Self::None),
            "weak" => Some(Self::Weak),
            "mixed" | "moderate" => Some(Self::Mixed),
            "strong" => Some(Self::Strong),
            _ => None,
        }
    }

    /// True when this evidence level meets the given minimum requirement.
    #[must_use]
    pub fn meets(self, minimum: Self) -> bool {
        self >= minimum
    }
}

/// Which layer of the detector pipeline produced the verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictLayer {
    /// Rhetoric / question / greeting filter — the first stage, no LLM.
    Rhetoric,
    /// Deterministic per-category bounds against population limits (pure Rust).
    Deterministic,
    /// Claim checked against the athlete's own computed physiology
    /// (VDOT-derived training paces, HR/power zones, recent load).
    /// Pure Rust; fires only when a personalized snapshot is supplied.
    Personalized,
    /// Claim about the athlete's own history checked against their own records
    /// — the activity cache and dossier rather than the literature corpus.
    ///
    /// Absent data is a verdict here, not a gap: a specific figure asserted
    /// about an athlete with no connected provider is contradicted, because we
    /// know there is nothing it could have come from.
    AthleteData,
    /// Evidence RAG against the curated sports-science corpus.
    Evidence,
    /// Cross-check against other claims in the same reply.
    Consistency,
    /// LLM-as-judge fallback when the earlier stages were inconclusive.
    Judge,
}

impl VerdictLayer {
    /// Every variant, in declaration order — the vocabulary a filter offers
    /// and a rejection message lists.
    pub const ALL: &'static [Self] = &[
        Self::Rhetoric,
        Self::Deterministic,
        Self::Personalized,
        Self::AthleteData,
        Self::Evidence,
        Self::Consistency,
        Self::Judge,
    ];

    /// Stable `snake_case` string used in the database `CHECK` constraint.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Rhetoric => "rhetoric",
            Self::Deterministic => "deterministic",
            Self::Personalized => "personalized",
            Self::AthleteData => "athlete_data",
            Self::Evidence => "evidence",
            Self::Consistency => "consistency",
            Self::Judge => "judge",
        }
    }

    /// Parse a layer from its stable stringified form.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "rhetoric" => Some(Self::Rhetoric),
            "deterministic" => Some(Self::Deterministic),
            "personalized" => Some(Self::Personalized),
            "athlete_data" => Some(Self::AthleteData),
            "evidence" => Some(Self::Evidence),
            "consistency" => Some(Self::Consistency),
            "judge" => Some(Self::Judge),
            _ => None,
        }
    }
}

/// What support concluded about a flagged verdict after reading it.
///
/// The pipeline's verdict says what the detector thought of the claim; the
/// disposition says what a human thought of the detector. It is the label the
/// false-positive rate is computed from, so it is deliberately three-valued:
/// a triager who cannot tell records `Unsure` rather than guessing either way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictDisposition {
    /// The detector was right: the claim deserved its flag.
    TrueCatch,
    /// The detector was wrong: the claim was fine and the flag was noise.
    FalsePositive,
    /// The triager could not decide from the evidence on the row.
    Unsure,
}

impl VerdictDisposition {
    /// Every variant, in declaration order — the vocabulary a filter offers
    /// and a rejection message lists.
    pub const ALL: &'static [Self] = &[Self::TrueCatch, Self::FalsePositive, Self::Unsure];

    /// Stable `snake_case` string used in the database `CHECK` constraint.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TrueCatch => "true_catch",
            Self::FalsePositive => "false_positive",
            Self::Unsure => "unsure",
        }
    }

    /// Parse a disposition from its stable stringified form.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "true_catch" => Some(Self::TrueCatch),
            "false_positive" => Some(Self::FalsePositive),
            "unsure" => Some(Self::Unsure),
            _ => None,
        }
    }
}

impl fmt::Display for VerdictDisposition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a disposition landed where it did — the knob engineering moves.
///
/// Each reason names one adjustable input of the pipeline, so counting
/// dispositions by reason says which layer's configuration is producing the
/// noise: a missing keyword is a corpus proposition, a bound too tight is the
/// deterministic table, a tolerance too tight is the personalized margin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispositionReason {
    /// The evidence corpus holds the proposition but retrieval never matched
    /// it: the claim's wording shares no 4+ letter token with the record.
    MissingKeyword,
    /// A deterministic population bound rejected a value a competent agent
    /// can legitimately give.
    BoundTooTight,
    /// The personalized or athlete-data layer called a difference a
    /// contradiction when it sat inside ordinary day-to-day variation.
    ToleranceTooTight,
    /// The corpus proposition the verdict leaned on is out of date.
    StaleEvidence,
    /// The extractor filed the claim under a category whose layers cannot
    /// judge it.
    ExtractorMisroute,
    /// The LLM judge reached the wrong conclusion on the evidence it was
    /// shown.
    JudgeError,
    /// Something the list above does not name; the note says what.
    Other,
}

impl DispositionReason {
    /// Every variant, in declaration order — the vocabulary a filter offers
    /// and a rejection message lists.
    pub const ALL: &'static [Self] = &[
        Self::MissingKeyword,
        Self::BoundTooTight,
        Self::ToleranceTooTight,
        Self::StaleEvidence,
        Self::ExtractorMisroute,
        Self::JudgeError,
        Self::Other,
    ];

    /// Stable `snake_case` string used in the database `CHECK` constraint.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MissingKeyword => "missing_keyword",
            Self::BoundTooTight => "bound_too_tight",
            Self::ToleranceTooTight => "tolerance_too_tight",
            Self::StaleEvidence => "stale_evidence",
            Self::ExtractorMisroute => "extractor_misroute",
            Self::JudgeError => "judge_error",
            Self::Other => "other",
        }
    }

    /// Parse a reason from its stable stringified form.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "missing_keyword" => Some(Self::MissingKeyword),
            "bound_too_tight" => Some(Self::BoundTooTight),
            "tolerance_too_tight" => Some(Self::ToleranceTooTight),
            "stale_evidence" => Some(Self::StaleEvidence),
            "extractor_misroute" => Some(Self::ExtractorMisroute),
            "judge_error" => Some(Self::JudgeError),
            "other" => Some(Self::Other),
            _ => None,
        }
    }
}

impl fmt::Display for DispositionReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A single verdict emitted by the claim-verification pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaimVerdict {
    /// Stable identifier.
    pub id: String,
    /// Tenant that owns the verdict.
    pub tenant_id: String,
    /// User the claim was said to.
    pub user_id: String,
    /// Agent persona that authored the claim, if known.
    pub agent_id: Option<String>,
    /// Conversation the claim came from.
    pub conversation_id: Option<String>,
    /// Message the claim was extracted from.
    pub message_id: Option<String>,
    /// The raw claim text as extracted.
    pub claim_text: String,
    /// Category assigned by the extractor.
    pub category: ClaimCategory,
    /// Pipeline verdict.
    pub status: ClaimStatus,
    /// Evidence strength backing the verdict (None for rhetorical/unverifiable).
    pub evidence_strength: EvidenceStrength,
    /// Pipeline confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// Which layer produced this verdict.
    pub layer_fired: VerdictLayer,
    /// User-facing explanation of the verdict, generated by the explanation layer.
    pub explanation: Option<String>,
    /// Comma-separated DOIs / PMIDs that backed a `Supported` verdict.
    pub evidence_refs: Option<String>,
    /// When the verdict was emitted.
    pub created_at: DateTime<Utc>,
    /// Support's judgement of the verdict, once someone has read it.
    pub disposition: Option<VerdictDisposition>,
    /// Which pipeline input the triager blamed, when they named one.
    pub disposition_reason: Option<DispositionReason>,
    /// Free-text note left with the disposition.
    pub disposition_note: Option<String>,
    /// Who disposed it: the admin's service name (their email under cookie
    /// auth, the token's service under bearer auth).
    pub disposed_by: Option<String>,
    /// When the disposition was last written.
    pub disposed_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn category_round_trips_through_str() {
        for c in [
            ClaimCategory::Physiological,
            ClaimCategory::TrainingPrescription,
            ClaimCategory::Nutrition,
            ClaimCategory::Recovery,
            ClaimCategory::Supplement,
            ClaimCategory::InjuryRehab,
        ] {
            assert_eq!(ClaimCategory::parse(c.as_str()), Some(c));
        }
    }

    #[test]
    fn evidence_strength_ordering() {
        assert!(EvidenceStrength::Strong > EvidenceStrength::Mixed);
        assert!(EvidenceStrength::Mixed > EvidenceStrength::Weak);
        assert!(EvidenceStrength::Weak > EvidenceStrength::None);
    }

    #[test]
    fn evidence_strength_deserializes_moderate_as_mixed() {
        // dravr-contremaitre evidence frontmatter uses `moderate`; the serde
        // path must accept it as a synonym for `mixed`, matching `parse`.
        assert_eq!(
            serde_json::from_str::<EvidenceStrength>("\"moderate\"").ok(),
            Some(EvidenceStrength::Mixed)
        );
        assert_eq!(
            EvidenceStrength::parse("moderate"),
            Some(EvidenceStrength::Mixed)
        );
        // Canonical form serializes back to `mixed`, not `moderate`.
        assert_eq!(
            serde_json::to_string(&EvidenceStrength::Mixed)
                .ok()
                .as_deref(),
            Some("\"mixed\"")
        );
    }

    #[test]
    fn meets_threshold_is_ge() {
        assert!(EvidenceStrength::Strong.meets(EvidenceStrength::Mixed));
        assert!(EvidenceStrength::Mixed.meets(EvidenceStrength::Mixed));
        assert!(!EvidenceStrength::Weak.meets(EvidenceStrength::Mixed));
    }

    #[test]
    fn status_round_trip() {
        for s in [
            ClaimStatus::Supported,
            ClaimStatus::Unsupported,
            ClaimStatus::Contradicted,
            ClaimStatus::Rhetorical,
            ClaimStatus::Unverifiable,
        ] {
            assert_eq!(ClaimStatus::parse(s.as_str()), Some(s));
        }
    }

    #[test]
    fn layer_round_trip() {
        for l in [
            VerdictLayer::Rhetoric,
            VerdictLayer::Deterministic,
            VerdictLayer::Personalized,
            VerdictLayer::Evidence,
            VerdictLayer::Consistency,
            VerdictLayer::Judge,
        ] {
            assert_eq!(VerdictLayer::parse(l.as_str()), Some(l));
        }
    }
}
