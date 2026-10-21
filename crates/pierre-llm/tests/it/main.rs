// ABOUTME: One integration-test binary for pierre-llm: every suite is a module here
// ABOUTME: Suites that mutate the process environment stay separate binaries in tests/
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Integration tests for the LLM crate, linked as one binary.
//!
//! A top-level file in `tests/` is a test binary of its own, linking its own
//! copy of the crate. Only a suite that writes the process environment stays
//! one, because the environment is shared by every test in a binary.

mod chain_assembly_test;
mod chain_cooldown_skip_notify_test;
mod chain_guard_lock;
mod chain_tier_probe_test;
mod failed_leg_token_accounting_test;
mod github_headroom_guard_test;
mod judge_extract_json_test;
mod llm_health_last_success_test;
mod llm_request_span_test;
mod prompt_schema_test;
mod provider_rate_limit_test;
mod provider_stop_test;
mod router_provider_selection_test;
mod served_tier_test;
mod smoke_test;
mod stage_routing_test;
mod tool_chain_preemptive_skip_test;
mod tool_types_wire_test;
