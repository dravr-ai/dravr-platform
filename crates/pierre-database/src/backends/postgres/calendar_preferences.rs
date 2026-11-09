// ABOUTME: PostgreSQL-backed CalendarPreferenceRepository, emitted from the shared body in repositories/calendar_preferences.rs
// ABOUTME: Postgres stores user ids as native uuid columns, so the shell passes the native uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::calendar_preferences::{
    impl_calendar_preference_repository, preference_error, CalendarPreference,
    CalendarPreferenceRepository, CLEAR_CALENDAR_PREFERENCE_SQL, GET_CALENDAR_PREFERENCE_SQL,
    SET_CALENDAR_PREFERENCE_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_calendar_preference_repository!(PostgresDatabase, NativeUuid);
