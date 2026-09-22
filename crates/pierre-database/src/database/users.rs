// ABOUTME: SQLite-backed UserRepository and ProfileRepository, emitted from the shared bodies in repositories/users.rs
// ABOUTME: and repositories/user_profiles.rs; uuid columns are TEXT here, and a duplicate reads off SQLite's result codes
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    CoachingPersona, TenantId, User, UserDeletion, UserReference, UserStatus, UserTier,
};
use pierre_core::pagination::{Cursor, CursorPage, PaginationParams};
use pierre_core::permissions::UserRole;
use serde_json::Value;
use sqlx::error::DatabaseError;
use sqlx::sqlite::SqliteRow;
use sqlx::Row;
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

use super::user_preferences as preferences;
use crate::backends::shared::enums::user_status_to_str;
use crate::backends::shared::transactions::TransactionGuard;
use crate::database::Database;
use crate::repositories::user_profiles::{
    apply_progress_fields, impl_profile_repository, CREATE_GOAL_SQL, GET_USER_CONFIGURATION_SQL,
    GET_USER_GOALS_SQL, GET_USER_GOAL_SQL, GET_USER_PROFILE_SQL, SAVE_USER_CONFIGURATION_SQL,
    UPDATE_USER_GOAL_SQL, UPSERT_USER_PROFILE_SQL,
};
use crate::repositories::user_references::{
    delete_user_completely, delete_user_error, user_reference_from_row, DELETION_BLOCKERS_SQL,
    SQLITE_USER_PURGE,
};
use crate::repositories::users::{
    impl_user_repository, user_from_row, user_status_filter, users_by_ids_sql,
    ADD_TENANT_MEMBER_SQL, COUNT_USERS_SQL, CREATE_USER_SQL, DELETE_USER_SQL, FIRST_ADMIN_USER_SQL,
    GET_USER_BY_EMAIL_SQL, GET_USER_BY_FIREBASE_UID_SQL, GET_USER_BY_ID_SQL,
    GET_USER_IN_TENANT_SQL, LIST_ADMIN_USERS_SQL, SET_USER_ADMIN_STATUS_SQL, SET_USER_TENANT_SQL,
    SET_USER_TIER_SQL, UPDATE_LAST_ACTIVE_SQL, UPDATE_USER_DISPLAY_NAME_SQL,
    UPDATE_USER_PASSWORD_SQL, UPDATE_USER_SQL, UPDATE_USER_STATUS_SQL,
    USERS_BY_STATUS_AFTER_CURSOR_SQL, USERS_BY_STATUS_IN_TENANT_SQL, USERS_BY_STATUS_PAGE_SQL,
    USERS_BY_STATUS_SQL,
};
use crate::repositories::uuid_columns::TextUuid;
use crate::repositories::{ProfileRepository, UserRepository};

/// `SQLite`'s extended result codes for a violated uniqueness constraint:
/// `SQLITE_CONSTRAINT_UNIQUE` and `SQLITE_CONSTRAINT_PRIMARYKEY`. `create` turns one
/// into the same structured error `PostgreSQL`'s `23505` produces, so a duplicate
/// reads identically to a caller whichever engine is underneath.
///
/// Matched on the driver's code rather than its message: the text is not part of
/// `SQLite`'s contract and changes between releases. Which index was hit still has
/// to come from the message — a result code says a unique constraint failed, not
/// which one.
const UNIQUE_VIOLATION_CODES: [&str; 2] = ["2067", "1555"];

/// The structured error for a violated unique constraint on `users`, or `None` when
/// the failure was something else.
///
/// `users` carries a second unique index besides `email`: `idx_users_firebase_uid`,
/// partial over non-null `firebase_uid`. Two concurrent Firebase sign-ins for one UID
/// can both pass `find_or_create_firebase_user`'s "no user for this UID" check and race
/// the insert, and the loser collides on *that* index — so reporting every duplicate as
/// an email collision sends whoever reads the log hunting the wrong column.
fn duplicate_user_error(error: &sqlx::Error) -> Option<AppError> {
    let is_duplicate = error
        .as_database_error()
        .and_then(DatabaseError::code)
        .is_some_and(|code| UNIQUE_VIOLATION_CODES.contains(&code.as_ref()));
    if !is_duplicate {
        return None;
    }
    Some(if error.to_string().contains("firebase_uid") {
        AppError::invalid_input("Firebase account already linked to another user")
    } else {
        AppError::invalid_input("Email already in use by another user")
    })
}

impl_user_repository!(
    Database,
    SqliteRow,
    TextUuid,
    duplicate_user_error,
    SQLITE_USER_PURGE
);
impl_profile_repository!(Database, TextUuid);
