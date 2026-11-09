// ABOUTME: Admin route handler functions, grouped by sub-area
// ABOUTME: Each module is a thin axum wrapper delegating business logic to pierre-services
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Handler functions for [`crate::AdminRoutes`].
//!
//! Each sub-module exposes one or more axum handler functions that the route
//! builders in `crate::routes` wire to URL patterns — `pub` where an
//! integration test drives the handler directly with a hand-built
//! `ValidatedAdminToken`, `pub(crate)` otherwise. The handlers consume
//! [`crate::AdminApiContext`] via `State<Arc<...>>` and delegate business
//! logic to `pierre-services` (tenant admin, memory facts, …).

pub mod admin_rate_limit_override;
/// Which tenants a console read covers, and the tenants a super admin can pick
pub mod admin_scope;
/// Admin endpoints for listing, creating, and revoking tenant API keys.
pub mod api_keys;
/// The admin audit trail, read newest first (carnet#1391).
pub mod audit_trail;
pub mod claim_verdicts;
pub mod coach_access;
pub mod contremaitre_admin;
/// Super-admin deletion ledger: the hash-chained receipts and the import tombstones.
pub mod deletion_ledger;
/// RFC 8628 Device Authorization Grant endpoints backing `pierre-cli auth login`.
pub mod device_auth;
/// Browser approval page for the device flow (gcloud-style super-admin login).
pub mod device_web;
/// The admin Engagement screen: activity tiers, agent leaderboard and published count in one scope.
pub mod engagement;
pub mod feature_flags;
/// Admin endpoints for the Guardian security-policy configuration.
pub mod guardian_config;
pub mod harness_config;
pub mod impersonation;
/// The single-use invitation a pre-approval allow can mail: issue the link,
/// hand it to the notifier, and report what became of it.
mod invitations;
pub mod llm_consumption;
pub mod memory_worker;
/// The new-user cohort: accounts created in a window and how far each got.
pub mod new_users;
/// Super-admin clone of one account's scrape-session provider connection onto another.
pub mod provider_clone;
/// Super-admin termination purge: every row one provider contributed, in every tenant.
pub mod provider_data;
/// The compare-and-set contract a document setting's save answers to.
pub mod setting_revision;
/// Admin endpoints for reading and writing tenant runtime settings.
pub mod settings;
/// Admin endpoints driving first-run / bootstrap setup of a tenant.
pub mod setup;
/// Signup codes: list, mint, revoke (carnet#1128)
pub mod signup_codes;
/// Super-admin endpoints managing the Strava shared-app OAuth credential pool.
pub mod strava_pool;
/// Admin endpoints for issuing and revoking admin / service API tokens.
pub mod tokens;
pub mod types;
/// Admin endpoints that remove a user completely, or disconnect one provider
/// for them, through the provider-disconnect chokepoint.
pub mod user_removal;
/// Admin endpoints for listing tenant users and managing their access.
///
/// Includes the standing pre-approval allow-list
/// (`/admin/pre-approved-emails`) that `pierre-cli user allow / disallow /
/// list-allowed` drives with its device-login token, and the single-use
/// invitation an allow can mail: the invitee chooses their own password on the
/// web app's accept page and the account lands active, `approved_by` attributed.
pub mod users;
/// dravr.ai website endpoints for its Worker: the docs magic link's send and redemption.
pub mod website;
