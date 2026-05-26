// ABOUTME: Coaches/roster/store REST routes — coach CRUD, version history, store browse, install, roster
// ABOUTME: Generic over CoachesCtx + MiddlewareCtx so the crate stays decoupled from pierre-server

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Coaches Routes
//!
//! Hosts three related REST surfaces:
//!
//! - `/api/coaches/{...}` + `/api/admin/coaches/{...}` — user-facing and
//!   admin-only coach CRUD, import/export (markdown + URL), import preview,
//!   LLM-driven generation from a conversation, favorites, usage tracking,
//!   hide/show, fork, version history, version diff, version revert.
//! - `/api/store/{...}` + `/api/admin/store/{...}` — coach store browse,
//!   search, categories, install/uninstall, and admin moderation
//!   (review queue, approve/reject/unpublish, store stats).
//! - `/api/roster/{...}` — coach-athlete assignment listing and management
//!   gated by `users.manages_roster=true` OR `users.is_admin=true`.
//!
//! The route groups are generic over [`pierre_runtime_context::CoachesCtx`]
//! (for repository access, prompt registry, admin config, notification
//! dispatch, and LLM provider handles) and
//! [`pierre_runtime_context::MiddlewareCtx`] (for the `AuthenticatedUser`
//! extractor); the composition root in `pierre-server` implements both
//! traits on its `ServerContext`.

#![warn(missing_docs)]

/// User and admin coach CRUD, import/export, version history.
pub mod coaches;

/// Coach-athlete roster assignment management.
pub mod roster;

/// Coach store browse, search, install, uninstall, listings.
pub mod store;

pub use coaches::{build_coaches_admin_router, build_coaches_router};
pub use roster::build_roster_router;
pub use store::build_store_router;
