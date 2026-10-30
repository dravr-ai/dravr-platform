// ABOUTME: Integration tests for pierre-sechoir, one module per suite in this single test binary
// ABOUTME: Exercises the public scan API over real directory trees written to a temp dir
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Integration tests for pierre-sechoir, linked as one binary. Each suite is
//! a module here; none writes the process environment.

mod scan;
