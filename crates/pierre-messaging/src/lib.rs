// ABOUTME: Re-export facade for dravr-canot multi-channel messaging library
// ABOUTME: All types, traits, and channel adapters are now canonical in dravr-canot
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![deny(unsafe_code)]

//! # Pierre Messaging Gateway
//!
//! This crate re-exports the [`dravr-canot`](https://crates.io/crates/dravr-canot)
//! messaging library. All types, traits, and channel adapters live in dravr-canot;
//! this crate exists solely for workspace path compatibility.

// Re-export all root-level items (traits, types)
pub use dravr_canot::*;

// Re-export modules so `pierre_messaging::channel::MessagingChannel` etc. resolve
pub use dravr_canot::channel;
pub use dravr_canot::channels;
pub use dravr_canot::config;
pub use dravr_canot::descriptor;
pub use dravr_canot::error;
pub use dravr_canot::factory;
pub use dravr_canot::http_client;
pub use dravr_canot::meta_signature;
pub use dravr_canot::models;
pub use dravr_canot::observability;
pub use dravr_canot::registry;
pub use dravr_canot::renderer;
pub use dravr_canot::retry;
pub use dravr_canot::transport;
pub use dravr_canot::turn;

/// Slash command infrastructure for messaging platforms
pub use dravr_canot::commands;

/// AG-UI SSE consumer — deserializes the server's event stream into
/// typed `AgUiEvent`s for channel-side status rendering.
#[cfg(feature = "agui")]
pub use dravr_canot::agui_consumer;

/// AG-UI → status-text mapping plus the `StatusAdapter` trait each
/// channel adapter implements.
#[cfg(feature = "agui")]
pub use dravr_canot::agui_status;
