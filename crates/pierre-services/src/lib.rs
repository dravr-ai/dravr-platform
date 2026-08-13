// ABOUTME: Domain service layer for the Pierre platform — leaf services moved from pierre-server
// ABOUTME: Protocol-agnostic business logic reusable across REST, MCP, A2A transports
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Services
//!
//! Wave 1 extraction of leaf domain services from `pierre-server`. Every
//! module here is protocol-agnostic and free of any dependency on the
//! pierre-server composition root — the only inputs are `pierre-core`,
//! sibling crates from this workspace, and external dravr-* crates.
//!
//! New services should land here (or in a future deeper crate) rather
//! than in `pierre-server`. The thin module hub at
//! `pierre_mcp_server::services` re-exports each module so existing
//! call-sites keep their `crate::services::X` paths during the
//! transition.

#![warn(missing_docs)]

/// Admin operations: user lifecycle, token management, settings, and analytics
pub mod admin_ops;
/// Advice capture: turn a coach recommendation into a PendingAdvice (playbook memory)
pub mod advice_capture;
/// Archetype aggregation: roll per-user playbooks into k-anonymous cold-start priors
pub mod archetype_aggregation;

/// Authentication service: registration, login, password management, token refresh
pub mod auth;

/// Product analytics (`PostHog`) for messaging funnel, tool usage, and command tracking
pub mod analytics;

/// Athlete physiology snapshot builder for the personalized-physiology layer.
#[cfg(feature = "tools-verification")]
pub mod athlete_snapshot;

/// Claim verification: bullshit detector pipeline + evidence corpus singleton
#[cfg(feature = "tools-verification")]
pub mod claim_verification;

/// ClaimVerdict backfill over historical chat_messages
#[cfg(feature = "tools-verification")]
pub mod claim_verdict_backfill;

/// Chat verdict service: maps ClaimVerdict rows into chat-facing wire shapes
pub mod chat_verdicts;

/// Chat provider factory.
///
/// Construction + lifecycle for `ChatProvider` instances (env-based
/// singleton, tenant-credential build, periodic health probe). Pure
/// pierre-llm wiring, free of `ServerContext`.
pub mod chat_provider_factory;

/// Per-tenant chat-provider resolution from stored BYO LLM credentials, with a
/// short TTL cache, so production chat uses a tenant's own key.
pub mod tenant_chat_provider;

/// Chat stream event surface: token-level streaming primitives shared by
/// chat pipeline channel adapters and tool-loop strategies that support
/// progressive streaming.
pub mod chat_stream;

/// Coach followup scheduler — periodically dispatches notifications when
/// pending followups become overdue and marks them delivered.
pub mod coach_followup_scheduler;

/// Per-coach content grading derived from claim verdict history
pub mod coach_grading;

/// Coach markdown import: URL fetching, security validation, warnings, definition conversion
pub mod coach_import;

/// Coach lifecycle operations: prerequisites, assignments, and generation
pub mod coaches;

/// Conversation compaction: keeps long conversations under the context window
pub mod conversation_compaction;

/// Bridge adapter so `pierre_llm::InstrumentedEmbeddingProvider` can persist
/// every embedding call into `embedding_usage` via the shared repository.
pub mod embedding_sink;

/// Admin browser over pierre-evals golden fixtures
#[cfg(feature = "tools-verification")]
pub mod eval_harness;

/// Health data sync adapter bridging dravr-enforme store traits to pierre-database
#[cfg(feature = "health-sync")]
pub mod health_sync;

/// Memory extraction: Tier 2 background distillation of user facts from finished turns
pub mod memory_extraction;

/// User-facing memory fact service — list and forget what the coach remembers
pub mod memory_facts;

/// OKF bundle rendering — the per-user Dossier projected to markdown for the prompt
pub mod okf;

/// PAR-Q+ pre-participation medical-safety gate (structured Y/N → medical flags)
pub mod parq;

/// Plan-save ramp check — the opening week against the athlete's real load.
pub mod ramp_check;

/// Server lifecycle notify events raised from the binary's startup and
/// SIGTERM paths.
pub mod server_lifecycle;

/// The athlete's recent training load — one source for the calibration
/// baseline and the plan-save ramp check.
pub mod recent_load;

pub mod password_reset;

/// Background outbound retry worker for messaging delivery queue
#[cfg(feature = "client-messaging")]
pub mod messaging_outbound;

/// Seed messaging channel configs from environment variables on startup
#[cfg(feature = "client-messaging")]
pub mod messaging_seed;

/// Channel-group binding for the messaging ingress path — resolves or
/// auto-creates a `coaching_groups` row for a non-DM chat
#[cfg(feature = "client-messaging")]
pub mod messaging_group_bind;

// Outer doc intentionally omitted — `messaging_status_bridge.rs`'s
// inner `//!` header is authoritative. When both an outer `///` on
// the mod declaration and an inner `//!` in the module file exist,
// rustdoc concatenates them into one virtual doc block whose first
// paragraph trips `clippy::too_long_first_doc_paragraph`.
#[cfg(feature = "client-messaging")]
pub mod messaging_status_bridge;

/// Extension trait for turning `AppError` into a channel-safe reply
#[cfg(feature = "client-messaging")]
pub mod channel_error_reply;

/// Myth-busting summary over claim verdicts (top recurring claims, coaches, categories)
pub mod myth_busting;

/// OAuth flow orchestration: state validation, token exchange, credential storage
pub mod oauth_flow;

/// Onboarding gate: requires at least one connected fitness provider before
/// the user can reach chat/coach/MCP tools.
pub mod onboarding_gate;
/// Outcome evaluator: label due advice from real data + reinforce playbooks
pub mod outcome_evaluator;
/// Render a user's proven coaching playbooks into a system-prompt block
pub mod playbook_render;
/// Training-plan prompt-block renderer.
pub mod training_plan_render;

/// Startup hook that loads `cat_llm_pricing` rows from
/// `admin_config_overrides` into the process-wide pricing registry.
pub mod pricing_loader;

/// Prompt exfiltration defense — fingerprint + reply scan for verbatim leaks
pub mod prompt_leak;

/// Detects LLM-provider CLI error text that leaked into assistant replies
pub mod provider_error_filter;

/// App-wide rate limiter for external fitness provider APIs
pub mod provider_rate_limiter;

/// Provider data refresh service: freshness checks, on-chat triggers, manual sync.
///
/// Owns `RefreshService`, `start_scheduled_sync`,
/// `compute_smart_interval`, and `SyncMetrics`. Push notifications go through
/// the abstract `SyncNotifier` trait so the service is decoupled from the
/// concrete pierre-server `SseManager`.
pub mod provider_refresh;

/// Recipe import/export and markdown conversion
pub mod recipes;

/// Short-link table hygiene: periodic sweep of expired reconnect/connect links
pub mod short_link_sweeper;

/// Social insights: friend-request validation, user search enrichment, insight adaptation
pub mod social_insights;

/// Tenant administration: slug validation, tenant creation, user provisioning
pub mod tenant_admin;

/// Usage counter service: quota enforcement with burst zones and warning thresholds
pub mod usage_counter;

/// Background task for periodic pruning of old usage counter records
pub mod usage_pruning;

/// Account-status authorization gate shared by HTTP middleware and messaging ingress.
pub mod user_status_gate;

/// User-approval notification seam (email + linked-channel messages).
pub mod user_approval;

/// Lazy weather backfill — fills missing ambient temperature on activities
/// from start coordinates + start time via dravr-meteo.
pub mod weather_backfill;

/// Endurance Phase 5 workout library — loads compiled-in cornerstone TOML templates.
pub mod workout_library;
