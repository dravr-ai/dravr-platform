// ABOUTME: Admin route handler functions, grouped by sub-area
// ABOUTME: Each module is a thin axum wrapper delegating business logic to pierre-services
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Handler functions for [`crate::AdminRoutes`].
//!
//! Each sub-module exposes one or more `pub(crate)` axum handler functions
//! that the route builders in `crate::routes` wire to URL patterns. The
//! handlers consume [`crate::AdminApiContext`] via `State<Arc<...>>` and
//! delegate business logic to `pierre-services` (myth-busting, coach
//! grading, tenant admin, eval harness, …).

pub mod admin_rate_limit_override;
/// Admin endpoints for listing, creating, and revoking tenant API keys.
pub mod api_keys;
pub mod claim_verdicts;
pub mod coach_followups;
pub mod coach_grading;
pub mod coach_notes;
pub mod contremaitre_admin;
/// RFC 8628 Device Authorization Grant endpoints backing `pierre-cli auth login`.
pub mod device_auth;
/// Browser approval page for the device flow (gcloud-style super-admin login).
pub mod device_web;
#[cfg(feature = "tools-verification")]
pub mod eval_harness;
pub mod feature_flags;
/// Admin endpoints for the Guardian security-policy configuration.
pub mod guardian_config;
pub mod harness_config;
pub mod impersonation;
pub mod llm_consumption;
pub mod memory_worker;
pub mod myth_busting;
/// Admin endpoints for reading and writing tenant runtime settings.
pub mod settings;
/// Admin endpoints driving first-run / bootstrap setup of a tenant.
pub mod setup;
/// Super-admin endpoints managing the Strava shared-app OAuth credential pool.
pub mod strava_pool;
/// Admin endpoints for issuing and revoking admin / service API tokens.
pub mod tokens;
pub mod types;
/// Admin endpoints for listing tenant users and managing their access.
pub mod users;
