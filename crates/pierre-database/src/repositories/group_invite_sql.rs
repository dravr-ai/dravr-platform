// ABOUTME: Shared statements for group invites — create, look up by code or id, claim an approving invite's use, list
// ABOUTME: Split from coaching_groups.rs so it stays under the size ceiling; the repository body there runs them

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// The columns every invite read decodes.
macro_rules! invite_columns {
    () => {
        "id, group_id, tenant_id, code, kind, created_by, expires_at, max_uses,
              use_count, is_active, approves_signup, created_at"
    };
}

// ============================================================================
// group_invites
// ============================================================================

/// One invite, unused and active from the start.
pub const INSERT_INVITE_SQL: &str = r"INSERT INTO group_invites
              (id, group_id, tenant_id, code, kind, created_by, expires_at, max_uses, use_count, is_active, approves_signup, created_at)
              VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 0, TRUE, $9, $10)";

/// The invite just created, read back under its tenant.
pub const GET_INVITE_SQL: &str = concat!(
    "SELECT ",
    invite_columns!(),
    " FROM group_invites
              WHERE id = $1 AND tenant_id = $2"
);

/// An active invite by code. Cross-tenant: codes are globally unique for
/// the join flow.
pub const GET_INVITE_BY_CODE_SQL: &str = concat!(
    "SELECT ",
    invite_columns!(),
    " FROM group_invites
              WHERE code = $1 AND is_active = TRUE"
);

/// Count one more redemption of an invite, only while it is under its use
/// limit: the check and the count are one statement, so two redemptions of
/// the last use cannot both pass.
pub const INCREMENT_INVITE_USE_COUNT_SQL: &str =
    "UPDATE group_invites SET use_count = use_count + 1
              WHERE id = $1 AND (max_uses IS NULL OR use_count < max_uses)";

/// Claim one use of a coach's approving invite by code, in one statement:
/// active, unexpired at `$2` and under its limit, or nothing.
pub const CLAIM_SIGNUP_INVITE_SQL: &str = concat!(
    "UPDATE group_invites SET use_count = use_count + 1
              WHERE code = $1 AND approves_signup = TRUE AND is_active = TRUE
                AND (expires_at IS NULL OR expires_at > $2)
                AND (max_uses IS NULL OR use_count < max_uses)
              RETURNING ",
    invite_columns!()
);

/// One invite by id, cross-tenant: activation follows a redemption to the
/// group it names.
pub const GET_INVITE_BY_ID_SQL: &str = concat!(
    "SELECT ",
    invite_columns!(),
    " FROM group_invites WHERE id = $1"
);

/// Every coach's approving invite, newest first, for the admin listing.
pub const LIST_SIGNUP_INVITES_SQL: &str = concat!(
    "SELECT ",
    invite_columns!(),
    " FROM group_invites WHERE approves_signup = TRUE ORDER BY created_at DESC"
);

/// Retire one invite, scoped by group so an admin cannot retire another
/// group's invite.
pub const DEACTIVATE_INVITE_SQL: &str =
    "UPDATE group_invites SET is_active = FALSE WHERE id = $1 AND group_id = $2";

/// Every invite of a group, newest first. No tenant filter: invites belong
/// to the group, and admins view cross-tenant.
pub const LIST_INVITES_SQL: &str = concat!(
    "SELECT ",
    invite_columns!(),
    " FROM group_invites
              WHERE group_id = $1
              ORDER BY created_at DESC"
);
