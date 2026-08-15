// ABOUTME: Runtime LLM-judge wiring tests — the resolve_claim_judge seam + a judge verdict changing an outcome
// ABOUTME: Pins factory-based resolution (the ctx.llm_provider-only read was dead code on every production turn)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Runtime claim-judge tests.
//
// This `//!` must precede the crate-level `#![cfg]`: when the feature is off the
// cfg empties the crate (dropping any inner `#![allow(missing_docs)]`), so without
// a surviving crate doc the command-line `-D warnings` trips `missing_docs`.
#![cfg(feature = "tools-verification")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;

use common::TestLlmProvider;
use pierre_chat_pipeline::stages::verification::resolve_claim_judge;
use pierre_evals::{EvidenceCorpus, VerificationConfig};
use pierre_llm::{ChatProvider, LlmProvider};
use pierre_memory::claims::{ClaimStatus, VerdictLayer};
use pierre_services::claim_verification::verify_reply_with_config_and_judge;

fn chat_provider() -> Arc<ChatProvider> {
    Arc::new(ChatProvider::Custom(Arc::new(TestLlmProvider::valid())))
}

fn llm_provider() -> Arc<dyn LlmProvider> {
    Arc::new(TestLlmProvider::valid())
}

#[test]
fn judge_resolution_follows_the_config_gate_and_the_factory_order() {
    let chat = chat_provider();
    let llm = llm_provider();

    // Disabled → no judge, even with both providers wired.
    assert!(resolve_claim_judge(false, Some(&chat), Some(&llm)).is_none());

    // Enabled + production wiring (chat_provider set, llm_provider None) —
    // the case the old ctx.llm_provider read silently lost.
    let resolved = resolve_claim_judge(true, Some(&chat), None).expect("chat provider resolves");
    assert_eq!(resolved.name(), "test");

    // Enabled + test wiring (llm_provider only) still resolves via Custom.
    let resolved = resolve_claim_judge(true, None, Some(&llm)).expect("llm provider resolves");
    assert_eq!(resolved.name(), "test");

    // Enabled + nothing wired → deterministic-only, no panic, no error.
    assert!(resolve_claim_judge(true, None, None).is_none());
}

/// A quantitative physiological claim (extractable via the "lactate
/// threshold" keyword bucket) with plausible numbers (no deterministic-bounds
/// trip) and no sibling contradiction. Run against an EMPTY corpus, the
/// evidence layer can never confidently resolve it — so Layer 5 is
/// deterministically the deciding voice when a judge is present.
const INCONCLUSIVE_REPLY: &str =
    "Drinking beetroot juice daily raises your lactate threshold by 12% within two weeks.";

#[tokio::test]
async fn a_wired_judge_decides_claims_the_deterministic_layers_cannot() {
    let config = VerificationConfig::default();
    let empty_corpus = EvidenceCorpus::default();

    // Without a judge: the claim extracts but settles on a deterministic
    // layer — nothing may settle on the Judge layer.
    let without = verify_reply_with_config_and_judge(
        INCONCLUSIVE_REPLY,
        &config,
        &empty_corpus,
        None,
        None,
        None,
    )
    .await
    .expect("deterministic pipeline succeeds");
    assert!(
        !without.is_empty(),
        "the lactate-threshold claim must be extracted"
    );
    assert!(
        without
            .iter()
            .all(|(_, v)| v.layer_fired != VerdictLayer::Judge),
        "no judge wired, so no Judge-layer verdicts"
    );

    // With a scripted judge contradicting the claim: the Judge layer fires
    // and its verdict carries the judge's status + rationale.
    let judge = TestLlmProvider::with_response(
        r#"{"verdict":"contradicted","confidence":0.9,"rationale":"No consensus supports this effect size."}"#
            .to_owned(),
    );
    let with = verify_reply_with_config_and_judge(
        INCONCLUSIVE_REPLY,
        &config,
        &empty_corpus,
        Some(&judge),
        None,
        None,
    )
    .await
    .expect("judged pipeline succeeds");

    let judged: Vec<_> = with
        .iter()
        .filter(|(_, v)| v.layer_fired == VerdictLayer::Judge)
        .collect();
    assert!(
        !judged.is_empty(),
        "the lactate-threshold claim must reach Layer 5 (got: {:?})",
        with.iter()
            .map(|(c, v)| (c.text.clone(), v.layer_fired))
            .collect::<Vec<_>>()
    );
    for (_, verdict) in &judged {
        assert_eq!(verdict.status, ClaimStatus::Contradicted);
        assert!(
            verdict.explanation.contains("No consensus supports"),
            "explanation must carry the judge rationale, got: {}",
            verdict.explanation
        );
    }
}
