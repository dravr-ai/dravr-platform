// ABOUTME: pierre-server-local services that have not been extracted yet
// ABOUTME: Most domain services live in pierre-services, pierre-commands, pierre-chat-pipeline, etc.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! pierre-server-local services
//!
//! Wave 1 leaf services live in the `pierre-services` workspace crate.
//! Chat pipeline lives in `pierre-chat-pipeline`. Slash command parsing
//! lives in `pierre-commands`. Tool execution + group fitness live in
//! `pierre-tool-runtime`. The modules here are the ones that haven't
//! been extracted because they still touch pierre-server-internal
//! resources (Discord/Slack gateways, messaging ingress, endurance
//! training history compute, chat verdict materialization).

/// Chat verdict service: maps ClaimVerdict rows into chat-facing wire shapes
pub mod chat_verdicts;

/// User-facing memory fact service: list and forget what the coach remembers (Sprint C5)
pub mod memory_facts;

/// Discord Gateway WebSocket client — bridges real-time messages to the webhook pipeline
#[cfg(feature = "client-messaging")]
pub mod discord_gateway;

/// Slack Socket Mode WebSocket client — bridges real-time Slack events to the webhook pipeline
#[cfg(feature = "client-messaging")]
pub mod slack_socket;

/// Messaging ingress: OTP flow, channel linking, session resolution, slash command dispatch
#[cfg(feature = "client-messaging")]
pub mod messaging_ingress;

/// Account-approved notifier: email + localized message on each linked channel
#[cfg(feature = "client-messaging")]
pub(crate) mod user_approval_notifier;

/// Backfill-completion notifier: pushes a "your history is ready" notice back to
/// the channel that triggered a historical activity backfill.
#[cfg(feature = "client-messaging")]
pub mod backfill_notifier;

/// Outbound channel-message constructors shared by the proactive senders
/// (backfill/approval notifiers) and the messaging-ingress reply paths.
#[cfg(feature = "client-messaging")]
pub(crate) mod outgoing;

/// Endurance Phase 2 training-history compute service — fetches activities + physiology,
/// runs `pierre_fitness_compute::training_history_compute`, persists rows.
pub mod training_history_compute;
