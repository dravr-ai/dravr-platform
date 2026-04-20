// ABOUTME: Domain service layer for business logic extracted from route handlers
// ABOUTME: Provides protocol-agnostic services reusable across REST, MCP, and A2A
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Domain service layer
//!
//! This module contains protocol-agnostic business logic extracted from route handlers.
//! Services are designed to be reusable across REST, MCP, and A2A protocols, ensuring
//! consistent business rules regardless of the entry point.

/// Coach lifecycle operations: prerequisites, assignments, and generation
pub mod coaches;

/// Canonical factory for `ChatProvider` instances — keeps provider
/// construction out of the routes layer so services can depend on it.
pub mod chat_provider_factory;

/// Coach markdown import: URL fetching, security validation, warnings, definition conversion
pub mod coach_import;

/// Authentication service: registration, login, password management, token refresh
pub mod auth;

/// OAuth flow orchestration: state validation, token exchange, credential storage
pub mod oauth_flow;

/// Recipe import/export and markdown conversion
pub mod recipes;

/// Tenant administration: slug validation, tenant creation, user provisioning
pub mod tenant_admin;

/// Chat orchestration: conversation creation, message persistence, LLM dispatch coordination
pub mod chat_orchestration;

/// Detects LLM-provider CLI error text that leaked into assistant replies
pub mod provider_error_filter;

/// Unified chat pipeline: single orchestrator for every chat turn.
pub mod chat_pipeline;

/// Chat verdict service: maps Tier 5.5 ClaimVerdict rows into chat-facing wire shapes
pub mod chat_verdicts;

/// User-facing memory fact service: list and forget what the coach remembers (Sprint C5)
pub mod memory_facts;

/// Phase C Sprint C9: prompt exfiltration defense — fingerprint + reply scan for verbatim leaks
pub mod prompt_leak;

/// Phase D Sprint C13: myth-busting summary over Tier 5.5 verdicts (top recurring claims, coaches, categories)
pub mod myth_busting;

/// Phase D Sprint C14: per-coach content grading derived from Tier 5.5 verdict history
pub mod coach_grading;

/// Phase B Sprint C16: admin browser over pierre-evals golden fixtures
#[cfg(feature = "tools-verification")]
pub mod eval_harness;

/// Phase D Sprint C17: ClaimVerdict backfill over historical chat_messages
#[cfg(feature = "tools-verification")]
pub mod claim_verdict_backfill;

/// Claim verification: Tier 5.5 bullshit detector pipeline + evidence corpus singleton
#[cfg(feature = "tools-verification")]
pub mod claim_verification;

/// Conversation compaction: keeps long conversations under the context window
pub mod conversation_compaction;

/// Memory extraction: Tier 2 background distillation of user facts from finished turns
pub mod memory_extraction;

/// Memory recall: Tier 2 retrieval of stored user facts for prompt injection
pub mod memory_recall;

/// Social insights: friend-request validation, user search enrichment, insight adaptation
pub mod social_insights;

/// Usage counter service: quota enforcement with burst zones and warning thresholds
pub mod usage_counter;

/// Background task for periodic pruning of old usage counter records
pub mod usage_pruning;

/// Background outbound retry worker for messaging delivery queue
#[cfg(feature = "client-messaging")]
pub mod messaging_outbound;

/// Seed messaging channel configs from environment variables on startup
#[cfg(feature = "client-messaging")]
pub mod messaging_seed;

/// Discord Gateway WebSocket client — bridges real-time messages to the webhook pipeline
#[cfg(feature = "client-messaging")]
pub mod discord_gateway;

/// Slack operations notifier for deploy and user lifecycle events
pub mod slack_ops_notifier;

/// Product analytics (`PostHog`) for messaging funnel, tool usage, and command tracking
pub mod analytics;

/// Messaging ingress: OTP flow, channel linking, session resolution, slash command dispatch
#[cfg(feature = "client-messaging")]
pub mod messaging_ingress;

// Outer doc intentionally omitted — `messaging_status_bridge.rs`'s
// inner `//!` header is authoritative. When both an outer `///` on
// the mod declaration and an inner `//!` in the module file exist,
// rustdoc concatenates them into one virtual doc block whose first
// paragraph trips `clippy::too_long_first_doc_paragraph`.
#[cfg(feature = "client-messaging")]
pub mod messaging_status_bridge;

/// Slash command handlers for messaging platforms
#[cfg(feature = "client-messaging")]
pub mod commands;

/// Batch fitness snapshot fetcher for group coaching context
#[cfg(feature = "tools-groups")]
pub mod group_fitness;

/// Tool execution strategies for multi-turn LLM chat (API, headless, CLI modes)
#[cfg(feature = "client-chat")]
pub mod tool_execution;

/// Admin operations: user lifecycle, token management, settings, and analytics
pub mod admin_ops;

/// Health data sync adapter bridging dravr-enforme store traits to pierre-database
#[cfg(feature = "health-sync")]
pub mod health_sync;

/// Provider data refresh service: freshness checks, on-chat triggers, manual sync
pub mod provider_refresh;

/// App-wide rate limiter for external fitness provider APIs
pub mod provider_rate_limiter;
