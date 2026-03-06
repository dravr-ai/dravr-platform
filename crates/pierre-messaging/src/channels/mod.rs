// ABOUTME: Feature-gated channel adapter modules for each messaging platform
// ABOUTME: Each channel provides transport (wire protocol) and renderer (message formatting)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// `WhatsApp` Business Cloud API adapter via Meta Graph API
#[cfg(feature = "channel-whatsapp")]
pub mod whatsapp;

/// Meta Messenger Platform adapter
#[cfg(feature = "channel-messenger")]
pub mod messenger;

/// Discord Bot API adapter with Ed25519 signature verification
#[cfg(feature = "channel-discord")]
pub mod discord;

/// Slack Events API adapter with Block Kit rendering
#[cfg(feature = "channel-slack")]
pub mod slack;

/// Telegram Bot API adapter with secret token verification
#[cfg(feature = "channel-telegram")]
pub mod telegram;
