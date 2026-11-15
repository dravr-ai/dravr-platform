// ABOUTME: Shared statements of the user repository — create, update, the reads by id, email and status, the single-column writes
// ABOUTME: Split from users.rs so it stays under the size ceiling; the repository body there runs them

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::users::user_columns;

/// Insert a new account. Tenant membership is not on this row; it lives in
/// `tenant_users`.
pub(crate) const CREATE_USER_SQL: &str = concat!(
    "INSERT INTO users (",
    user_columns!(""),
    ") VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21)"
);

/// Write a whole account onto its row. `email` and `created_at` are not
/// writable here: an email change would race the unique index, and a
/// creation date is not a mutable fact.
pub(crate) const UPDATE_USER_SQL: &str = r"
            UPDATE users SET
                display_name = $2,
                password_hash = $3,
                tier = $4,
                is_active = $5,
                user_status = $6,
                role = $7,
                approved_by = $8,
                approved_at = $9,
                firebase_uid = $10,
                auth_provider = $11,
                analytics_consent = $12,
                analytics_consent_at = $13,
                locale = $14,
                coaching_persona = $15,
                manages_roster = $16,
                timezone = $17,
                theme = $18,
                last_active = CURRENT_TIMESTAMP
            WHERE id = $1
            ";

/// One user, only when they are a member of the tenant.
pub(crate) const GET_USER_IN_TENANT_SQL: &str = concat!(
    "SELECT ",
    user_columns!("u."),
    " FROM users u
            INNER JOIN tenant_users tu ON u.id = tu.user_id AND tu.tenant_id = $2
            WHERE u.id = $1"
);

/// One user by id, without tenant scoping: for system-level callers only.
pub(crate) const GET_USER_BY_ID_SQL: &str =
    concat!("SELECT ", user_columns!(""), " FROM users WHERE id = $1");

/// One user by email, case aside: `lower(email)` is what `idx_users_email_lower` indexes.
pub(crate) const GET_USER_BY_EMAIL_SQL: &str = concat!(
    "SELECT ",
    user_columns!(""),
    " FROM users WHERE lower(email) = lower($1)"
);

/// One user by the Firebase UID they signed in with.
pub(crate) const GET_USER_BY_FIREBASE_UID_SQL: &str = concat!(
    "SELECT ",
    user_columns!(""),
    " FROM users WHERE firebase_uid = $1"
);

/// The oldest admin account, for system agent seeding.
pub(crate) const FIRST_ADMIN_USER_SQL: &str = concat!(
    "SELECT ",
    user_columns!(""),
    " FROM users WHERE role = 'admin' ORDER BY created_at ASC LIMIT 1"
);

/// Every admin account, by email.
pub(crate) const LIST_ADMIN_USERS_SQL: &str = concat!(
    "SELECT ",
    user_columns!(""),
    " FROM users WHERE role = 'admin' ORDER BY email ASC"
);

/// The batch read behind `get_global_many`: one placeholder per id, so the
/// ids bind through the codec on both drivers rather than as an array on one.
pub(crate) fn users_by_ids_sql(count: usize) -> String {
    let placeholders = (1..=count)
        .map(|i| format!("${i}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "SELECT {} FROM users WHERE id IN ({placeholders})",
        user_columns!("")
    )
}

/// Stamp the moment the user was last seen.
pub(crate) const UPDATE_LAST_ACTIVE_SQL: &str =
    "UPDATE users SET last_active = CURRENT_TIMESTAMP WHERE id = $1";

/// How many accounts exist.
pub(crate) const COUNT_USERS_SQL: &str = "SELECT COUNT(*) AS count FROM users";

/// How many accounts were created at or after `$1`.
pub(crate) const COUNT_USERS_CREATED_SINCE_SQL: &str =
    "SELECT COUNT(*) AS count FROM users WHERE created_at >= $1";

/// Every account in a status, newest first. A row whose status was never set
/// counts as active.
pub(crate) const USERS_BY_STATUS_SQL: &str = concat!(
    "SELECT ",
    user_columns!(""),
    " FROM users
            WHERE COALESCE(user_status, 'active') = $1
            ORDER BY created_at DESC"
);

/// Every account in a status that is a member of the tenant, newest first.
pub(crate) const USERS_BY_STATUS_IN_TENANT_SQL: &str = concat!(
    "SELECT ",
    user_columns!("u."),
    " FROM users u
            INNER JOIN tenant_users tu ON u.id = tu.user_id AND tu.tenant_id = $2
            WHERE COALESCE(u.user_status, 'active') = $1
            ORDER BY u.created_at DESC"
);

/// The first page of a status listing under keyset pagination. The status
/// [`ALL_USER_STATUSES`](super::users::ALL_USER_STATUSES) lists every account.
pub(crate) const USERS_BY_STATUS_PAGE_SQL: &str = concat!(
    "SELECT ",
    user_columns!(""),
    " FROM users
            WHERE ($1 = 'all' OR COALESCE(user_status, 'active') = $1)
            ORDER BY created_at DESC, id DESC
            LIMIT $2"
);

/// A later page: every row strictly after the `(created_at, id)` cursor in
/// the listing's order, so an account created meanwhile neither repeats nor
/// hides another. The id compares natively on both drivers: a uuid orders
/// bytewise on Postgres, which is the order its hyphenated text has on `SQLite`.
pub(crate) const USERS_BY_STATUS_AFTER_CURSOR_SQL: &str = concat!(
    "SELECT ",
    user_columns!(""),
    " FROM users
            WHERE ($1 = 'all' OR COALESCE(user_status, 'active') = $1)
              AND (created_at < $2 OR (created_at = $2 AND id < $3))
            ORDER BY created_at DESC, id DESC
            LIMIT $4"
);

/// Approve, suspend or reset an account.
pub(crate) const UPDATE_USER_STATUS_SQL: &str = r"
            UPDATE users
            SET user_status = $1, approved_by = $2, approved_at = $3
            WHERE id = $4
            ";

/// Write the role; the database derives `users.is_admin` from it.
pub(crate) const SET_USER_ADMIN_STATUS_SQL: &str = "UPDATE users SET role = $1 WHERE id = $2";

/// Link the account to a tenant on its own row; membership follows in
/// [`ADD_TENANT_MEMBER_SQL`].
pub(crate) const SET_USER_TENANT_SQL: &str = "UPDATE users SET tenant_id = $1 WHERE id = $2";

/// Add the account as a member of the tenant, once.
pub(crate) const ADD_TENANT_MEMBER_SQL: &str = r"
            INSERT INTO tenant_users (id, tenant_id, user_id, role, invited_at, joined_at)
            VALUES ($1, $2, $3, 'member', $4, $4)
            ON CONFLICT (tenant_id, user_id) DO NOTHING
            ";

/// Replace the password hash.
pub(crate) const UPDATE_USER_PASSWORD_SQL: &str =
    "UPDATE users SET password_hash = $1, last_active = CURRENT_TIMESTAMP WHERE id = $2";

/// Replace the display name.
pub(crate) const UPDATE_USER_DISPLAY_NAME_SQL: &str =
    "UPDATE users SET display_name = $1, last_active = CURRENT_TIMESTAMP WHERE id = $2";

/// Remove the account row; the rows whose foreign keys cascade go with it.
pub(crate) const DELETE_USER_SQL: &str = "DELETE FROM users WHERE id = $1";

/// Move the account to a billing tier.
pub(crate) const SET_USER_TIER_SQL: &str =
    "UPDATE users SET tier = $1, last_active = CURRENT_TIMESTAMP WHERE id = $2";
