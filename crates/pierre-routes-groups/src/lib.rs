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
//! so REST analytics and the chat agent share one all-providers +
//! deduplicated snapshot source. Routes are generic over
//! `C: ToolRuntime + MiddlewareCtx + GroupsCtx` so they can construct
//! OAuth-authenticated fitness providers per member from the same
//! `Arc<C>` that satisfies the group trait bounds. Mounted by the
//! composition root next to [`groups::GroupRoutes::routes`] under the
//! shared `/api/groups` prefix.

#![warn(missing_docs)]

/// `TrainingPeaks` delegated connections: a group's coach links roster
/// athletes to members, who confirm; either side ends a link.
pub mod delegated_connections;

/// Group analytics router (`/stats`, `/report`, `/health`).
///
/// Sits next to [`mod@groups`]; shares the
/// [`pierre_runtime_context::GroupsCtx`] surface and the
/// `/api/groups/*` URL prefix.
pub mod group_analytics;

/// The seam through which the weekly-digest scheduler posts into a group's
/// own bound chat; implemented in `pierre-server` over the messaging adapters.
pub mod group_chat_poster;

/// Background scheduler that sends each group its weekly digest at a fixed
/// local slot — into its bound chat and to its managers — gated by the
/// per-tenant `weekly_digest` tier flag.
pub mod group_digest_scheduler;

/// When a group's weekly digest is due: Monday 08:00 in the group's own zone,
/// caught up later that week in daytime, never at night.
pub mod group_digest_slot;

/// The group body the `/api/groups` routes return, its AI agent and human
/// coach named for the reader.
pub mod group_response;

/// Group coaching endpoints (CRUD, membership, invites, analytics).
pub mod groups;

/// Who may update a group's settings: owners and admins every field, the
/// attached human coach the weekly digest mode only.
mod group_update_access;

/// Push-notification endpoints (device tokens, preferences, feed, scheduling).
pub mod notifications;

pub use delegated_connections::DelegatedConnectionRoutes;
pub use group_chat_poster::GroupChatPoster;
pub use groups::{
    GroupMetadata, GroupRoutes, HealthFlagsResponse, StatsResponse, WeeklyReportResponse,
};
pub use notifications::NotificationRoutes;
