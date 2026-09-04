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

/// The activity-sport vocabulary shared with the clients, for server-rendered text.
pub mod activity_sports;
/// Admin operations: user lifecycle, token management, settings, and analytics
pub mod admin_ops;
/// System-wide operator settings — auto-approval and its env shadow
pub mod admin_settings;
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

/// Coach-selection recording — the one emit site for `coach.selected`
pub mod coach_selection;

/// Coach generation from a conversation — the draft behind `/coach create`,
/// plus the per-user coach quota read.
///
pub mod coach_generation;

/// Coach Store browse / search / install, shared by the REST routes and the
/// chat-callable `store` MCP tools.
pub mod coach_store;

/// Coach lifecycle operations: prerequisites, assignments, and generation
pub mod coaches;

/// Commitment sweep: counts due athlete commitments against real activity data
/// and hands the verdict to a reporter for delivery.
pub mod commitment_sweep;

/// Conversation compaction: keeps long conversations under the context window
pub mod conversation_compaction;
/// Forging a fresh chat conversation — the ceremony the messaging self-heal
/// and `/reset` both run.
pub mod conversation_forge;

/// Admin browser over pierre-evals golden fixtures
#[cfg(feature = "tools-verification")]
pub mod eval_harness;

/// Health data sync adapter bridging dravr-enforme store traits to pierre-database
#[cfg(feature = "health-sync")]
pub mod health_sync;

/// Memory extraction: Tier 2 background distillation of user facts from finished turns
pub mod memory_extraction;

/// User-facing memory fact service — list and forget what the coach remembers
/// Deciding whether an extracted fact is new or a restatement of an existing one.
pub mod memory_dedup;
/// Folding an athlete's already-stored duplicate facts into their anchors.
pub mod memory_dedup_backfill;
pub mod memory_facts;

/// OKF bundle rendering — the per-user Dossier projected to markdown for the prompt
pub mod okf;

/// PAR-Q+ pre-participation medical-safety gate (structured Y/N → medical flags)
pub mod parq;

/// « Style de coaching » persona cards rendered from the live persona-contract registry
pub mod personas;

/// The messaging intake walk — profile type, then the PAR-Q+.
///
/// Asked by the platform rather than the coach, so a standardised instrument
/// reaches the athlete verbatim instead of paraphrased.
pub mod intake;

/// Plan-save ramp check — the opening week against the athlete's real load.
pub mod ramp_check;

/// Server lifecycle notify events raised from the binary's startup and
/// SIGTERM paths.
pub mod server_lifecycle;

/// The athlete's recent training load — one source for the calibration
/// baseline and the plan-save ramp check.
pub mod recent_load;

pub mod about_you;
pub mod email_verification;
pub mod link_token;

/// Platform-initiated outbound messaging: send a localized text on every
/// channel a user has linked (`client-messaging` feature).
#[cfg(feature = "client-messaging")]
pub mod messaging_broadcast;

/// Background outbound retry worker for messaging delivery queue
#[cfg(feature = "client-messaging")]
pub mod messaging_outbound;

/// Seed messaging channel configs from environment variables on startup
#[cfg(feature = "client-messaging")]
pub mod messaging_seed;

/// Publish the slash-command catalogue to Telegram's `setMyCommands` so the
/// bot's `/` menu matches what the server dispatches (`client-messaging`).
#[cfg(feature = "client-messaging")]
pub mod telegram_bot_commands;

/// Publish the same catalogue as Messenger's persistent menu — the one
/// always-on menu surface a bot can set for itself (`client-messaging`).
#[cfg(feature = "client-messaging")]
pub mod messenger_persistent_menu;

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

/// Best-effort bridge notification after a successful OAuth connection
mod oauth_bridge_notify;

/// OAuth flow orchestration: state validation, token exchange, credential storage
pub mod oauth_flow;

/// Upstream grant revocation + provider-data purge for the disconnect chokepoint
mod provider_revocation;

/// Post-OAuth redirect URL validation and construction (allowlist, state decoding)
pub mod oauth_redirects;

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

/// The messaging delivery sink for dispatched notifications — the third sink
/// beside persist and Expo push (`client-messaging` feature).
#[cfg(feature = "client-messaging")]
pub mod notification_channel_sink;

/// The persona push-policy gate for dispatched notifications — resolves each
/// recipient's tier floor, digest cadence, and arming flag.
pub mod persona_notification_policy_gate;

/// The one renderer turning a stored notification event plus its parameters
/// into a sentence, in the locale of whoever is reading it.
pub mod notification_text;

/// The dispatch-time localizer: resolves the recipient's stored locale and
/// renders the event before the push and the linked channels go out.
pub mod notification_localizer;

/// Weekly digest scheduler that rolls persona-gated notifications into one
/// localized push per user.
pub mod notification_digest_scheduler;

/// The usage-cap policy shared by chat turns and direct `/mcp` tool
/// calls — one ladder, one tier resolution, one bypass rule.
pub mod quota_policy;

/// Provider data refresh service: freshness checks, on-chat triggers, manual sync.
///
/// Owns `RefreshService`, `start_scheduled_sync`,
/// `compute_smart_interval`, and `SyncMetrics`. Push notifications go through
/// the abstract `SyncNotifier` trait so the service is decoupled from the
/// concrete pierre-server `SseManager`.
pub mod provider_refresh;

/// The language an athlete reads, resolved once for every surface
pub mod locale;

/// One tick loop for every background worker
pub mod periodic;

/// Recipe import/export and markdown conversion
pub mod recipes;

/// Short-link table hygiene: periodic sweep of expired reconnect/connect links
pub mod mcp_task_sweeper;
pub mod short_link_sweeper;

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

/// Standing per-email pre-approval allow-list.
pub mod pre_approval;

/// Lazy weather backfill — fills missing ambient temperature on activities
/// from start coordinates + start time via dravr-meteo.
pub mod weather_backfill;

/// Push an athlete's active training plan to a provider calendar and reconcile the ledger.
pub mod plan_calendar_push;
