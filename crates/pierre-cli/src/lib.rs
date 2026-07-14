// ABOUTME: Library surface of pierre-cli — command dispatch, helpers, and the remote-client modules
// ABOUTME: The `pierre-cli` binary (src/main.rs) is a thin clap wrapper over this library; tests exercise it here
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Reusable implementation behind the `pierre-cli` binary.
//!
//! The remote-server client layer (credential cache, HTTP client, device-login
//! flow) lives here — as a library target — so integration tests can exercise
//! it directly (the crate has no `src`-level unit tests by project convention).
//! The command dispatch and clap surface stay binary-private in `src/main.rs`.

pub mod remote;
