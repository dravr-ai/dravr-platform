// ABOUTME: Error-related constants including codes and messages
// ABOUTME: Organizes error handling constants by protocol and domain
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.io

//! Error constants module

/// Error codes and messages
pub mod codes;

// Re-export all error constants

/// Re-export all error code constants
pub use codes::*;
