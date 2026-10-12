// ABOUTME: Claim verification service — lazy-loaded evidence corpus + pipeline runner
// ABOUTME: Falls back to the evidence corpus the pinned dravr-contremaitre crate compiles in
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Claim Verification Service
//!
//! Dispatch pipeline runners: thin wrappers run a single claim (or a whole
//! agent reply) through the detector pipeline.
//!
//! The corpus comes from [`pierre_contremaitre::EvidenceRegistry`]: its
//! `resolved_corpus` prefers the propositions synced from dravr-contremaitre
//! and falls back to [`compiled_in_corpus`], the files the pinned crate
//! compiles in, so the service works offline. The runners without a corpus
//! argument verify against that compiled-in fallback.

use pierre_contremaitre::evidence_registry::compiled_in_corpus;
use pierre_evals::{
    check_claim, claim_extractor::ExtractedClaim, evidence_retriever::EvidenceCorpus,
    extract_heuristic, VerdictOutcome, VerificationConfig,
};
use pierre_memory::claims::EvidenceStrength;
use std::slice;
use tracing::warn;

/// Verify an agent reply against the compiled-in fallback corpus.
///
/// Thin wrapper over [`verify_reply_heuristic_with`] for callers that
/// don't have a [`ServerContext`] handy (tests, tool dispatch when the
/// registry is not yet initialized). Production dispatch should prefer
/// [`verify_reply_heuristic_with`] with the registry's `resolved_corpus`.
#[must_use]
pub fn verify_reply_heuristic(
    agent_reply: &str,
    minimum_strength: EvidenceStrength,
) -> Vec<(ExtractedClaim, VerdictOutcome)> {
    verify_reply_heuristic_with(agent_reply, minimum_strength, compiled_in_corpus())
}

/// Verify an agent reply end-to-end against a caller-provided corpus.
///
/// Extracts claims heuristically, runs each through the detector pipeline
/// at the provided minimum evidence strength, and returns a list of
/// `(claim, outcome)` pairs. Never calls an LLM; safe to run synchronously
/// from any dispatch path.
#[must_use]
pub fn verify_reply_heuristic_with(
    agent_reply: &str,
    minimum_strength: EvidenceStrength,
    corpus: &EvidenceCorpus,
) -> Vec<(ExtractedClaim, VerdictOutcome)> {
    let claims = extract_heuristic(agent_reply);
    if claims.is_empty() {
        return Vec::new();
    }
    // Pass the full sibling set so the consistency-check layer can cross-check each
    // claim against the others in the same reply.
    claims
        .iter()
        .map(|claim| {
            let outcome = check_claim(claim, &claims, corpus, minimum_strength, None, None);
            (claim.clone(), outcome)
        })
        .collect()
}

/// Verify an agent reply honoring the per-agent [`VerificationConfig`],
/// against the compiled-in fallback corpus.
#[must_use]
pub fn verify_reply_with_config(
    agent_reply: &str,
    config: &VerificationConfig,
) -> Vec<(ExtractedClaim, VerdictOutcome)> {
    verify_reply_with_config_and_corpus(agent_reply, config, compiled_in_corpus())
}

/// Verify an agent reply honoring the per-agent [`VerificationConfig`],
/// against a caller-provided corpus.
///
/// Returns an empty vec when the config has `enabled = false`. For enabled
/// configs, each extracted claim is filtered by its category's enabled flag
/// and checked at the category's `min_strength` threshold. Categories the
/// agent opted out of are silently dropped (not even persisted as verdicts).
#[must_use]
pub fn verify_reply_with_config_and_corpus(
    agent_reply: &str,
    config: &VerificationConfig,
    corpus: &EvidenceCorpus,
) -> Vec<(ExtractedClaim, VerdictOutcome)> {
    if !config.enabled {
        return Vec::new();
    }
    let claims = extract_heuristic(agent_reply);
    if claims.is_empty() {
        return Vec::new();
    }
    // Cross-check the consistency-check layer against every extracted claim, not just the enabled
    // ones — a self-contradiction is worth flagging even if the sibling falls
    // in a category the agent opted out of persisting.
    claims
        .iter()
        .filter(|claim| config.is_enabled_for(claim.category))
        .map(|claim| {
            let min_strength = config.for_category(claim.category).min_strength;
            let outcome = check_claim(claim, &claims, corpus, min_strength, None, None);
            (claim.clone(), outcome)
        })
        .collect()
}

/// Verify a single claim against a caller-provided corpus.
///
/// Used by the `verify_claim` MCP tool to honor the runtime registry
/// without leaking the singleton through its API surface.
#[must_use]
pub fn verify_single_claim_with(
    claim: &ExtractedClaim,
    minimum_strength: EvidenceStrength,
    corpus: &EvidenceCorpus,
) -> VerdictOutcome {
    // Single-claim path: no siblings for the consistency-check layer to cross-check against.
    check_claim(
        claim,
        slice::from_ref(claim),
        corpus,
        minimum_strength,
        None,
        None,
    )
}

/// Warm the corpus at startup and log its size.
///
/// Called once from server boot so parse failures surface early instead
/// of mid-request.
pub fn warm_corpus() {
    let c = compiled_in_corpus();
    if c.is_empty() {
        warn!("evidence corpus is empty — verification will fall through to Unsupported");
    } else {
        let count = c.len();
        tracing::info!("evidence corpus loaded: {count} propositions");
    }
}
