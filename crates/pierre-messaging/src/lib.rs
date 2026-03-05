// ABOUTME: Multi-channel messaging gateway for Pierre fitness platform
// ABOUTME: Trait-based architecture with transport adapters and response renderers per channel
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![deny(unsafe_code)]

//! # Pierre Messaging Gateway
//!
//! This crate provides a unified messaging interface for routing conversations
//! through external chat platforms (`WhatsApp`, Messenger, Discord, Slack, Telegram).
//!
//! Each channel implements two traits:
//! - [`TransportAdapter`] — webhook parsing, signature verification, raw HTTP send
//! - [`ResponseRenderer`] — format `OutgoingMessage` into channel-specific payloads

/// Channel trait combining transport and rendering
pub mod channel;

/// Channel descriptor metadata (name, type, webhook path, capabilities)
pub mod descriptor;

/// Transport adapter trait for webhook ingress and outbound HTTP
pub mod transport;

/// Response renderer trait for channel-specific message formatting
pub mod renderer;

/// Channel registry mapping `ChannelType` to adapter instances
pub mod registry;

/// Message router: inbound dispatch to Pierre chat, outbound formatting and queueing
pub mod router;

/// Stateless adapter factory for on-demand channel adapter construction
pub mod factory;

/// Outbound retry worker with exponential backoff and dead-letter queue
pub mod retry;

/// Observability helpers for structured tracing spans and metrics
pub mod observability;

/// Per-channel adapter implementations
pub mod channels;

// Re-export primary types for consumers
pub use channel::MessagingChannel;
pub use descriptor::ChannelDescriptor;
pub use registry::ChannelRegistry;
pub use renderer::ResponseRenderer;
pub use transport::TransportAdapter;
