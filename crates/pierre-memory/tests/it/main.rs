// ABOUTME: One integration-test binary for pierre-memory: every suite is a module here
// ABOUTME: Suites that mutate the process environment stay separate binaries in tests/
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Integration tests for the memory crate, linked as one binary.
//!
//! A top-level file in `tests/` is a test binary of its own, linking its own
//! copy of the crate. Only a suite that writes the process environment stays
//! one, because the environment is shared by every test in a binary.

mod claims_test;
mod commitments_test;
mod compaction_test;
mod facts_test;
mod followups_test;
mod playbooks_test;
mod scope_test;
mod sessions_test;
mod smoke_test;
mod training_plans_test;
