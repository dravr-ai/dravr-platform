// ABOUTME: `user create --force` must never demote a super-admin — the flag promotes, its absence does not revoke
// ABOUTME: Pins the rule the repository layer states on set_admin_status, which the --force update path walked around
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_cli::admin_role::forced_admin_role;
use pierre_core::permissions::UserRole;

/// The regression. `pierre-cli user create --email <existing-super-admin>
/// --password <p> --force` — the form the dev-setup script uses to re-point an
/// account — resolved the role from the `--super-admin` flag alone, so omitting
/// the flag wrote `role = Admin` over a live super-admin.
///
/// Asserted on the exact role rather than "is admin or higher": the account
/// stays `is_admin` either way, so a predicate that accepts `Admin` passes
/// while the operator has silently lost impersonation, admin-role management,
/// system config and audit-log access.
#[test]
fn force_update_without_the_flag_leaves_a_super_admin_super() {
    assert_eq!(
        forced_admin_role(false, UserRole::SuperAdmin),
        UserRole::SuperAdmin,
        "a password reset is not a demotion — set_admin_status refuses this outright"
    );
}

/// The flag still promotes. This is the half that always worked, pinned so a
/// fix for the case above cannot quietly cost the command its actual job.
#[test]
fn the_flag_promotes_an_ordinary_admin() {
    assert_eq!(
        forced_admin_role(true, UserRole::Admin),
        UserRole::SuperAdmin
    );
    assert_eq!(
        forced_admin_role(true, UserRole::User),
        UserRole::SuperAdmin
    );
    assert_eq!(
        forced_admin_role(true, UserRole::SuperAdmin),
        UserRole::SuperAdmin,
        "re-asserting the flag on someone who already has it is a no-op, not an error"
    );
}

/// Without the flag, an account that is not already a super-admin becomes a
/// plain admin — which is the whole point of `user create --force`, and the
/// behaviour a naive "always preserve the existing role" fix would break.
#[test]
fn without_the_flag_a_non_super_admin_becomes_a_plain_admin() {
    assert_eq!(forced_admin_role(false, UserRole::User), UserRole::Admin);
    assert_eq!(forced_admin_role(false, UserRole::Admin), UserRole::Admin);
}

/// The resolved role is what a caller writes, so it must be exhaustive over the
/// enum: every `(flag, existing)` pair resolves to a role that is admin or
/// higher. `user create` grants admin unconditionally; there is no input to
/// this function that should ever produce `User`.
#[test]
fn every_outcome_is_admin_or_higher() {
    for flag in [true, false] {
        for existing in [UserRole::User, UserRole::Admin, UserRole::SuperAdmin] {
            let resolved = forced_admin_role(flag, existing);
            assert!(
                resolved.is_admin_or_higher(),
                "user create grants admin; ({flag}, {existing}) resolved to {resolved}"
            );
        }
    }
}
