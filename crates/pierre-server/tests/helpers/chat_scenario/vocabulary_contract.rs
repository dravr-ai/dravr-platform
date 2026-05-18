// ABOUTME: Coach vocabulary contracts loaded from the contremaitre manifest
// ABOUTME: One source of truth — same list referenced by prompt and test asserter
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coach vocabulary contracts.
//!
//! A *vocabulary contract* is a per-coach commitment that the coach
//! will use at least one term from a curated domain vocabulary in
//! every reply, regardless of the question topic. The same list lives
//! in two places:
//!
//! 1. **The coach's prompt** (in `dravr-contremaitre`) — instructs the
//!    LLM to always frame responses using these terms.
//! 2. **This registry** — loaded by the test runner so the
//!    `AssertionSpec::VocabularyContract` asserter can check the
//!    contract is being honoured.
//!
//! Keeping both in sync is the contributor's job. The registry below
//! ships fallback defaults so the framework is usable before the
//! contremaitre manifest extension lands; once it does, the loader
//! reads from there instead.

use std::collections::HashMap;

/// A single coach's vocabulary commitment.
#[derive(Debug, Clone)]
pub struct VocabularyContract {
    /// Lower-snake-case domain terms. Asserter matches case-insensitive
    /// substring; word-boundary refinement is a follow-up if false
    /// positives surface.
    pub terms: Vec<String>,
}

/// Registry of vocabulary contracts, keyed by coach id.
#[derive(Debug, Clone, Default)]
pub struct VocabularyContractRegistry {
    contracts: HashMap<String, VocabularyContract>,
}

impl VocabularyContractRegistry {
    /// Build a registry with the compile-time fallback contracts.
    ///
    /// These are the same terms the contremaitre prompt should
    /// declare — when contremaitre ships a `vocabulary_contract`
    /// field per coach (P4 follow-up), this loader will read from
    /// there instead of hard-coding.
    #[must_use]
    pub fn with_defaults() -> Self {
        let mut m = HashMap::new();
        m.insert(
            "strength".to_owned(),
            VocabularyContract {
                terms: vec![
                    "strength".to_owned(),
                    "resistance".to_owned(),
                    "lift".to_owned(),
                    "squat".to_owned(),
                    "muscle".to_owned(),
                    "deload".to_owned(),
                    "rir".to_owned(),
                    "rep".to_owned(),
                    "set".to_owned(),
                    "volume".to_owned(),
                    "hypertrophy".to_owned(),
                    "soreness".to_owned(),
                ],
            },
        );
        m.insert(
            "endurance".to_owned(),
            VocabularyContract {
                terms: vec![
                    "zone".to_owned(),
                    "tempo".to_owned(),
                    "threshold".to_owned(),
                    "z2".to_owned(),
                    "vo2".to_owned(),
                    "pace".to_owned(),
                    "cadence".to_owned(),
                    "long run".to_owned(),
                    "fartlek".to_owned(),
                    "interval".to_owned(),
                    "endurance".to_owned(),
                ],
            },
        );
        m.insert(
            "nutrition".to_owned(),
            VocabularyContract {
                terms: vec![
                    "glucides".to_owned(),
                    "protéines".to_owned(),
                    "carbs".to_owned(),
                    "protein".to_owned(),
                    "tdee".to_owned(),
                    "macros".to_owned(),
                    "hydration".to_owned(),
                    "fuelling".to_owned(),
                    "calories".to_owned(),
                    "kcal".to_owned(),
                ],
            },
        );
        m.insert(
            "mobility".to_owned(),
            VocabularyContract {
                terms: vec![
                    "mobility".to_owned(),
                    "stretch".to_owned(),
                    "warm-up".to_owned(),
                    "cool-down".to_owned(),
                    "range of motion".to_owned(),
                    "rom".to_owned(),
                    "foam roll".to_owned(),
                    "activation".to_owned(),
                ],
            },
        );
        Self { contracts: m }
    }

    /// Empty registry — used in unit tests that supply their own.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            contracts: HashMap::new(),
        }
    }

    /// Install or overwrite a contract.
    pub fn insert(&mut self, coach_id: String, contract: VocabularyContract) {
        self.contracts.insert(coach_id, contract);
    }

    /// Look up by coach id.
    #[must_use]
    pub fn contract_for(&self, coach_id: &str) -> Option<&VocabularyContract> {
        self.contracts.get(coach_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_cover_the_four_coach_archetypes() {
        let r = VocabularyContractRegistry::with_defaults();
        assert!(r.contract_for("strength").is_some());
        assert!(r.contract_for("endurance").is_some());
        assert!(r.contract_for("nutrition").is_some());
        assert!(r.contract_for("mobility").is_some());
        assert!(r.contract_for("nonexistent_coach").is_none());
    }

    #[test]
    fn strength_contract_includes_recovery_aware_terms() {
        // The Monitor: LLM Live Integration test failure showed the
        // strength coach answering recovery questions with sleep
        // vocabulary. The contract must cover recovery framing too —
        // deload, soreness, RIR — so the asserter accepts the LLM's
        // recovery answer when framed in strength terms.
        let r = VocabularyContractRegistry::with_defaults();
        let c = r.contract_for("strength").unwrap();
        assert!(c.terms.iter().any(|t| t == "deload"));
        assert!(c.terms.iter().any(|t| t == "soreness"));
    }
}
