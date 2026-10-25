// ABOUTME: One integration-test binary for pierre-auth: every suite is a module here
// ABOUTME: Suites that mutate the process environment stay separate binaries in tests/
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Integration tests for the auth crate, linked as one binary.
//!
//! A top-level file in `tests/` is a test binary of its own, linking its own
//! copy of the crate. Only a suite that writes the process environment stays
//! one, because the environment is shared by every test in a binary.

mod claims_client_id_test;
mod client_address_test;
mod debug_redaction_test;
mod google_sign_in_config_test;
mod kek_provider_test;
mod mcp_resource_indicator_test;
mod oauth_issuer_test;
mod smoke_test;
