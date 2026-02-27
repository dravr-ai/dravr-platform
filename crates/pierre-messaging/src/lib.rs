// ABOUTME: Messaging provider abstraction for external chat platform integrations
// ABOUTME: Defines the MessagingProvider trait and shared types for Slack, Discord, Teams
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Messaging Layer
//!
//! Provides a pluggable messaging provider abstraction for integrating with external
//! chat platforms (Slack, Discord, Microsoft Teams). Each provider implements the
//! [`MessagingProvider`] trait, enabling bidirectional message bridging between
//! external channels and the Pierre AI chat system.
//!
//! ## Feature flags
//!
//! * **`slack`** — enables the Slack messaging provider implementation.

#![deny(unsafe_code)]

// Re-export pierre-core modules for consistent error/model access within this crate
pub use pierre_core::errors;

/// Core messaging provider trait and shared abstractions
pub mod provider;

/// Shared types for messaging providers
pub mod types;

/// Slack messaging provider (requires `slack` feature)
#[cfg(feature = "slack")]
pub mod slack;

// Re-export key types for convenience
pub use provider::MessagingProvider;
pub use types::{ChannelInfo, IncomingMessage, OutgoingMessage};
