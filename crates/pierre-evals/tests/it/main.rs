// ABOUTME: One integration-test binary for pierre-evals: every suite is a module here
// ABOUTME: Suites that mutate the process environment stay separate binaries in tests/
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Integration tests for the coaching evals crate, linked as one binary.
//!
//! A top-level file in `tests/` is a test binary of its own, linking its own
//! copy of the crate. Only a suite that writes the process environment stays
//! one, because the environment is shared by every test in a binary.

mod athlete_data_layer_test;
mod athlete_data_named_activity_test;
mod athlete_data_period_test;
mod bullshit_detector_test;
mod claim_extractor_test;
mod claim_verifier_layers_test;
mod critical_power_verification_test;
mod deterministic_bounds_test;
mod deterministic_test;
mod eval_pipeline_test;
mod evidence_retriever_test;
mod explanation_gen_test;
mod fixtures_test;
mod judge_test;
mod multi_turn_test;
mod personalized_form_test;
mod personalized_locale_probe_test;
mod personalized_test;
mod recovery_claim_pipeline_test;
mod report_test;
mod rhetoric_detector_test;
mod rubrics_test;
mod seeded_fabrications_test;
mod verification_config_test;
