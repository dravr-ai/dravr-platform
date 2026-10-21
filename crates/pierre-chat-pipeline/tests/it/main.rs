// ABOUTME: One integration-test binary for pierre-chat-pipeline: every suite is a module here
// ABOUTME: Suites that mutate the process environment stay separate binaries in tests/
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Integration tests for the chat pipeline crate, linked as one binary.
//!
//! A top-level file in `tests/` is a test binary of its own, linking its own
//! copy of the crate. Only a suite that writes the process environment stays
//! one, because the environment is shared by every test in a binary.

mod calibration_turn_test;
mod command_reply_replay_test;
mod commitments_block_test;
mod embacle_transport_contract_test;
mod first_reply_introduction_test;
mod fortnight_rail_budget_test;
mod freshness_hint_turn_clock_test;
mod identity_anchor_survives_hardening_test;
mod onboarding_directive_tail_test;
mod persona_contract_rules_test;
mod persona_enforcement_test;
mod persona_rewrite_recheck_test;
mod prefetch_grounding_test;
mod progressive_support_test;
mod season_turn_test;
mod single_system_message_invariant_test;
mod static_prompt_budget_test;
mod surface_capability_catalogue_test;
mod tool_discipline_shared_test;
mod turn_directive_test;
mod viz_blocks_test;
mod viz_repair_test;
mod viz_route_hydration_test;
mod viz_schema_test;
mod wire_shape_telemetry_test;
mod withheld_reply_replay_test;
mod withheld_turn_learning_test;
