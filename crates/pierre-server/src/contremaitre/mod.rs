// ABOUTME: Prompt hot-reload system backed by a GitHub repository
// ABOUTME: Provides PromptRegistry, GitHub sync, and webhook-driven updates
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Contremaitre — Prompt Hot-Reload System
//!
//! Manages system prompts and coach personas from a private GitHub repository
//! (`dravr-contremaitre`). On startup the server syncs prompts from GitHub;
//! a webhook endpoint receives push events for instant hot-reload. Admin
//! write-back commits changes to the repo via the GitHub Contents API.
//!
//! The compiled-in `include_str!()` constants from `pierre-llm` serve as
//! fallback when GitHub is unreachable.

/// Admin API endpoints for viewing and editing system prompts
pub mod admin;
/// Environment-based configuration for GitHub repo, branch, PAT, and webhook secret
pub mod config;
/// Structured error types mapped to `AppError` via `ErrorCode`
pub mod errors;
/// Hot-reloadable Tier 5.5 evidence registry for the bullshit detector
pub mod evidence_registry;
/// GitHub Contents API client for reading and writing prompt files
pub mod github;
/// Manifest parsing and SHA-256 hash computation for change detection
pub mod manifest;
/// Hot-reloadable user-facing messaging strings (channel replies, errors)
pub mod messaging_strings;
/// In-memory prompt registry with compiled-in fallback
pub mod registry;
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
pub use tool_descriptions::ToolDescriptionRegistry;
