// ABOUTME: Tier 5.5 claim verification service — lazy-loaded evidence corpus + pipeline runner
// ABOUTME: Embeds 26 markdown propositions via include_str!; corpus moves to dravr-contremaitre in Phase A follow-up
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Claim Verification Service
//!
//! Process-wide singleton [`EvidenceCorpus`] + dispatch pipeline runners.
//!
//! The corpus is parsed from the 26 embedded markdown proposition files
//! under `crates/pierre-evals/fixtures/sports_science/`. Thin wrappers run
//! a single claim (or a whole coach reply) through the detector pipeline.
//!
//! Each proposition is a markdown file with YAML frontmatter matching the
//! dravr-contremaitre prompts/ convention. Files are compiled into the
//! binary so the service works offline and requires no filesystem access.
//! The runtime [`crate::contremaitre::EvidenceRegistry`] picks up freshly
//! published propositions via webhook sync; this module falls back to the
//! embedded files when the registry is empty.

use crate::mcp::resources::ServerResources;
use pierre_evals::{
    check_claim, claim_extractor::ExtractedClaim, evidence_retriever::EvidenceCorpus,
    extract_heuristic, VerdictOutcome, VerificationConfig,
};
use pierre_memory::claims::EvidenceStrength;
use std::sync::OnceLock;
use tracing::{error, warn};

/// All 26 embedded proposition files as (filename, contents) pairs.
/// Organized by category to match the `dravr-contremaitre` `evidence/`
/// layout. Adding a new proposition means adding both a new `.md` file
/// under `crates/pierre-evals/fixtures/sports_science/` and a new line
/// in this table.
const EMBEDDED_PROPOSITIONS: &[(&str, &str)] = &[
    // Nutrition (5)
    (
        "nutrition/morton-2018-protein-intake.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/nutrition/morton-2018-protein-intake.md"
        ),
    ),
    (
        "nutrition/jeukendrup-2014-carbs-during-exercise.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/nutrition/jeukendrup-2014-carbs-during-exercise.md"
        ),
    ),
    (
        "nutrition/schoenfeld-aragon-2018-protein-timing.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/nutrition/schoenfeld-aragon-2018-protein-timing.md"
        ),
    ),
    (
        "nutrition/hew-butler-2008-hyponatremia.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/nutrition/hew-butler-2008-hyponatremia.md"
        ),
    ),
    (
        "nutrition/sim-2019-iron-deficiency.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/nutrition/sim-2019-iron-deficiency.md"
        ),
    ),
    // Supplement (4)
    (
        "supplement/issn-2017-creatine.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/supplement/issn-2017-creatine.md"
        ),
    ),
    (
        "supplement/issn-2021-caffeine.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/supplement/issn-2021-caffeine.md"
        ),
    ),
    (
        "supplement/wolfe-2017-bcaa.md",
        include_str!("../../../pierre-evals/fixtures/sports_science/supplement/wolfe-2017-bcaa.md"),
    ),
    (
        "supplement/issn-2015-beta-alanine.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/supplement/issn-2015-beta-alanine.md"
        ),
    ),
    // Physiological (5)
    (
        "physiological/tanaka-2001-hrmax-formula.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/physiological/tanaka-2001-hrmax-formula.md"
        ),
    ),
    (
        "physiological/saltin-1968-vo2max-elite.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/physiological/saltin-1968-vo2max-elite.md"
        ),
    ),
    (
        "physiological/faude-2009-lactate-threshold.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/physiological/faude-2009-lactate-threshold.md"
        ),
    ),
    (
        "physiological/barnes-kilding-2015-running-economy.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/physiological/barnes-kilding-2015-running-economy.md"
        ),
    ),
    (
        "physiological/bergstrom-glycogen-depletion.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/physiological/bergstrom-glycogen-depletion.md"
        ),
    ),
    // Training prescription (5)
    (
        "training_prescription/nielsen-2012-volume-injury.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/training_prescription/nielsen-2012-volume-injury.md"
        ),
    ),
    (
        "training_prescription/rosenblat-2020-polarized-training.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/training_prescription/rosenblat-2020-polarized-training.md"
        ),
    ),
    (
        "training_prescription/schoenfeld-2017-strength-training.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/training_prescription/schoenfeld-2017-strength-training.md"
        ),
    ),
    (
        "training_prescription/mujika-padilla-2003-taper.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/training_prescription/mujika-padilla-2003-taper.md"
        ),
    ),
    (
        "training_prescription/denadai-2017-plyometrics.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/training_prescription/denadai-2017-plyometrics.md"
        ),
    ),
    // Recovery (4)
    (
        "recovery/watson-2015-sleep-duration.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/recovery/watson-2015-sleep-duration.md"
        ),
    ),
    (
        "recovery/roberts-2015-cold-immersion.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/recovery/roberts-2015-cold-immersion.md"
        ),
    ),
    (
        "recovery/plews-2013-hrv-recovery.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/recovery/plews-2013-hrv-recovery.md"
        ),
    ),
    (
        "recovery/scoon-2007-sauna-heat-adaptation.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/recovery/scoon-2007-sauna-heat-adaptation.md"
        ),
    ),
    // Injury rehab (3)
    (
        "injury_rehab/bjsm-2017-achilles-return-to-run.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/injury_rehab/bjsm-2017-achilles-return-to-run.md"
        ),
    ),
    (
        "injury_rehab/jospt-2019-acl-return-to-sport.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/injury_rehab/jospt-2019-acl-return-to-sport.md"
        ),
    ),
    (
        "injury_rehab/alfredson-1998-eccentric-achilles.md",
        include_str!(
            "../../../pierre-evals/fixtures/sports_science/injury_rehab/alfredson-1998-eccentric-achilles.md"
        ),
    ),
];

static CORPUS: OnceLock<EvidenceCorpus> = OnceLock::new();

/// Return the compiled-in fallback corpus.
///
/// Parses the 26 embedded markdown propositions on first call. Parse
/// failures log an error and return an empty corpus so verification
/// gracefully degrades rather than panicking.
///
/// **This is the fallback source.** Production dispatch should use
/// [`resolve_corpus`] to prefer the runtime `EvidenceRegistry` populated
/// from dravr-contremaitre and fall back here only when the registry is
/// empty.
#[must_use]
pub fn corpus() -> &'static EvidenceCorpus {
    CORPUS.get_or_init(|| {
        match EvidenceCorpus::from_markdown_files(EMBEDDED_PROPOSITIONS.iter().copied()) {
            Ok(c) => c,
            Err(e) => {
                error!("Failed to parse embedded sports-science corpus: {e}");
                EvidenceCorpus::default()
            }
        }
    })
}

/// Resolve the evidence corpus for a given request.
///
/// Prefers the runtime [`crate::contremaitre::EvidenceRegistry`] sourced
/// from dravr-contremaitre when it holds at least one proposition, falling
/// back to the compiled-in [`corpus`] otherwise. The contremaitre registry
/// is always non-empty when a successful startup sync + webhook pipeline
/// is in place; the fallback path only fires on:
///
/// - First boot before `full_sync` completes
/// - `CONTREMAITRE_REPO` unset (contremaitre disabled)
/// - GitHub unreachable at startup + every subsequent sync attempt
/// - An empty `evidence/` tree on the remote manifest
///
/// Returns an owned [`EvidenceCorpus`] because the registry aggregates
/// per-proposition corpora on every call; cloning is cheap (a `Vec` of
/// small records) and keeps the lock hold time minimal.
#[must_use]
#[cfg(feature = "contremaitre")]
pub fn resolve_corpus(resources: &ServerResources) -> EvidenceCorpus {
    let runtime = resources.evidence_registry.full_corpus();
    if runtime.is_empty() {
        corpus().clone()
    } else {
        runtime
    }
}

/// Non-contremaitre build fallback: always return the compiled-in corpus.
#[must_use]
#[cfg(not(feature = "contremaitre"))]
pub fn resolve_corpus(_resources: &ServerResources) -> EvidenceCorpus {
    corpus().clone()
}

/// Verify a coach reply against the compiled-in fallback corpus.
///
/// Thin wrapper over [`verify_reply_heuristic_with`] for callers that
/// don't have a [`ServerResources`] handy (tests, tool dispatch when the
/// registry is not yet initialized). Production dispatch should prefer
/// [`verify_reply_heuristic_with`] with [`resolve_corpus`].
#[must_use]
pub fn verify_reply_heuristic(
    coach_reply: &str,
    minimum_strength: EvidenceStrength,
) -> Vec<(ExtractedClaim, VerdictOutcome)> {
    verify_reply_heuristic_with(coach_reply, minimum_strength, corpus())
}

/// Verify a coach reply end-to-end against a caller-provided corpus.
///
/// Extracts claims heuristically, runs each through the detector pipeline
/// at the provided minimum evidence strength, and returns a list of
/// `(claim, outcome)` pairs. Never calls an LLM; safe to run synchronously
/// from any dispatch path.
#[must_use]
pub fn verify_reply_heuristic_with(
    coach_reply: &str,
    minimum_strength: EvidenceStrength,
    corpus: &EvidenceCorpus,
) -> Vec<(ExtractedClaim, VerdictOutcome)> {
    let claims = extract_heuristic(coach_reply);
    if claims.is_empty() {
        return Vec::new();
    }
    claims
        .into_iter()
        .map(|claim| {
            let outcome = check_claim(&claim, corpus, minimum_strength);
            (claim, outcome)
        })
        .collect()
}

/// Verify a coach reply honoring the per-coach [`VerificationConfig`],
/// against the compiled-in fallback corpus.
#[must_use]
pub fn verify_reply_with_config(
    coach_reply: &str,
    config: &VerificationConfig,
) -> Vec<(ExtractedClaim, VerdictOutcome)> {
    verify_reply_with_config_and_corpus(coach_reply, config, corpus())
}

/// Verify a coach reply honoring the per-coach [`VerificationConfig`],
/// against a caller-provided corpus.
///
/// Returns an empty vec when the config has `enabled = false`. For enabled
/// configs, each extracted claim is filtered by its category's enabled flag
/// and checked at the category's `min_strength` threshold. Categories the
/// coach opted out of are silently dropped (not even persisted as verdicts).
#[must_use]
pub fn verify_reply_with_config_and_corpus(
    coach_reply: &str,
    config: &VerificationConfig,
    corpus: &EvidenceCorpus,
) -> Vec<(ExtractedClaim, VerdictOutcome)> {
    if !config.enabled {
        return Vec::new();
    }
    let claims = extract_heuristic(coach_reply);
    if claims.is_empty() {
        return Vec::new();
    }
    claims
        .into_iter()
        .filter(|claim| config.is_enabled_for(claim.category))
        .map(|claim| {
            let min_strength = config.for_category(claim.category).min_strength;
            let outcome = check_claim(&claim, corpus, min_strength);
            (claim, outcome)
        })
        .collect()
}

/// Verify a single caller-provided claim (already extracted) at the given
/// minimum evidence strength, using the compiled-in fallback corpus.
#[must_use]
pub fn verify_single_claim(
    claim: &ExtractedClaim,
    minimum_strength: EvidenceStrength,
) -> VerdictOutcome {
    check_claim(claim, corpus(), minimum_strength)
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
    check_claim(claim, corpus, minimum_strength)
}

/// Warm the corpus at startup and log its size.
///
/// Called once from server boot so parse failures surface early instead
/// of mid-request.
pub fn warm_corpus() {
    let c = corpus();
    if c.is_empty() {
        warn!("Tier 5.5 evidence corpus is empty — verification will fall through to Unsupported");
    } else {
        let count = c.len();
        tracing::info!("Tier 5.5 evidence corpus loaded: {count} propositions");
    }
}
