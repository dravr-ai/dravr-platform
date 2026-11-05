// ABOUTME: SQLite-backed CalendarPreferenceRepository, emitted from the shared body in repositories/calendar_preferences.rs
// ABOUTME: SQLite stores user ids as hyphenated TEXT, so the shell passes the text uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::calendar_preferences::{
    impl_calendar_preference_repository, preference_error, CalendarPreference,
    CalendarPreferenceRepository, CLEAR_CALENDAR_PREFERENCE_SQL, GET_CALENDAR_PREFERENCE_SQL,
    SET_CALENDAR_PREFERENCE_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_calendar_preference_repository!(Database, TextUuid);
