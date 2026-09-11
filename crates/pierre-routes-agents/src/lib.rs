// ABOUTME: Coaches/roster/store REST routes — agent CRUD, version history, store browse, install, roster
// ABOUTME: Generic over AgentsCtx + MiddlewareCtx so the crate stays decoupled from pierre-server

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Coaches Routes
//!
//! Hosts three related REST surfaces:
//!
//! - `/api/agents/{...}` + `/api/admin/agents/{...}` — user-facing and
//!   admin-only agent CRUD, import/export (markdown + URL), import preview,
//!   LLM-driven generation from a conversation, favorites, usage tracking,
//!   hide/show, fork, version history, version diff, version revert.
//! - `/api/store/{...}` + `/api/admin/store/{...}` — agent store browse,
//!   search, categories, install/uninstall, and admin moderation
//!   (review queue, approve/reject/unpublish, store stats).
//! - `/api/roster/{...}` — agent-athlete assignment listing and management
//!   gated by `users.manages_roster=true` OR `users.is_admin=true`.
//!
//! The route groups are generic over [`pierre_runtime_context::AgentsCtx`]
//! (for repository access, prompt registry, admin config, notification
//! dispatch, and LLM provider handles) and
//! [`pierre_runtime_context::MiddlewareCtx`] (for the `AuthenticatedUser`
//! extractor); the composition root in `pierre-server` implements both
//! traits on its `ServerContext`.

#![warn(missing_docs)]

/// User and admin agent CRUD, import/export, version history.
pub mod agents;

/// Agent-athlete roster assignment management.
pub mod roster;

/// Agent store browse, search, install, uninstall, listings.
pub mod store;

pub use agents::{build_agents_admin_router, build_agents_router};
pub use roster::build_roster_router;
pub use store::build_store_router;
