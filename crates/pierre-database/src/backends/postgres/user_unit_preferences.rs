// ABOUTME: PostgreSQL-backed UnitPreferencesRepository, emitted from the shared body in repositories/user_unit_preferences.rs
// ABOUTME: Postgres stores user_id as a native uuid column, so the shell passes the native uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::AppResult;
use pierre_core::models::{UnitPreference, UnitSystem};
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::user_unit_preferences::{
    decode_unit_preferences, impl_unit_preferences_repository, unit_preferences_error,
    StoredUnitPreferences, UnitPreferencesRepository, GET_UNIT_PREFERENCES_SQL,
    UPSERT_DEVICE_LOCALE_SQL, UPSERT_PROVIDER_UNITS_SQL, UPSERT_UNIT_PREFERENCE_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_unit_preferences_repository!(PostgresDatabase, NativeUuid);
