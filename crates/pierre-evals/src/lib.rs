// ABOUTME: Coaching evaluation harness — golden sets, LLM-as-judge rubrics, deterministic checks
// ABOUTME: Tier 5 of the coaching harness. Pure types + helpers; no MCP server dependencies.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![deny(unsafe_code)]

//! # Pierre Evaluation Harness
//!
//! Tooling for evaluating dravr's coaching responses with three kinds of
//! checks:
//!
//! - **Deterministic** — schema/structure validation, persona keyword
//!   presence, prompt-injection pattern detection.
//! - **LLM-as-judge** — rubric-based scoring against golden fixtures.
//! - **Multi-turn** — sliding-window scoring over a synthetic conversation.
//!
//! The deterministic and multi-turn checks are pure-Rust and live entirely in
//! this crate; the LLM-as-judge check delegates to [`pierre_llm::judge`] so the
//! same JSON parsing plumbing is reused everywhere a structured verdict is needed.
//!
//! The crate also houses the **claim-verification** ("bullshit detector") stages
//! that run post-LLM over a coach reply — rhetoric filter, deterministic bounds,
//! personalized physiology, evidence retrieval, consistency check, and the LLM
//! judge — synthesized by [`verdict_engine`] into a single verdict.

/// Claim verifier: claims about the athlete's own records, checked against
/// their own data rather than the literature.
pub mod athlete_data;
/// Claim verifier: claim extraction from coach replies.
pub mod claim_extractor;
/// Claim verifier: consistency cross-check against sibling claims.
pub mod consistency;
/// Eval harness: deterministic structural and content checks.
pub mod deterministic;
/// Claim verifier: deterministic per-category bounds against population limits.
pub mod deterministic_bounds;
/// Claim verifier: evidence retrieval from the curated sports-science corpus.
pub mod evidence_retriever;
/// Claim verifier: user-facing explanation rendering.
pub mod explanation_gen;
/// Eval harness: golden fixture loader.
pub mod fixtures;
/// LLM-as-judge invocation backed by `pierre_llm::judge` (used by both the eval
/// harness and the claim verifier).
pub mod judge;
/// Eval harness: multi-turn sliding-window evaluator.
pub mod multi_turn;
/// Claim verifier: claim checked against the athlete's own computed physiology.
pub mod personalized;
/// Aggregated rubric-driven scoring report.
pub mod report;
/// Claim verifier: rhetoric vs propositional classifier.
pub mod rhetoric_detector;
/// Rubric definitions used by the judge.
pub mod rubrics;
/// Claim verifier: pipeline synthesis — runs a claim through the verifier stages
/// and emits a single verdict.
pub mod verdict_engine;
/// Claim verifier: per-coach verification config loaded from YAML frontmatter.
pub mod verification_config;

pub use claim_extractor::{extract_heuristic, extract_with_llm, ExtractedClaim};
pub use consistency::{find_contradiction, ConsistencyConflict};
pub use deterministic::{DeterministicCheck, DeterministicReport};
pub use evidence_retriever::{EvidenceCorpus, EvidenceMatch, EvidenceRecord};
pub use fixtures::{GoldenCase, GoldenFixture, Turn};
pub use judge::{ClaimJudgement, JudgeVerdict, RubricScore};
pub use multi_turn::{MultiTurnEvaluator, MultiTurnReport};
pub use personalized::{
    AthleteMetrics, CoachConfiguredStrategy, ConservativeStrategy, PersonalizedContext,
    TightStrategy, ToleranceCall, ToleranceStrategy,
};
pub use report::{EvalSummary, RubricKind};
pub use rhetoric_detector::{classify as classify_rhetoric, RhetoricVerdict};
pub use rubrics::Rubric;
pub use verdict_engine::{check_claim, check_claim_judged, check_reply, VerdictOutcome};
pub use verification_config::{
    ActionMode, AuditOnlyPolicy, CategoryConfig, ContradictionPolicy, InheritConfigPolicy,
    PersonalizedConfig, ResolvedAction, ToleranceMode, UserWarnPolicy, VerificationConfig,
    VerificationFallback,
};
