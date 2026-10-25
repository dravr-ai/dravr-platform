// ABOUTME: One integration-test binary for pierre-core: every suite is a module here
// ABOUTME: Suites that mutate the process environment stay separate binaries in tests/
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Integration tests for the core crate, linked as one binary.
//!
//! A top-level file in `tests/` is a test binary of its own, linking its own
//! copy of the crate. Only a suite that writes the process environment stays
//! one, because the environment is shared by every test in a binary.

mod admin_token_gate_test;
mod admin_token_request_test;
mod agent_translation_test;
#[cfg(feature = "billing")]
mod billing_dummy_test;
#[cfg(feature = "billing-stripe")]
mod billing_stripe_test;
mod calibration_topic_test;
mod civil_time_clock_date_test;
mod civil_time_relative_day_test;
mod debug_redaction_test;
mod feature_flags_test;
mod field_update_test;
mod form_reading_test;
mod guided_window_test;
mod hosted_page_css_test;
mod identity_leak_context_test;
mod identity_leak_denial_test;
mod identity_leak_self_id_test;
mod markdown_test;
mod narration_mechanics_selftalk_test;
mod narration_peer_denial_test;
mod narration_scrub_test;
mod onboarding_state_test;
mod pagination_args_test;
mod pillar_test;
mod prompt_fingerprint_test;
mod provider_auth_required_test;
mod redaction_test;
mod refresh_test;
mod replayable_content_test;
mod reply_chunking_test;
mod safety_test;
mod season_topic_test;
mod sport_family_test;
mod sport_type_alias_test;
mod tokens_test;
#[cfg(feature = "telemetry")]
mod trace_propagation_test;
mod ungrounded_appeal_scrub_test;
mod untrusted_athlete_text_test;
mod untrusted_display_line_test;
