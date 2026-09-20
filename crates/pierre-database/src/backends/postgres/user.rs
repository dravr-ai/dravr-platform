// ABOUTME: PostgreSQL-backed UserRepository and ProfileRepository, emitted from the shared bodies in repositories/users.rs
// ABOUTME: and repositories/user_profiles.rs; uuid columns are native here, and a duplicate reads off the SQLSTATE and constraint
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{CoachingPersona, TenantId, User, UserStatus, UserTier};
use pierre_core::pagination::{Cursor, CursorPage, PaginationParams};
use pierre_core::permissions::UserRole;
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::Row;
use std::collections::HashMap;
use uuid::Uuid;

use super::user_preferences as preferences;
use crate::backends::postgres::PostgresDatabase;
use crate::backends::shared::enums::user_status_to_str;
use crate::repositories::user_profiles::{
    apply_progress_fields, impl_profile_repository, CREATE_GOAL_SQL, GET_USER_CONFIGURATION_SQL,
    GET_USER_GOALS_SQL, GET_USER_GOAL_SQL, GET_USER_PROFILE_SQL, SAVE_USER_CONFIGURATION_SQL,
    UPDATE_USER_GOAL_SQL, UPSERT_USER_PROFILE_SQL,
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
use crate::repositories::uuid_columns::NativeUuid;
use crate::repositories::{ProfileRepository, UserRepository};

/// `PostgreSQL`'s SQLSTATE for a violated unique constraint. `create` turns one into
/// the same structured error `SQLite`'s `UNIQUE constraint failed` produces, so a
/// duplicate reads identically to a caller whichever engine is underneath.
const UNIQUE_VIOLATION: &str = "23505";

/// The structured error for a violated unique constraint on `users`, or `None` when the
/// failure was something else.
///
/// `users` carries a second unique index besides `email`: `idx_users_firebase_uid`,
/// partial over non-null `firebase_uid`. Two concurrent Firebase sign-ins for one UID can
/// both pass `find_or_create_firebase_user`'s "no user for this UID" check and race the
/// insert, and the loser collides on *that* index — so reporting every duplicate as an
/// email collision sends whoever reads the log hunting the wrong column. Postgres names
/// the constraint, so this asks it rather than guessing.
fn duplicate_user_error(error: &sqlx::Error) -> Option<AppError> {
    let sqlx::Error::Database(db) = error else {
        return None;
    };
    if db.code().as_deref() != Some(UNIQUE_VIOLATION) {
        return None;
    }
    let names_firebase_uid = db
        .constraint()
        .is_some_and(|name| name.contains("firebase_uid"));
    Some(if names_firebase_uid {
        AppError::invalid_input("Firebase account already linked to another user")
    } else {
        AppError::invalid_input("Email already in use by another user")
    })
}

impl_user_repository!(PostgresDatabase, PgRow, NativeUuid, duplicate_user_error);
impl_profile_repository!(PostgresDatabase, NativeUuid);
