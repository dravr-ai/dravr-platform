// ABOUTME: The RFC 8628 device authorization statements: open, read by either code, approve, deny, consume, sweep
// ABOUTME: One SQL text per operation, shared by both backends through the OAuth 2.0 server repository macro
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Delete every device authorization past its expiry: `$1` is now, in epoch
/// seconds. `<=` is the comparison the token endpoint and the approval read
/// an expired row by, so what goes is exactly what neither would honour.
///
/// Nothing else removes a row whose device never polled again (a login
/// started and abandoned, or one denied and never collected), so the store
/// runs this each time it opens a new authorization: the table holds the
/// logins still inside their window, and each abandoned row is deleted
/// once, by the next login to start. `idx_device_authorization_expires_at`
/// serves the range.
pub(crate) const SWEEP_EXPIRED_DEVICE_AUTHORIZATIONS_SQL: &str =
    "DELETE FROM device_authorization WHERE expires_at <= $1";

/// Open an RFC 8628 device authorization, pending until an admin acts.
pub(crate) const CREATE_DEVICE_AUTHORIZATION_SQL: &str = r"
            INSERT INTO device_authorization
                (device_code_hash, user_code, status, approved_by, created_at, expires_at)
            VALUES ($1, $2, $3, $4, $5, $6)
            ";

/// A device authorization by the hash of its device code (the polling side).
pub(crate) const GET_DEVICE_AUTHORIZATION_BY_CODE_HASH_SQL: &str = r"
            SELECT device_code_hash, user_code, status, approved_by, created_at, expires_at
            FROM device_authorization
            WHERE device_code_hash = $1
            ";

/// A device authorization by its user code (the admin side).
pub(crate) const GET_DEVICE_AUTHORIZATION_BY_USER_CODE_SQL: &str = r"
            SELECT device_code_hash, user_code, status, approved_by, created_at, expires_at
            FROM device_authorization
            WHERE user_code = $1
            ";

/// Approve a still-pending authorization; an already-decided one matches nothing.
pub(crate) const APPROVE_DEVICE_AUTHORIZATION_SQL: &str = r"
            UPDATE device_authorization
            SET status = 'approved', approved_by = $2
            WHERE user_code = $1 AND status = 'pending'
            ";

/// Deny a still-pending authorization.
pub(crate) const DENY_DEVICE_AUTHORIZATION_SQL: &str = r"
            UPDATE device_authorization
            SET status = 'denied'
            WHERE user_code = $1 AND status = 'pending'
            ";

/// Consume a device authorization; the token endpoint mints only when a row
/// went, so a duplicate poll can never mint twice.
pub(crate) const DELETE_DEVICE_AUTHORIZATION_SQL: &str =
    "DELETE FROM device_authorization WHERE device_code_hash = $1";
