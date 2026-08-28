// ABOUTME: The role `user create --force` writes, given the flag passed and the role already held
// ABOUTME: On the lib target so the rule is testable without standing up the CLI's whole database path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Role resolution for the admin-user upsert.
//!
//! `pierre-cli user create --force` re-asserts an existing account as an admin.
//! Deciding *which* admin role to write is the whole content of this module,
//! and it lives here rather than inline in `commands::user` because the command
//! path needs a database, a master encryption key and a JWT secret before it
//! reaches the decision — so a test of the decision would have to stand all
//! three up to assert four lines of logic.

use pierre_core::permissions::UserRole;

/// The role `pierre-cli user create --force` should write onto an existing account.
///
/// `--super-admin` is a **floor, not an exact value**: it promotes, and its
/// absence must never demote.
///
/// The bug this exists to stop: the flag alone decided the role, so
/// `user create --email <existing-super-admin> --password <p> --force` — the
/// exact form the dev-setup script uses to re-point an account — wrote
/// `role = Admin` over a live super-admin, silently stripping impersonation and
/// every super-admin-gated route. It routed around the guard the repository
/// layer states on `set_admin_status` ("Super-admins cannot be demoted via this
/// method"), and it was unreachable on `PostgreSQL` until carnet#124 was fixed,
/// so the fix for that is what exposed it.
///
/// Demotion has a deliberate path — `set_admin_status`, which refuses it
/// outright — and a password reset is not it.
#[must_use]
pub const fn forced_admin_role(super_admin_requested: bool, existing: UserRole) -> UserRole {
    if super_admin_requested || existing.is_super_admin() {
        UserRole::SuperAdmin
    } else {
        UserRole::Admin
    }
}
