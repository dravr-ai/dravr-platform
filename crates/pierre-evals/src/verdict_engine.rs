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
//! Four entry points share the same layer logic:
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
//! - [`verify_reply`] is the chat stage's entry: it extracts the claims from
//!   the reply text and honours the agent's [`VerificationConfig`] before
//!   running each claim through [`check_claim_judged`]. The seeded-fabrication
//!   eval calls it too, so the eval measures the loop production runs.

use crate::athlete_data::{check as athlete_data_check, AthleteRecord};
use crate::claim_extractor::{extract_heuristic, ExtractedClaim};
use crate::consistency::find_contradiction;
use crate::deterministic_bounds;
use crate::evidence_retriever::{EvidenceCorpus, EvidenceMatch};
use crate::judge::{judge_claim, ClaimJudge};
use crate::personalized::{check as personalized_check, PersonalizedContext};
use crate::rhetoric_detector::{classify as classify_rhetoric, RhetoricVerdict};
use crate::verification_config::VerificationConfig;
use pierre_core::errors::AppResult;
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

/// Explanation recorded on the verdict of a claim whose LLM-judge call failed.
///
/// Persisted verbatim in the verdict row, so the triage view and the coach
/// view read "this claim was never checked" from the row itself.
const JUDGE_ERROR_EXPLANATION: &str = "Not checked: judge error";

impl VerdictOutcome {
    /// The verdict for a claim the LLM judge failed to check: the provider
    /// refused, timed out, or answered something that was not the JSON the judge
    /// asked for.
    ///
    /// The claim is [`ClaimStatus::Unverifiable`], the vocabulary's "no
    /// confident verdict", with zero confidence. It never reads as
    /// `Supported`, and it raises no flag of its own, because an unchecked
    /// claim is not a contradicted one. What it does is stay visible as
    /// unchecked: [`Self::is_judge_error`] tells it apart from a judge that
    /// answered "unverifiable".
    #[must_use]
    pub fn judge_error() -> Self {
        Self {
            status: ClaimStatus::Unverifiable,
            evidence_strength: EvidenceStrength::None,
            confidence: 0.0,
            layer_fired: VerdictLayer::Judge,
            explanation: JUDGE_ERROR_EXPLANATION.to_owned(),
            evidence_refs: None,
        }
    }

    /// `true` when this verdict stands for a claim the LLM judge failed to
    /// check, as built by [`Self::judge_error`].
    #[must_use]
    pub fn is_judge_error(&self) -> bool {
        self.layer_fired == VerdictLayer::Judge
            && self.status == ClaimStatus::Unverifiable
            && self.explanation == JUDGE_ERROR_EXPLANATION
    }
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
/// `siblings` is the other claims in the same agent reply (pass the full slice
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
/// additionally run the LLM-judge layer over a whole agent reply.
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

/// Run all five pipeline layers over every claim in an agent reply.
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
    judge: Option<ClaimJudge<'_>>,
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
    judge: Option<ClaimJudge<'_>>,
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

/// Verify a whole agent reply honouring the agent's [`VerificationConfig`],
/// running all five layers including the LLM-judge fallback.
///
/// Returns an empty vec when the config has `enabled = false`. Otherwise the
/// claims are extracted heuristically; a claim whose category the agent opted
/// out of is dropped (not even persisted as a verdict), and every other claim
/// runs through [`check_claim_judged`] at its category's `min_strength`. Each
/// is cross-checked against every extracted claim, not just the enabled ones —
/// a self-contradiction is worth flagging even when the sibling falls in a
/// category the agent opted out of. The personalized layer fires when
/// `personalized` is supplied, the athlete-data layer when `athlete_record`
/// is. Pass `judge: None` to keep the run fully deterministic.
///
/// The judge is the only step that can fail, and it fails one claim at a time:
/// a claim whose judge call errors (the provider refused, timed out, or
/// answered something unparseable) is recorded with
/// [`VerdictOutcome::judge_error`] and the run goes on. Every other claim keeps
/// its verdict, the ones the deterministic layers settled included, so a judge
/// outage costs the claims that needed the judge and nothing else. The caller
/// reads [`VerdictOutcome::is_judge_error`] to see which claims were not checked.
pub async fn verify_reply(
    agent_reply: &str,
    config: &VerificationConfig,
    corpus: &EvidenceCorpus,
    judge: Option<ClaimJudge<'_>>,
    personalized: Option<&PersonalizedContext<'_>>,
    athlete_record: Option<&AthleteRecord>,
) -> Vec<(ExtractedClaim, VerdictOutcome)> {
    if !config.enabled {
        return Vec::new();
    }
    let claims = extract_heuristic(agent_reply);
    let mut out = Vec::with_capacity(claims.len());
    for claim in claims.iter().filter(|c| config.is_enabled_for(c.category)) {
        let outcome = match check_claim_judged(
            claim,
            &claims,
            corpus,
            config.for_category(claim.category).min_strength,
            judge,
            personalized,
            athlete_record,
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(e) => {
                // The claim text is a sentence of the reply, so only its
                // category is logged.
                tracing::warn!(
                    error = %e,
                    category = claim.category.as_str(),
                    "claim judge failed — recording the claim as not checked"
                );
                VerdictOutcome::judge_error()
            }
        };
        out.push((claim.clone(), outcome));
    }
    out
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
    judge: Option<ClaimJudge<'_>>,
) -> AppResult<VerdictOutcome> {
    let Some(judge) = judge else {
        return Ok(inconclusive_evidence_verdict(matches, minimum_strength));
    };
    let evidence_context = evidence_context(matches);
    let judgement = judge_claim(judge, &claim.text, &evidence_context).await?;
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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use pierre_core::errors::AppError;
    use pierre_core::llm::{
        ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, MessageRole,
    };
    use pierre_memory::{ClaimStatus, EvidenceStrength, VerdictLayer};

    use super::{verify_reply, VerdictOutcome};
    use crate::{ClaimJudge, EvidenceCorpus, VerificationConfig};

    #[test]
    fn a_judge_error_is_an_unchecked_claim_that_never_reads_as_support() {
        let verdict = VerdictOutcome::judge_error();

        assert_eq!(verdict.status, ClaimStatus::Unverifiable);
        assert_eq!(verdict.layer_fired, VerdictLayer::Judge);
        assert_eq!(verdict.evidence_strength, EvidenceStrength::None);
        assert!(verdict.confidence.abs() < f32::EPSILON);
        // Pinned on purpose: a persisted value, written verbatim to the verdict
        // row that the triage and coach views read back.
        assert_eq!(verdict.explanation, "Not checked: judge error");
        assert_eq!(verdict.evidence_refs, None);
        assert!(verdict.is_judge_error());
    }

    #[test]
    fn a_judge_that_answered_unverifiable_is_not_a_judge_error() {
        // Same status and layer as a judge error, but the judge ran and said it
        // could not tell: that claim was checked.
        let answered = VerdictOutcome {
            status: ClaimStatus::Unverifiable,
            evidence_strength: EvidenceStrength::None,
            confidence: 0.4,
            layer_fired: VerdictLayer::Judge,
            explanation: "LLM judge: the evidence is mixed".to_owned(),
            evidence_refs: None,
        };

        assert!(!answered.is_judge_error());
    }

    #[test]
    fn only_a_judge_layer_unverifiable_verdict_can_be_a_judge_error() {
        let by_other_layer = VerdictOutcome {
            layer_fired: VerdictLayer::Evidence,
            ..VerdictOutcome::judge_error()
        };
        let by_other_status = VerdictOutcome {
            status: ClaimStatus::Unsupported,
            ..VerdictOutcome::judge_error()
        };

        assert!(!by_other_layer.is_judge_error());
        assert!(!by_other_status.is_judge_error());
    }

    /// A deterministic bound violation: the first claim never reaches the judge.
    const OUT_OF_BOUNDS: &str = "Your maximum heart rate is 300 bpm.";
    /// Reaches the judge, because an empty corpus can never settle it.
    const JUDGED_AND_FAILED: &str =
        "Drinking beetroot juice daily raises your lactate threshold by 12% within two weeks.";
    /// Reaches the judge too, in another category so it never conflicts with the claim above.
    const JUDGED_AND_ANSWERED: &str =
        "Tart cherry juice before bed improves your sleep quality by 8% within a month.";

    /// A judge provider that fails on one claim and supports every other.
    struct FailsOnClaim {
        failing_claim: &'static str,
        calls: AtomicUsize,
    }

    impl FailsOnClaim {
        fn new(failing_claim: &'static str) -> Self {
            Self {
                failing_claim,
                calls: AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl LlmProvider for FailsOnClaim {
        fn name(&self) -> &'static str {
            "fails_on_claim"
        }
        fn display_name(&self) -> &'static str {
            "Judge that fails on one claim"
        }
        fn capabilities(&self) -> LlmCapabilities {
            LlmCapabilities::SYSTEM_MESSAGES
        }
        fn default_model(&self) -> &'static str {
            "mock-model"
        }
        fn available_models(&self) -> &[String] {
            &[]
        }
        async fn complete(&self, request: &ChatRequest) -> Result<ChatResponse, AppError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let asked_about_failing_claim = request
                .messages
                .iter()
                .any(|m| m.role == MessageRole::User && m.content.contains(self.failing_claim));
            if asked_about_failing_claim {
                return Err(AppError::internal("judge provider timed out"));
            }
            Ok(ChatResponse {
                content: r#"{"verdict":"supported","confidence":0.9,"rationale":"Replicated in trials."}"#
                    .to_owned(),
                model: "mock-model".to_owned(),
                usage: None,
                finish_reason: Some("stop".to_owned()),
                warnings: None,
                tool_calls: None,
            })
        }
        async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
            Err(AppError::internal("streaming is not used here"))
        }
        async fn health_check(&self) -> Result<bool, AppError> {
            Ok(true)
        }
    }

    fn judged(provider: &FailsOnClaim) -> ClaimJudge<'_> {
        ClaimJudge {
            provider,
            system_prompt: "Judge the claim.",
            model: None,
            recorder: None,
        }
    }

    #[tokio::test]
    async fn a_judge_error_on_one_claim_keeps_every_other_verdict() {
        let reply = format!("{OUT_OF_BOUNDS} {JUDGED_AND_FAILED} {JUDGED_AND_ANSWERED}");
        let provider = FailsOnClaim::new("beetroot");

        let verdicts = verify_reply(
            &reply,
            &VerificationConfig::default(),
            &EvidenceCorpus::default(),
            Some(judged(&provider)),
            None,
            None,
        )
        .await;

        let texts: Vec<&str> = verdicts
            .iter()
            .map(|(claim, _)| claim.text.as_str())
            .collect();
        assert_eq!(
            texts,
            [OUT_OF_BOUNDS, JUDGED_AND_FAILED, JUDGED_AND_ANSWERED]
        );

        let (_, bound) = &verdicts[0];
        assert_eq!(bound.status, ClaimStatus::Contradicted);
        assert_eq!(bound.layer_fired, VerdictLayer::Deterministic);

        let (_, unchecked) = &verdicts[1];
        assert!(unchecked.is_judge_error());
        assert_eq!(unchecked.status, ClaimStatus::Unverifiable);
        assert_eq!(unchecked.layer_fired, VerdictLayer::Judge);

        let (_, answered) = &verdicts[2];
        assert!(!answered.is_judge_error());
        assert_eq!(answered.status, ClaimStatus::Supported);
        assert_eq!(answered.layer_fired, VerdictLayer::Judge);
        assert!(answered.explanation.contains("Replicated in trials."));

        assert_eq!(
            provider.calls.load(Ordering::SeqCst),
            2,
            "the judge is asked about both inconclusive claims and no other"
        );
    }

    #[tokio::test]
    async fn a_judge_that_fails_on_every_claim_still_returns_a_verdict_for_each() {
        let reply = format!("{OUT_OF_BOUNDS} {JUDGED_AND_FAILED}");
        // The empty needle is in every prompt, so every judge call fails.
        let provider = FailsOnClaim::new("");

        let verdicts = verify_reply(
            &reply,
            &VerificationConfig::default(),
            &EvidenceCorpus::default(),
            Some(judged(&provider)),
            None,
            None,
        )
        .await;

        assert_eq!(verdicts.len(), 2, "no claim is dropped by a judge outage");
        assert_eq!(verdicts[0].1.layer_fired, VerdictLayer::Deterministic);
        assert_eq!(verdicts[0].1.status, ClaimStatus::Contradicted);
        assert!(verdicts[1].1.is_judge_error());
    }

    #[tokio::test]
    async fn without_a_judge_no_claim_is_marked_as_a_judge_error() {
        let reply = format!("{OUT_OF_BOUNDS} {JUDGED_AND_FAILED}");

        let verdicts = verify_reply(
            &reply,
            &VerificationConfig::default(),
            &EvidenceCorpus::default(),
            None,
            None,
            None,
        )
        .await;

        assert_eq!(verdicts.len(), 2);
        assert!(verdicts
            .iter()
            .all(|(_, outcome)| !outcome.is_judge_error()));
        assert_eq!(verdicts[1].1.status, ClaimStatus::Unsupported);
    }
}
