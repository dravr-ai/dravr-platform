// ABOUTME: Coaching evaluation harness — golden sets, LLM-as-judge rubrics, deterministic checks
// ABOUTME: Tier 5 of the coaching harness. Pure types + helpers; no MCP server dependencies.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![deny(unsafe_code)]

//! # Pierre Evaluation Harness
//!
//! Tooling for evaluating dravr's coaching responses with three layers of
//! checks per the gist (Part 7):
//!
//! 1. **Deterministic** — schema/structure validation, persona keyword
//!    presence, prompt-injection pattern detection.
//! 2. **LLM-as-judge** — rubric-based scoring against golden fixtures.
//! 3. **Multi-turn** — sliding-window scoring over a synthetic
//!    conversation.
//!
//! Layers 1 and 3 are pure-Rust and live entirely in this crate. Layer 2
//! delegates to [`pierre_llm::judge`] (lifted in Tier −1) so the same JSON
//! parsing plumbing is reused everywhere a structured verdict is needed.

/// Tier 5.5 Layer 0 — claim extraction from coach replies.
pub mod claim_extractor;
/// Deterministic structural and content checks (Layer 1).
pub mod deterministic;
/// Tier 5.5 Layer 2 — deterministic per-category bounds.
pub mod deterministic_bounds;
/// Tier 5.5 Layer 3 — evidence retrieval from the curated sports-science corpus.
pub mod evidence_retriever;
/// Tier 5.5 Layer 6 — user-facing explanation rendering.
pub mod explanation_gen;
/// Golden fixture loader (Layer 2 / 3 source data).
pub mod fixtures;
/// LLM-as-judge invocation backed by `pierre_llm::judge`.
pub mod judge;
/// Multi-turn sliding-window evaluator (Layer 3).
pub mod multi_turn;
/// Aggregated rubric-driven scoring report.
pub mod report;
/// Tier 5.5 Layer 1 — rhetoric vs propositional classifier.
pub mod rhetoric_detector;
/// Rubric definitions used by the judge.
pub mod rubrics;
/// Tier 5.5 pipeline synthesis — runs claims through layers 1–3 and emits a verdict.
pub mod verdict_engine;
/// Tier 5.5 per-coach verification config loaded from YAML frontmatter.
pub mod verification_config;

pub use claim_extractor::{extract_heuristic, extract_with_llm, ExtractedClaim};
pub use deterministic::{DeterministicCheck, DeterministicReport};
pub use evidence_retriever::{EvidenceCorpus, EvidenceMatch, EvidenceRecord};
pub use fixtures::{GoldenCase, GoldenFixture, Turn};
pub use judge::{JudgeVerdict, RubricScore};
pub use multi_turn::{MultiTurnEvaluator, MultiTurnReport};
pub use report::{EvalSummary, RubricKind};
pub use rhetoric_detector::{classify as classify_rhetoric, RhetoricVerdict};
pub use rubrics::Rubric;
pub use verdict_engine::{check_claim, VerdictOutcome};
pub use verification_config::{CategoryConfig, VerificationConfig, VerificationFallback};
