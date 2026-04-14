// ABOUTME: Synthesizes layers 1-5 of the bullshit detector pipeline into a single ClaimVerdict
// ABOUTME: Runs rhetoric → deterministic → evidence → consistency; LLM judge reserved for Phase D
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Verdict Engine (pipeline synthesis)
//!
//! Given an [`ExtractedClaim`], runs it through the detector layers in
//! order and returns a single [`VerdictOutcome`]. Short-circuits on the
//! first confident verdict:
//!
//! 1. **Rhetoric** — [`rhetoric_detector::classify`]. If rhetorical,
//!    emit `ClaimStatus::Rhetorical` with `VerdictLayer::Rhetoric` and stop.
//! 2. **Deterministic** — hard-coded per-category sanity bounds. If any
//!    bound is violated, emit `ClaimStatus::Contradicted` with
//!    `VerdictLayer::Deterministic`.
//! 3. **Evidence** — retrieval against [`EvidenceCorpus`]. If a match at
//!    or above the configured minimum strength, emit
//!    `ClaimStatus::Supported`; otherwise `ClaimStatus::Unsupported`.
//! 4. **Consistency** and **Judge** are deferred to Phase D — the engine
//!    already knows how to call them, but the Phase A boot path stops
//!    after the evidence layer.

use crate::claim_extractor::ExtractedClaim;
use crate::deterministic_bounds;
use crate::evidence_retriever::EvidenceCorpus;
use crate::rhetoric_detector::{classify as classify_rhetoric, RhetoricVerdict};
use pierre_memory::{ClaimStatus, EvidenceStrength, VerdictLayer};

/// The outcome of running a single claim through the detector pipeline.
#[derive(Debug, Clone)]
pub struct VerdictOutcome {
    /// Final verdict status.
    pub status: ClaimStatus,
    /// Evidence strength backing the verdict (None for rhetorical / unverifiable).
    pub evidence_strength: EvidenceStrength,
    /// Pipeline confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// Which layer fired the final verdict.
    pub layer_fired: VerdictLayer,
    /// User-facing explanation text.
    pub explanation: String,
    /// Comma-separated citation ids for the evidence that backed a support.
    pub evidence_refs: Option<String>,
}

/// Check a claim against pipeline layers 1–3 in order, stopping at the
/// first confident verdict.
///
/// `minimum_strength` controls the Evidence Strength threshold at which an
/// evidence-layer match counts as "supported". Below that, the verdict
/// falls through to `Unsupported`.
#[must_use]
pub fn check_claim(
    claim: &ExtractedClaim,
    corpus: &EvidenceCorpus,
    minimum_strength: EvidenceStrength,
) -> VerdictOutcome {
    // Layer 1 — rhetoric.
    if classify_rhetoric(&claim.text) == RhetoricVerdict::Rhetorical {
        return VerdictOutcome {
            status: ClaimStatus::Rhetorical,
            evidence_strength: EvidenceStrength::None,
            confidence: 1.0,
            layer_fired: VerdictLayer::Rhetoric,
            explanation: "Treated as rhetorical or non-propositional — no verification fired."
                .to_owned(),
            evidence_refs: None,
        };
    }

    // Layer 2 — deterministic bounds.
    if let Some(violation) = deterministic_bounds::check(claim) {
        return VerdictOutcome {
            status: ClaimStatus::Contradicted,
            evidence_strength: EvidenceStrength::Strong,
            confidence: 0.95,
            layer_fired: VerdictLayer::Deterministic,
            explanation: format!("Deterministic bound violation: {}", violation.reason),
            evidence_refs: None,
        };
    }

    // Layer 3 — evidence retrieval.
    let matches = corpus.retrieve(&claim.text, claim.category, 3);
    if matches.is_empty() {
        return VerdictOutcome {
            status: ClaimStatus::Unsupported,
            evidence_strength: EvidenceStrength::None,
            confidence: 0.5,
            layer_fired: VerdictLayer::Evidence,
            explanation: "No evidence retrieved from the sports-science corpus for this claim."
                .to_owned(),
            evidence_refs: None,
        };
    }

    let best = &matches[0];
    if best.record.strength.meets(minimum_strength) {
        let refs = matches
            .iter()
            .map(|m| m.record.id.clone())
            .collect::<Vec<_>>()
            .join(",");
        let citation = best.record.citation.clone();
        VerdictOutcome {
            status: ClaimStatus::Supported,
            evidence_strength: best.record.strength,
            confidence: 0.8,
            layer_fired: VerdictLayer::Evidence,
            explanation: format!("Supported by {citation}"),
            evidence_refs: Some(refs),
        }
    } else {
        VerdictOutcome {
            status: ClaimStatus::Unsupported,
            evidence_strength: best.record.strength,
            confidence: 0.55,
            layer_fired: VerdictLayer::Evidence,
            explanation: format!(
                "Best evidence strength ({}) is below the required minimum ({})",
                best.record.strength.as_str(),
                minimum_strength.as_str()
            ),
            evidence_refs: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::errors::AppResult;
    use pierre_memory::ClaimCategory;

    const CORPUS: &str = r#"
{"id":"doi:10.1/a","category":"nutrition","proposition":"Protein intake of 1.6 to 2.2 g per kg body weight per day maximizes muscle protein synthesis","strength":"strong","citation":"Morton 2018"}
{"id":"doi:10.1/b","category":"supplement","proposition":"Creatine monohydrate at 3 to 5 grams per day increases muscle phosphocreatine stores","strength":"strong","citation":"ISSN 2017"}
"#;

    fn corpus() -> AppResult<EvidenceCorpus> {
        EvidenceCorpus::from_jsonl(CORPUS)
    }

    #[test]
    fn rhetorical_short_circuits() -> AppResult<()> {
        let claim = ExtractedClaim {
            text: "You're crushing it!".into(),
            category: ClaimCategory::TrainingPrescription,
        };
        let outcome = check_claim(&claim, &corpus()?, EvidenceStrength::Mixed);
        assert_eq!(outcome.status, ClaimStatus::Rhetorical);
        assert_eq!(outcome.layer_fired, VerdictLayer::Rhetoric);
        Ok(())
    }

    #[test]
    fn supported_when_strong_evidence_present() -> AppResult<()> {
        let claim = ExtractedClaim {
            text: "Aim for 1.6 grams of protein per kg of body weight per day".into(),
            category: ClaimCategory::Nutrition,
        };
        let outcome = check_claim(&claim, &corpus()?, EvidenceStrength::Mixed);
        assert_eq!(outcome.status, ClaimStatus::Supported);
        assert_eq!(outcome.evidence_strength, EvidenceStrength::Strong);
        assert_eq!(outcome.layer_fired, VerdictLayer::Evidence);
        assert!(outcome.evidence_refs.is_some());
        Ok(())
    }

    #[test]
    fn unsupported_when_corpus_silent() -> AppResult<()> {
        let claim = ExtractedClaim {
            text: "Eating chalk improves electrolyte balance".into(),
            category: ClaimCategory::Nutrition,
        };
        let outcome = check_claim(&claim, &corpus()?, EvidenceStrength::Mixed);
        assert_eq!(outcome.status, ClaimStatus::Unsupported);
        Ok(())
    }

    #[test]
    fn contradicted_by_deterministic_bounds() -> AppResult<()> {
        let claim = ExtractedClaim {
            text: "Your max heart rate is 300 bpm".into(),
            category: ClaimCategory::Physiological,
        };
        let outcome = check_claim(&claim, &corpus()?, EvidenceStrength::Mixed);
        assert_eq!(outcome.status, ClaimStatus::Contradicted);
        assert_eq!(outcome.layer_fired, VerdictLayer::Deterministic);
        Ok(())
    }
}
