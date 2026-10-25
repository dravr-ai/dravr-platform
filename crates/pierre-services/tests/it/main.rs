// ABOUTME: One integration-test binary for pierre-services: every suite is a module here
// ABOUTME: Suites that mutate the process environment stay separate binaries in tests/
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Integration tests for the services crate, linked as one binary.
//!
//! A top-level file in `tests/` is a test binary of its own, linking its own
//! copy of the crate. Only a suite that writes the process environment stays
//! one, because the environment is shared by every test in a binary.

mod activity_sports_test;
mod advice_capture_test;
mod agent_prerequisites_test;
mod archetype_aggregation_test;
mod commitment_sweep_test;
mod debug_redaction_test;
mod fact_sentence_test;
mod fortnight_decision_test;
mod llm_probe_skip_test;
mod memory_dedup_test;
mod memory_extraction_parsing_test;
mod okf_bundle_test;
mod outcome_evaluator_test;
mod parq_questions_test;
mod plan_card_float_precision_test;
mod playbook_render_test;
mod provider_refresh_bound_test;
mod ramp_check_test;
mod recent_load_snapshot_test;
mod side_call_usage_test;
mod spawn_periodic_test;
mod training_plan_render_test;
