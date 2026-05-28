// ABOUTME: Synthesizes layers 1-5 of the bullshit detector pipeline into a single ClaimVerdict
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
//! 1. **Rhetoric** — [`rhetoric_detector::classify`]. If rhetorical,
//!    emit `ClaimStatus::Rhetorical` with `VerdictLayer::Rhetoric` and stop.
//! 2. **Deterministic** — hard-coded per-category sanity bounds. If any
//!    bound is violated, emit `ClaimStatus::Contradicted` with
//!    `VerdictLayer::Deterministic`.
//! 3. **Evidence** — retrieval against [`EvidenceCorpus`]. If a match at
//!    or above the configured minimum strength, emit
//!    `ClaimStatus::Supported` with `VerdictLayer::Evidence` and stop.
//! 4. **Consistency** — [`consistency::find_contradiction`] cross-checks
//!    the claim against the other claims in the same reply. If a sibling
//!    directly contradicts it, emit `ClaimStatus::Contradicted` with
//!    `VerdictLayer::Consistency`.
//! 5. **Judge** — [`judge::judge_claim`] LLM fallback, invoked only when
//!    layers 1–4 were inconclusive *and* a provider was injected. Emits the
//!    judge's `ClaimStatus` with `VerdictLayer::Judge`.
//!
//! Three entry points share the same layer logic:
//!
//! - [`check_claim`] runs a single claim through layers 1–4 (the LLM-free
//!   path). Sibling claims for Layer 4 are passed in; Layer 5 needs an LLM
//!   provider so it never runs here — an inconclusive claim falls through to
//!   the evidence verdict.
//! - [`check_claim_judged`] is the async, single-claim counterpart that also
//!   runs Layer 5 when a provider is supplied.
//! - [`check_reply`] runs a whole reply's claims through all five layers,
//!   threading each claim's siblings into Layer 4 and the optional judge
//!   provider into Layer 5.

use crate::claim_extractor::ExtractedClaim;
use crate::consistency::find_contradiction;
use crate::deterministic_bounds;
use crate::evidence_retriever::{EvidenceCorpus, EvidenceMatch};
use crate::judge::judge_claim;
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

/// Result of running the four pure-Rust layers (rhetoric, deterministic,
/// evidence, consistency). Either one of them reached a confident verdict, or
/// they were all inconclusive and the retrieved evidence is carried forward so
/// the optional Layer 5 judge can ground its decision.
enum LayerResult {
    /// A confident verdict from layers 1–4.
    Resolved(VerdictOutcome),
    /// Layers 1–4 were inconclusive; carries the evidence Layer 3 retrieved.
    Inconclusive(Vec<EvidenceMatch>),
}

/// Check a claim against pipeline layers 1–4 in order, stopping at the first
/// confident verdict.
///
/// `siblings` is the other claims in the same coach reply (pass the full slice
/// including `claim` — it is skipped by identity) so Layer 4 can detect a
/// self-contradiction. For a claim checked in true isolation, pass an empty
/// slice; Layer 4 then has nothing to compare against and the verdict falls
/// through to the evidence layer.
///
/// `minimum_strength` controls the Evidence Strength threshold at which an
/// evidence-layer match counts as "supported". Below that, and absent a
/// sibling contradiction, the verdict falls through to `Unsupported`.
///
/// This is the LLM-free path: Layer 5 (judge) needs an LLM provider, so this
/// function never makes a network call. Use [`check_reply`] to additionally
/// run Layer 5 over a whole coach reply.
#[must_use]
pub fn check_claim(
    claim: &ExtractedClaim,
    siblings: &[ExtractedClaim],
    corpus: &EvidenceCorpus,
    minimum_strength: EvidenceStrength,
) -> VerdictOutcome {
    match run_layers_1_to_4(claim, siblings, corpus, minimum_strength) {
        LayerResult::Resolved(outcome) => outcome,
        LayerResult::Inconclusive(matches) => {
            inconclusive_evidence_verdict(&matches, minimum_strength)
        }
    }
}

/// Run all five pipeline layers over every claim in a coach reply.
///
/// Each claim is checked against layers 1–3 individually, then Layer 4
/// (consistency) cross-checks it against its `claims` siblings, and Layer 5
/// (judge) is invoked as the fallback when the first four layers were
/// inconclusive and `judge` is `Some`. Pass `judge: None` to keep the
/// pipeline fully deterministic (no LLM call), in which case inconclusive
/// claims settle on the evidence layer's `Unsupported` verdict.
///
/// # Errors
///
/// Propagates the LLM error from Layer 5 when the judge is invoked and the
/// provider call (or its JSON parse) fails.
pub async fn check_reply(
    claims: &[ExtractedClaim],
    corpus: &EvidenceCorpus,
    minimum_strength: EvidenceStrength,
    judge: Option<&dyn LlmProvider>,
) -> AppResult<Vec<(ExtractedClaim, VerdictOutcome)>> {
    let mut out = Vec::with_capacity(claims.len());
    for claim in claims {
        let outcome = match run_layers_1_to_4(claim, claims, corpus, minimum_strength) {
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
/// The async, judge-enabled counterpart of [`check_claim`]: layers 1–4 run
/// exactly as in the sync path, and when they are inconclusive Layer 5 invokes
/// `judge` (when `Some`). This is the per-claim entry the config-aware
/// verification service uses so each claim can carry its category's own
/// `minimum_strength` while still being cross-checked against the full sibling
/// set.
///
/// # Errors
///
/// Propagates the LLM error from Layer 5 when the judge is invoked and the
/// provider call (or its JSON parse) fails.
pub async fn check_claim_judged(
    claim: &ExtractedClaim,
    siblings: &[ExtractedClaim],
    corpus: &EvidenceCorpus,
    minimum_strength: EvidenceStrength,
    judge: Option<&dyn LlmProvider>,
) -> AppResult<VerdictOutcome> {
    match run_layers_1_to_4(claim, siblings, corpus, minimum_strength) {
        LayerResult::Resolved(outcome) => Ok(outcome),
        LayerResult::Inconclusive(matches) => {
            run_judge_or_settle(claim, &matches, minimum_strength, judge).await
        }
    }
}

/// Run the four pure-Rust layers (rhetoric, deterministic, evidence,
/// consistency) over a single claim. Returns the confident verdict, or
/// [`LayerResult::Inconclusive`] carrying the evidence for the judge.
fn run_layers_1_to_4(
    claim: &ExtractedClaim,
    siblings: &[ExtractedClaim],
    corpus: &EvidenceCorpus,
    minimum_strength: EvidenceStrength,
) -> LayerResult {
    // Layers 1–2.
    if let Some(early) = run_rhetoric_and_deterministic(claim) {
        return LayerResult::Resolved(early);
    }

    // Layer 3 — evidence retrieval. A confident support stops the pipeline.
    let matches = corpus.retrieve(&claim.text, claim.category, 3);
    if let Some(supported) = evidence_support(&matches, minimum_strength) {
        return LayerResult::Resolved(supported);
    }

    // Layer 4 — consistency cross-check against sibling claims.
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

/// Layer 5: invoke the LLM judge when a provider is available, otherwise
/// settle on the evidence layer's inconclusive verdict.
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

/// Run Layers 1 (rhetoric) and 2 (deterministic) over a claim. Returns
/// `Some(outcome)` when either layer fires a confident verdict, `None` when
/// the claim should continue to the evidence layer.
fn run_rhetoric_and_deterministic(claim: &ExtractedClaim) -> Option<VerdictOutcome> {
    // Layer 1 — rhetoric.
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

    // Layer 2 — deterministic bounds.
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

/// Layer 3 success path: returns `Some(Supported)` when the best match meets
/// `minimum_strength`, `None` when the evidence is absent or too weak (the
/// claim then flows to consistency / judge).
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

/// Terminal verdict when layers 1–4 were inconclusive and no judge ran.
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

/// Render retrieved evidence as a plain-text block for the Layer 5 judge.
fn evidence_context(matches: &[EvidenceMatch]) -> String {
    matches
        .iter()
        .map(|m| format!("- {} ({})", m.record.proposition, m.record.citation))
        .collect::<Vec<_>>()
        .join("\n")
}
