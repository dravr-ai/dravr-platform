// ABOUTME: PostgreSQL-backed HomePreferencesRepository, emitted from the shared body in repositories/user_home_preferences.rs
// ABOUTME: Postgres stores user_id as a native uuid column, so the shell passes the native uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::AppResult;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::user_home_preferences::{
    home_preferences_error, impl_home_preferences_repository, HomePreferences,
    HomePreferencesRepository, GET_HOME_PREFERENCES_SQL, UPSERT_HOME_PREFERENCES_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_home_preferences_repository!(PostgresDatabase, NativeUuid);
