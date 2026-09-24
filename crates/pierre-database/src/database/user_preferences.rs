// ABOUTME: SQLite writes for the single-column preferences hanging off the users row
// ABOUTME: Shell over the shared body; SQLite stores users.id as TEXT, so the id binds stringified
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Preference writes for the `SQLite` backend.
//!
//! The statements and the bodies live in
//! [`crate::repositories::user_preferences`]; this module only names the type
//! the pool is parameterised on and how a user id reaches a `TEXT` column.

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::CoachingPersona;
use sqlx::{Pool, Sqlite};
use uuid::Uuid;

use crate::repositories::user_preferences::{
    impl_user_preferences, GET_PROVIDER_TERMS_SQL, SET_ANALYTICS_CONSENT_SQL,
    SET_COACHING_PERSONA_SQL, SET_LOCALE_SQL, SET_MANAGES_ROSTER_SQL, SET_PROVIDER_TERMS_SQL,
    SET_THEME_SQL, SET_TIMEZONE_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_user_preferences!(Sqlite, TextUuid::bind);
