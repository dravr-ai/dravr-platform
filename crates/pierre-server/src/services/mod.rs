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

/// Tracks the detached turns a webhook starts, so shutdown can drain them.
pub mod turn_lifecycle;

/// Chat verdict service: maps ClaimVerdict rows into chat-facing wire shapes
pub mod chat_verdicts;
/// Client for the photograveur press service (Scene -> PNG).
pub mod photograveur_client;

/// User-facing memory fact service: list and forget what the coach remembers (Sprint C5)
pub mod memory_facts;

/// User-facing persona cards handler: « Style de coaching » from the live contract registry
pub mod personas;

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

/// Spawns the coaching background workers (outcome evaluator, archetype
/// aggregation, commitment sweep).
pub mod coaching_workers;

/// Turns a commitment sweep's refresh request into background provider
/// fetches, so a quiet athlete's window still gets counted.
pub mod commitment_refresher;

/// Delivers a swept commitment verdict back to the athlete, applying the
/// per-channel proactive-messaging policy.
#[cfg(feature = "client-messaging")]
pub mod commitment_reporter;

/// AuthService-backed credential refresher installed on the health-sync
/// storage post-Arc (the refresh path needs the composition-root runtime).
#[cfg(feature = "health-sync")]
pub mod health_sync_refresher;

/// Endurance Phase 2 training-history compute service — fetches activities + physiology,
/// runs `pierre_fitness_compute::training_history_compute`, persists rows.
pub mod training_history_compute;
