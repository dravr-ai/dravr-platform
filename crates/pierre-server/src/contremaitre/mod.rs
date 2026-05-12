// ABOUTME: Prompt hot-reload system backed by GCS (default) or GitHub Contents API
// ABOUTME: Provides PromptRegistry, sync engine, and webhook-driven updates
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Contremaitre — Prompt Hot-Reload System
//!
//! Manages system prompts and coach personas authored in the private
//! `dravr-contremaitre` GitHub repository. On startup the server pulls
//! the latest snapshot; a webhook endpoint triggers selective reload on
//! pushes. Admin write-back still commits to the repo via the GitHub
//! Contents API (writes are rare, attribution requires a real PAT).
//!
//! ## Read-path source of truth
//!
//! Reads do **not** go to GitHub at runtime. A GitHub Action in
//! dravr-contremaitre mirrors the asset tree to a Google Cloud Storage
//! bucket on every push to `main`; Pierre reads from that bucket via
//! [`store::gcs::GcsPromptStore`]. This decouples contremaitre from the
//! per-GitHub-user 5000/hr core API rate limit, which is shared with the
//! Copilot LLM provider on the same `jfarcand` account.
//!
//! When `CONTREMAITRE_GCS_BUCKET` is unset (local dev / non-GCP installs),
//! [`store::github::GitHubPromptStore`] takes over and the GitHub
//! Contents API path runs.
//!
//! Compiled-in `include_str!()` constants in `pierre-llm` serve as the
//! ultimate fallback when neither store is reachable.

/// Admin API endpoints for viewing and editing system prompts
pub mod admin;
/// Environment-based configuration for repo, GCS bucket, PAT, and webhook secret
pub mod config;
/// Structured error types mapped to `AppError` via `ErrorCode`
pub mod errors;
/// Hot-reloadable Tier 5.5 evidence registry for the bullshit detector
pub mod evidence_registry;
/// GitHub Contents API client (used for admin writes; reads now go via [`store`])
pub mod github;
/// Manifest parsing and SHA-256 hash computation for change detection
pub mod manifest;
/// Hot-reloadable user-facing messaging strings (channel replies, errors)
pub mod messaging_strings;
/// Hot-reloadable per-event Slack routing rules for `dravr-tronc::notify`
pub mod notify_routing;
/// In-memory prompt registry with compiled-in fallback
pub mod registry;
/// Read-only [`store::PromptStore`] trait + GCS / GitHub backends
pub mod store;
/// Startup and webhook-triggered sync engine
pub mod sync;
/// Hot-reloadable tool description overlays for MCP tool schemas
pub mod tool_descriptions;
/// GitHub webhook handler with HMAC-SHA256 verification
pub mod webhook;

pub use config::ContremaitreConfig;
pub use errors::ContremaitreError;
pub use evidence_registry::EvidenceRegistry;
pub use manifest::Manifest;
pub use messaging_strings::MessagingStringsRegistry;
pub use registry::PromptRegistry;
pub use store::PromptStore;
pub use tool_descriptions::ToolDescriptionRegistry;
