// ABOUTME: One integration-test binary for pierre-cli: every suite is a module here
// ABOUTME: Suites that mutate the process environment stay separate binaries in tests/
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Integration tests for pierre-cli, linked as one binary.
//!
//! A top-level file in `tests/` is a test binary of its own, linking its own
//! copy of the crate. Only a suite that writes the process environment stays
//! one, because the environment is shared by every test in a binary.

mod admin_role_test;
mod device_flow_test;
mod pierre_cli_test;
mod remote_token_target_test;
mod sciotte_login_canary_test;
mod signup_code_cli_test;
mod strava_webhook_test;
mod token_generate_output_test;
mod user_removal_cli_test;
