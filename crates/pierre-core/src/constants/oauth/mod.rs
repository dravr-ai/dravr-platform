// ABOUTME: OAuth-related constants and configuration values
// ABOUTME: Includes provider identifiers and OAuth configuration defaults
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! OAuth constants module

/// What derived content owes the AI consents of the data it was drawn from
pub mod derived_consent;
/// OAuth provider constants
pub mod providers;

// Re-export for easier access

/// Re-export the derived-content consent rules
pub use derived_consent::*;
/// Re-export all OAuth provider constants
pub use providers::*;
