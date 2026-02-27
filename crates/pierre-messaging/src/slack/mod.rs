// ABOUTME: Slack messaging provider module entry point
// ABOUTME: Re-exports SlackProvider client, types, and signature verification utilities
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Slack Messaging Provider
//!
//! Implements the [`MessagingProvider`](crate::MessagingProvider) trait for Slack
//! using the Slack Bot Token API. Supports sending messages via `chat.postMessage`
//! and verifying incoming Events API webhooks via HMAC-SHA256 signatures.

/// Slack API client implementing the MessagingProvider trait
pub mod client;

/// Slack webhook request signature verification
pub mod signature;

/// Slack-specific event and API types
pub mod types;

pub use client::SlackProvider;
