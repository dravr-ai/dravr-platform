// ABOUTME: Synthesizes the bullshit detector pipeline layers into a single ClaimVerdict
// ABOUTME: Runs rhetoric → deterministic → evidence → consistency → LLM judge in order
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Verdict Engine (pipeline synthesis)
//!
//! Given an [`ExtractedClaim`], runs it through the detector layers in
//! order and returns a single [`VerdictOutcome`]. Short-circuits on the
//! first confident verdict:
//!
//! - **The rhetoric filter** — [`rhetoric_detector::classify`]. If rhetorical,
//!   emit `ClaimStatus::Rhetorical` with `VerdictLayer::Rhetoric` and stop.
//! - **The deterministic-bounds layer** — hard-coded per-category population
//!   bounds. If any bound is violated, emit `ClaimStatus::Contradicted` with
//!   `VerdictLayer::Deterministic`.
//! - **The personalized-physiology layer** — when an athlete snapshot is
//!   supplied, check the claim against that athlete's own VDOT-derived paces,
//!   zones, and load. A clear mismatch emits `ClaimStatus::Contradicted`, a
//!   match emits `ClaimStatus::Supported`, both with `VerdictLayer::Personalized`.
//! - **The evidence-retrieval layer** — retrieval against [`EvidenceCorpus`].
//!   If a match at or above the configured minimum strength, emit
//!   `ClaimStatus::Supported` with `VerdictLayer::Evidence` and stop.
//! - **The consistency-check layer** — [`consistency::find_contradiction`]
//!   cross-checks the claim against the other claims in the same reply. If a
//!   sibling directly contradicts it, emit `ClaimStatus::Contradicted` with
//!   `VerdictLayer::Consistency`.
//! - **The LLM-judge layer** — [`judge::judge_claim`] LLM fallback, invoked
//!   only when the earlier layers were inconclusive *and* a provider was
//!   injected. Emits the judge's `ClaimStatus` with `VerdictLayer::Judge`.
//!
//! Three entry points share the same layer logic:
//!
//! - [`check_claim`] runs a single claim through every pure-Rust layer (the
//!   LLM-free path). Sibling claims for the consistency-check layer are passed
//!   in; the LLM-judge layer needs an LLM provider so it never runs here — an
//!   inconclusive claim falls through to the evidence verdict.
//! - [`check_claim_judged`] is the async, single-claim counterpart that also
//!   runs the LLM-judge layer when a provider is supplied.
//! - [`check_reply`] runs a whole reply's claims through every layer,
//!   threading each claim's siblings into the consistency-check layer and the
//!   optional judge provider into the LLM-judge layer.

use crate::athlete_data::{check as athlete_data_check, AthleteRecord};
use crate::claim_extractor::ExtractedClaim;
use crate::consistency::find_contradiction;
use crate::deterministic_bounds;
use crate::evidence_retriever::{EvidenceCorpus, EvidenceMatch};
use crate::judge::judge_claim;
use crate::personalized::{check as personalized_check, PersonalizedContext};
use crate::rhetoric_detector::{classify as classify_rhetoric, RhetoricVerdict};
use pierre_core::errors::AppResult;
use pierre_llm::LlmProvider;
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

/// Result of running the pure-Rust layers (rhetoric, deterministic,
/// evidence, consistency). Either one of them reached a confident verdict, or
/// they were all inconclusive and the retrieved evidence is carried forward so
/// the optional LLM-judge layer can ground its decision.
enum LayerResult {
    /// A confident verdict from the pure-Rust layers.
    Resolved(VerdictOutcome),
    /// The pure-Rust layers were inconclusive; carries the evidence the
    /// evidence-retrieval layer retrieved.
    Inconclusive(Vec<EvidenceMatch>),
}

/// Check a claim against the pure-Rust pipeline layers in order, stopping at
/// the first confident verdict.
///
/// `siblings` is the other claims in the same coach reply (pass the full slice
/// including `claim` — it is skipped by identity) so the consistency-check
/// layer can detect a self-contradiction. For a claim checked in true
/// isolation, pass an empty slice; the consistency-check layer then has nothing
/// to compare against and the verdict falls through to the evidence layer.
///
/// `minimum_strength` controls the Evidence Strength threshold at which an
/// evidence-layer match counts as "supported". Below that, and absent a
/// sibling contradiction, the verdict falls through to `Unsupported`.
///
/// This is the LLM-free path: the LLM-judge layer needs an LLM provider, so
/// this function never makes a network call. Use [`check_reply`] to
/// additionally run the LLM-judge layer over a whole coach reply.
#[must_use]
pub fn check_claim(
    claim: &ExtractedClaim,
    siblings: &[ExtractedClaim],
    corpus: &EvidenceCorpus,
    minimum_strength: EvidenceStrength,
    personalized: Option<&PersonalizedContext<'_>>,
    athlete_record: Option<&AthleteRecord>,
) -> VerdictOutcome {
    match run_layers_1_to_4(
        claim,
        siblings,
        corpus,
        minimum_strength,
        personalized,
        athlete_record,
    ) {
        LayerResult::Resolved(outcome) => outcome,
        LayerResult::Inconclusive(matches) => {
            inconclusive_evidence_verdict(&matches, minimum_strength)
        }
    }
}

/// Run all five pipeline layers over every claim in a coach reply.
///
/// Each claim is checked against the rhetoric, deterministic-bounds, and
/// evidence-retrieval layers individually, then the consistency-check layer
/// cross-checks it against its `claims` siblings, and the LLM-judge layer is
/// invoked as the fallback when the earlier layers were inconclusive and
/// `judge` is `Some`. Pass `judge: None` to keep the pipeline fully
/// deterministic (no LLM call), in which case inconclusive claims settle on the
/// evidence layer's `Unsupported` verdict.
///
/// # Errors
///
/// Propagates the LLM error from the LLM-judge layer when the judge is invoked
/// and the provider call (or its JSON parse) fails.
pub async fn check_reply(
    claims: &[ExtractedClaim],
    corpus: &EvidenceCorpus,
    minimum_strength: EvidenceStrength,
    judge: Option<&dyn LlmProvider>,
    personalized: Option<&PersonalizedContext<'_>>,
    athlete_record: Option<&AthleteRecord>,
) -> AppResult<Vec<(ExtractedClaim, VerdictOutcome)>> {
    let mut out = Vec::with_capacity(claims.len());
    for claim in claims {
        let outcome = match run_layers_1_to_4(
            claim,
            claims,
            corpus,
            minimum_strength,
            personalized,
            athlete_record,
        ) {
            LayerResult::Resolved(outcome) => outcome,
            LayerResult::Inconclusive(matches) => {
                run_judge_or_settle(claim, &matches, minimum_strength, judge).await?
            }
        };
        out.push((claim.clone(), outcome));
    }
    Ok(out)
}

/// Run all five pipeline layers over a single claim with explicit siblings.
///
/// The async, judge-enabled counterpart of [`check_claim`]: the pure-Rust
/// layers run exactly as in the sync path, and when they are inconclusive the
/// LLM-judge layer invokes `judge` (when `Some`). This is the per-claim entry
/// the config-aware verification service uses so each claim can carry its
/// category's own `minimum_strength` while still being cross-checked against the
/// full sibling set.
///
/// # Errors
///
/// Propagates the LLM error from the LLM-judge layer when the judge is invoked
/// and the provider call (or its JSON parse) fails.
pub async fn check_claim_judged(
    claim: &ExtractedClaim,
    siblings: &[ExtractedClaim],
    corpus: &EvidenceCorpus,
    minimum_strength: EvidenceStrength,
    judge: Option<&dyn LlmProvider>,
    personalized: Option<&PersonalizedContext<'_>>,
    athlete_record: Option<&AthleteRecord>,
) -> AppResult<VerdictOutcome> {
    match run_layers_1_to_4(
        claim,
        siblings,
        corpus,
        minimum_strength,
        personalized,
        athlete_record,
    ) {
        LayerResult::Resolved(outcome) => Ok(outcome),
        LayerResult::Inconclusive(matches) => {
            run_judge_or_settle(claim, &matches, minimum_strength, judge).await
        }
    }
}

/// Run the pure-Rust layers (rhetoric, deterministic, evidence,
/// consistency) over a single claim. Returns the confident verdict, or
/// [`LayerResult::Inconclusive`] carrying the evidence for the judge.
fn run_layers_1_to_4(
    claim: &ExtractedClaim,
    siblings: &[ExtractedClaim],
    corpus: &EvidenceCorpus,
    minimum_strength: EvidenceStrength,
    personalized: Option<&PersonalizedContext<'_>>,
    athlete_record: Option<&AthleteRecord>,
) -> LayerResult {
    // The rhetoric filter and deterministic-bounds layer.
    if let Some(early) = run_rhetoric_and_deterministic(claim) {
        return LayerResult::Resolved(early);
    }

    // The personalized-physiology layer. Fires only when the caller supplied
    // an athlete snapshot; otherwise the claim flows straight to evidence.
    if let Some(ctx) = personalized {
        if let Some(outcome) = personalized_check(claim, ctx) {
            return LayerResult::Resolved(outcome);
        }
    }

    // The athlete-data layer. Placed ahead of evidence retrieval because a
    // claim about this athlete's own records is a database question: the
    // literature corpus has nothing to say about whether one person ran 21 km,
    // and letting it answer would dress a corpus miss up as a verdict.
    //
    // Without a record supplied, an athlete-data claim falls through
    // unadjudicated — which is the pre-Phase-5 behaviour, not a new gap.
    if let Some(record) = athlete_record {
        if let Some(outcome) = athlete_data_check(claim, record) {
            return LayerResult::Resolved(outcome);
        }
    }

    // The evidence-retrieval layer. A confident support stops the pipeline.
    let matches = corpus.retrieve(&claim.text, claim.category, 3);
    if let Some(supported) = evidence_support(&matches, minimum_strength) {
        return LayerResult::Resolved(supported);
    }

    // The consistency-check layer cross-checks against sibling claims.
    if let Some(conflict) = find_contradiction(claim, siblings) {
        return LayerResult::Resolved(VerdictOutcome {
            status: ClaimStatus::Contradicted,
            evidence_strength: EvidenceStrength::None,
            confidence: 0.7,
            layer_fired: VerdictLayer::Consistency,
            explanation: format!("Self-contradiction: {}", conflict.reason),
            evidence_refs: None,
        });
    }

    LayerResult::Inconclusive(matches)
}

/// The LLM-judge layer: invoke the LLM judge when a provider is available,
/// otherwise settle on the evidence layer's inconclusive verdict.
async fn run_judge_or_settle(
    claim: &ExtractedClaim,
    matches: &[EvidenceMatch],
    minimum_strength: EvidenceStrength,
    judge: Option<&dyn LlmProvider>,
) -> AppResult<VerdictOutcome> {
    let Some(provider) = judge else {
        return Ok(inconclusive_evidence_verdict(matches, minimum_strength));
    };
    let evidence_context = evidence_context(matches);
    let judgement = judge_claim(provider, &claim.text, &evidence_context).await?;
    Ok(VerdictOutcome {
        status: judgement.status,
        evidence_strength: EvidenceStrength::None,
        confidence: judgement.confidence,
        layer_fired: VerdictLayer::Judge,
        explanation: format!("LLM judge: {}", judgement.rationale),
        evidence_refs: None,
    })
}

/// Run the rhetoric filter and the deterministic-bounds layer over a claim.
/// Returns `Some(outcome)` when either layer fires a confident verdict, `None`
/// when the claim should continue to the evidence layer.
fn run_rhetoric_and_deterministic(claim: &ExtractedClaim) -> Option<VerdictOutcome> {
    // The rhetoric filter.
    if classify_rhetoric(&claim.text) == RhetoricVerdict::Rhetorical {
        return Some(VerdictOutcome {
            status: ClaimStatus::Rhetorical,
            evidence_strength: EvidenceStrength::None,
            confidence: 1.0,
            layer_fired: VerdictLayer::Rhetoric,
            explanation: "Treated as rhetorical or non-propositional — no verification fired."
                .to_owned(),
            evidence_refs: None,
        });
    }

    // The deterministic-bounds layer.
    if let Some(violation) = deterministic_bounds::check(claim) {
        return Some(VerdictOutcome {
            status: ClaimStatus::Contradicted,
            evidence_strength: EvidenceStrength::Strong,
            confidence: 0.95,
            layer_fired: VerdictLayer::Deterministic,
            explanation: format!("Deterministic bound violation: {}", violation.reason),
            evidence_refs: None,
        });
    }

    None
}

/// Evidence-retrieval success path: returns `Some(Supported)` when the best
/// match meets `minimum_strength`, `None` when the evidence is absent or too
/// weak (the claim then flows to consistency / judge).
fn evidence_support(
    matches: &[EvidenceMatch],
    minimum_strength: EvidenceStrength,
) -> Option<VerdictOutcome> {
    let best = matches.first()?;
    if !best.record.strength.meets(minimum_strength) {
        return None;
    }
    let refs = matches
        .iter()
        .map(|m| m.record.id.clone())
        .collect::<Vec<_>>()
        .join(",");
    let citation = best.record.citation.clone();
    Some(VerdictOutcome {
        status: ClaimStatus::Supported,
        evidence_strength: best.record.strength,
        confidence: 0.8,
        layer_fired: VerdictLayer::Evidence,
        explanation: format!("Supported by {citation}"),
        evidence_refs: Some(refs),
    })
}

/// Terminal verdict when the pure-Rust layers were inconclusive and no judge ran.
/// Either no evidence was retrieved or the best match was below threshold.
fn inconclusive_evidence_verdict(
    matches: &[EvidenceMatch],
    minimum_strength: EvidenceStrength,
) -> VerdictOutcome {
    let Some(best) = matches.first() else {
        return VerdictOutcome {
            status: ClaimStatus::Unsupported,
            evidence_strength: EvidenceStrength::None,
            confidence: 0.5,
            layer_fired: VerdictLayer::Evidence,
            explanation: "No evidence retrieved from the sports-science corpus for this claim."
                .to_owned(),
            evidence_refs: None,
        };
    };
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

/// Render retrieved evidence as a plain-text block for the LLM-judge layer.
fn evidence_context(matches: &[EvidenceMatch]) -> String {
    matches
        .iter()
        .map(|m| format!("- {} ({})", m.record.proposition, m.record.citation))
        .collect::<Vec<_>>()
        .join("\n")
}
