// ABOUTME: SQLite-backed UnitPreferencesRepository, emitted from the shared body in repositories/user_unit_preferences.rs
// ABOUTME: SQLite stores user_id as hyphenated TEXT, so the shell passes the text uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::AppResult;
use pierre_core::models::UnitSystem;
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::user_unit_preferences::{
    decode_units, impl_unit_preferences_repository, unit_preferences_error,
    UnitPreferencesRepository, GET_UNITS_SQL, UPSERT_UNITS_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_unit_preferences_repository!(Database, TextUuid);
