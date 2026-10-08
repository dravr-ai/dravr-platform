// ABOUTME: SQLite-backed HomePreferencesRepository, emitted from the shared body in repositories/user_home_preferences.rs
// ABOUTME: SQLite stores user_id as hyphenated TEXT, so the shell passes the text uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::AppResult;
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::user_home_preferences::{
    home_preferences_error, impl_home_preferences_repository, HomePreferences,
    HomePreferencesRepository, GET_HOME_PREFERENCES_SQL, UPSERT_HOME_PREFERENCES_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_home_preferences_repository!(Database, TextUuid);
