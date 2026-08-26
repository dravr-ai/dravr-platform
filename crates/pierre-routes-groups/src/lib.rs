// ABOUTME: Groups and notifications route group for the Pierre platform
// ABOUTME: Generic over GroupsCtx + MiddlewareCtx so the crate stays decoupled from pierre-server
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Pierre Groups and Notifications Routes
//!
//! Hosts the `/api/groups/*` and `/api/notifications/*` REST endpoints:
//! group CRUD / membership / invites / analytics, device-token
//! registration, notification preferences, the notification feed and
//! scheduled notifications, plus the background weekly-digest scheduler.
//!
//! The route group is generic over [`pierre_runtime_context::GroupsCtx`]
//! (for the notification service, group service, and admin-config reads)
//! and [`pierre_runtime_context::MiddlewareCtx`] (for repository access
//! and the `AuthenticatedUser` extractor); the composition root in
//! `pierre-server` implements both traits on its `ServerContext`.
//!
//! **Group analytics** (`/stats`, `/report`, `/health`) live in
//! [`mod@group_analytics`], which builds member snapshots via the
//! canonical [`pierre_tool_runtime::group_fitness::fetch_member_snapshots`]
//! so REST analytics and the chat coach share one all-providers +
//! deduplicated snapshot source. Routes are generic over
//! `C: ToolRuntime + MiddlewareCtx + GroupsCtx` so they can construct
//! OAuth-authenticated fitness providers per member from the same
//! `Arc<C>` that satisfies the group trait bounds. Mounted by the
//! composition root next to [`groups::GroupRoutes::routes`] under the
//! shared `/api/groups` prefix.

#![warn(missing_docs)]

/// Group analytics router (`/stats`, `/report`, `/health`).
///
/// Sits next to [`mod@groups`]; shares the
/// [`pierre_runtime_context::GroupsCtx`] surface and the
/// `/api/groups/*` URL prefix.
pub mod group_analytics;

/// Background scheduler that pushes group weekly digests on a weekly cadence,
/// gated by the per-tenant `weekly_digest` tier flag.
pub mod group_digest_scheduler;

/// Group coaching endpoints (CRUD, membership, invites, analytics).
pub mod groups;

/// Push-notification endpoints (device tokens, preferences, feed, scheduling).
pub mod notifications;

pub use groups::{
    GroupMetadata, GroupRoutes, HealthFlagsResponse, StatsResponse, WeeklyReportResponse,
};
pub use notifications::NotificationRoutes;
