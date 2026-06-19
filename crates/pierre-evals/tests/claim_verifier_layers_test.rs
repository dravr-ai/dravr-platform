// ABOUTME: Integration tests for bullshit detector layers 4 (consistency) and 5 (LLM judge)
// ABOUTME: Proves consistency catches a contradictory sibling pair and the judge resolves inconclusive claims
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::slice;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use pierre_core::errors::AppError;
use pierre_evals::{
    check_claim, check_claim_judged, check_reply, claim_extractor::ExtractedClaim,
    evidence_retriever::EvidenceCorpus, find_contradiction,
};
use pierre_llm::{ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider};
use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};

const TEST_CORPUS: &str = r#"
{"id":"doi:10.1/a","category":"nutrition","proposition":"Protein intake of 1.6 to 2.2 g per kg body weight per day maximizes muscle protein synthesis","strength":"strong","citation":"Morton 2018"}
{"id":"doi:10.1/c","category":"physiological","proposition":"Maximum heart rate is approximated by 208 minus 0.7 times age","strength":"mixed","citation":"Tanaka 2001"}
"#;

fn corpus() -> EvidenceCorpus {
    EvidenceCorpus::from_jsonl(TEST_CORPUS).expect("parse test corpus")
}

fn claim(text: &str, category: ClaimCategory) -> ExtractedClaim {
    ExtractedClaim {
        text: text.to_owned(),
        category,
    }
}

/// Test double for an LLM provider. Returns a canned JSON body for every
/// `complete` call and counts how many times it was invoked, so a test can
/// assert the judge fired (or did NOT fire) without a network call.
///
/// Mock is confined to test code per the project mock policy: it stands in for
/// a real `LlmProvider` to exercise the Layer 5 fallback deterministically.
struct CannedJudge {
    body: String,
    calls: AtomicUsize,
}

impl CannedJudge {
    fn new(body: &str) -> Self {
        Self {
            body: body.to_owned(),
            calls: AtomicUsize::new(0),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmProvider for CannedJudge {
    fn name(&self) -> &'static str {
        "canned-judge"
    }

    fn display_name(&self) -> &'static str {
        "Canned Judge"
    }

    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }

    fn default_model(&self) -> &'static str {
        "canned"
    }

    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(ChatResponse {
            content: self.body.clone(),
            model: "canned".to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        // The Layer 5 judge only calls `complete`; streaming is unused.
        Err(AppError::internal("canned judge does not stream"))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

// ---------------------------------------------------------------------------
// Layer 4 — consistency
// ---------------------------------------------------------------------------

#[test]
fn consistency_flags_numeric_divergence_between_siblings() {
    let a = claim(
        "Your easy run pace should be around 5 minutes per kilometre.",
        ClaimCategory::TrainingPrescription,
    );
    let b = claim(
        "Your easy run pace should be around 7 minutes per kilometre.",
        ClaimCategory::TrainingPrescription,
    );
    let conflict = find_contradiction(&a, slice::from_ref(&b));
    assert!(
        conflict.is_some(),
        "diverging easy-pace figures should be flagged as a contradiction"
    );
    let conflict = conflict.unwrap();
    assert!(
        conflict.reason.contains("pace") || conflict.reason.contains("easy"),
        "reason should name the shared subject, got: {}",
        conflict.reason
    );
}

#[test]
fn consistency_flags_negation_conflict_between_siblings() {
    let a = claim(
        "You should take creatine every single training day.",
        ClaimCategory::Supplement,
    );
    let b = claim(
        "You should avoid creatine entirely during this block.",
        ClaimCategory::Supplement,
    );
    let conflict = find_contradiction(&a, slice::from_ref(&b));
    assert!(
        conflict.is_some(),
        "assert-vs-negate on creatine should be flagged"
    );
}

#[test]
fn consistency_ignores_agreeing_siblings() {
    let a = claim(
        "Aim for 1.6 grams of protein per kg of body weight.",
        ClaimCategory::Nutrition,
    );
    let b = claim(
        "Around 1.6 g per kg of protein keeps you in the right range.",
        ClaimCategory::Nutrition,
    );
    assert!(
        find_contradiction(&a, slice::from_ref(&b)).is_none(),
        "agreeing protein figures must not be flagged"
    );
}

#[test]
fn consistency_ignores_cross_category_numbers() {
    let a = claim(
        "Aim for 1.6 grams of protein per kg of body weight.",
        ClaimCategory::Nutrition,
    );
    // Same number, different category and subject — must not collide.
    let b = claim(
        "Your lactate threshold sits near 1.6 of something unrelated.",
        ClaimCategory::Physiological,
    );
    assert!(find_contradiction(&a, slice::from_ref(&b)).is_none());
}

#[tokio::test]
async fn consistency_layer_fires_in_full_pipeline() {
    // Two training-prescription claims with diverging easy-pace numbers.
    // Neither trips a deterministic bound, neither matches the evidence corpus,
    // so Layer 4 must be what fires the verdict.
    let claims = vec![
        claim(
            "Your easy run pace should be around 5 minutes per kilometre.",
            ClaimCategory::TrainingPrescription,
        ),
        claim(
            "Your easy run pace should be around 7 minutes per kilometre.",
            ClaimCategory::TrainingPrescription,
        ),
    ];
    let verdicts = check_reply(&claims, &corpus(), EvidenceStrength::Mixed, None, None)
        .await
        .expect("pipeline runs without a judge");
    assert_eq!(verdicts.len(), 2);
    assert!(
        verdicts
            .iter()
            .any(|(_, o)| o.layer_fired == VerdictLayer::Consistency
                && o.status == ClaimStatus::Contradicted),
        "expected a Consistency-layer contradiction, got {:?}",
        verdicts
            .iter()
            .map(|(_, o)| o.layer_fired)
            .collect::<Vec<_>>()
    );
}

#[test]
fn check_claim_without_siblings_skips_consistency() {
    let c = claim(
        "Your easy run pace should be around 5 minutes per kilometre.",
        ClaimCategory::TrainingPrescription,
    );
    // No siblings — Layer 4 has nothing to compare against, so the verdict
    // falls through to the evidence layer (no corpus match → Unsupported).
    let outcome = check_claim(&c, &[], &corpus(), EvidenceStrength::Mixed, None);
    assert_eq!(outcome.layer_fired, VerdictLayer::Evidence);
    assert_eq!(outcome.status, ClaimStatus::Unsupported);
}

// ---------------------------------------------------------------------------
// Layer 5 — LLM judge
// ---------------------------------------------------------------------------

#[tokio::test]
async fn judge_resolves_inconclusive_claim() {
    // A claim that no deterministic layer can resolve and that the corpus is
    // silent on: layers 1–4 are inconclusive, so the judge must fire.
    let c = claim(
        "Beetroot juice before a 10k improves time-trial performance.",
        ClaimCategory::Supplement,
    );
    let judge = CannedJudge::new(
        r#"{"verdict":"supported","confidence":0.82,"rationale":"Nitrate supplementation has RCT support for endurance."}"#,
    );
    let outcome = check_claim_judged(
        &c,
        slice::from_ref(&c),
        &corpus(),
        EvidenceStrength::Mixed,
        Some(&judge),
        None,
    )
    .await
    .expect("judge call succeeds");

    assert_eq!(judge.call_count(), 1, "judge should have been invoked once");
    assert_eq!(outcome.layer_fired, VerdictLayer::Judge);
    assert_eq!(outcome.status, ClaimStatus::Supported);
    assert!((outcome.confidence - 0.82).abs() < 1e-6);
    assert!(outcome.explanation.contains("LLM judge"));
}

#[tokio::test]
async fn judge_maps_unknown_verdict_to_unverifiable() {
    let c = claim(
        "Eating moon dust improves grip strength.",
        ClaimCategory::Supplement,
    );
    let judge = CannedJudge::new(
        r#"{"verdict":"who knows","confidence":0.3,"rationale":"No data exists."}"#,
    );
    let outcome = check_claim_judged(
        &c,
        slice::from_ref(&c),
        &corpus(),
        EvidenceStrength::Mixed,
        Some(&judge),
        None,
    )
    .await
    .expect("judge call succeeds");
    assert_eq!(outcome.layer_fired, VerdictLayer::Judge);
    assert_eq!(outcome.status, ClaimStatus::Unverifiable);
}

#[tokio::test]
async fn judge_does_not_fire_when_earlier_layer_resolves() {
    // Deterministic bound violation resolves at Layer 2; the judge must never
    // be invoked.
    let c = claim(
        "Your max heart rate is 400 bpm.",
        ClaimCategory::Physiological,
    );
    let judge = CannedJudge::new(r#"{"verdict":"supported","confidence":1.0,"rationale":"x"}"#);
    let outcome = check_claim_judged(
        &c,
        slice::from_ref(&c),
        &corpus(),
        EvidenceStrength::Mixed,
        Some(&judge),
        None,
    )
    .await
    .expect("pipeline runs");
    assert_eq!(judge.call_count(), 0, "judge must not fire after Layer 2");
    assert_eq!(outcome.layer_fired, VerdictLayer::Deterministic);
    assert_eq!(outcome.status, ClaimStatus::Contradicted);
}

#[tokio::test]
async fn no_judge_settles_on_evidence_layer() {
    let c = claim(
        "Beetroot juice before a 10k improves time-trial performance.",
        ClaimCategory::Supplement,
    );
    let outcome = check_claim_judged(
        &c,
        slice::from_ref(&c),
        &corpus(),
        EvidenceStrength::Mixed,
        None,
        None,
    )
    .await
    .expect("pipeline runs without judge");
    assert_eq!(outcome.layer_fired, VerdictLayer::Evidence);
    assert_eq!(outcome.status, ClaimStatus::Unsupported);
}

// ---------------------------------------------------------------------------
// Layers 1–3 still behave through the new entry points
// ---------------------------------------------------------------------------

#[tokio::test]
async fn check_reply_preserves_rhetoric_short_circuit() {
    let claims = vec![claim(
        "You're crushing it, champ!",
        ClaimCategory::TrainingPrescription,
    )];
    let judge = CannedJudge::new(r#"{"verdict":"supported","confidence":1.0,"rationale":"x"}"#);
    let verdicts = check_reply(
        &claims,
        &corpus(),
        EvidenceStrength::Mixed,
        Some(&judge),
        None,
    )
    .await
    .expect("pipeline runs");
    assert_eq!(verdicts[0].1.layer_fired, VerdictLayer::Rhetoric);
    assert_eq!(
        judge.call_count(),
        0,
        "rhetoric short-circuits before judge"
    );
}

#[tokio::test]
async fn check_reply_preserves_evidence_support() {
    let claims = vec![claim(
        "Aim for 1.6 grams of protein per kg of body weight daily.",
        ClaimCategory::Nutrition,
    )];
    let verdicts = check_reply(&claims, &corpus(), EvidenceStrength::Mixed, None, None)
        .await
        .expect("pipeline runs");
    assert_eq!(verdicts[0].1.status, ClaimStatus::Supported);
    assert_eq!(verdicts[0].1.layer_fired, VerdictLayer::Evidence);
}
