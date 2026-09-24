// ABOUTME: PostgreSQL writes for the single-column preferences hanging off the users row
// ABOUTME: Shell over the shared body; Postgres stores users.id as uuid, so the id binds natively
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Preference writes for the `PostgreSQL` backend.
//!
//! The statements and the bodies live in
//! [`crate::repositories::user_preferences`]; this module only names the type
//! the pool is parameterised on and how a user id reaches a `uuid` column.

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::CoachingPersona;
use sqlx::{Pool, Postgres};
use uuid::Uuid;

use crate::repositories::user_preferences::{
    impl_user_preferences, GET_TRAININGPEAKS_TERMS_SQL, SET_ANALYTICS_CONSENT_SQL,
    SET_COACHING_PERSONA_SQL, SET_LOCALE_SQL, SET_MANAGES_ROSTER_SQL, SET_THEME_SQL,
    SET_TIMEZONE_SQL, SET_TRAININGPEAKS_TERMS_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_user_preferences!(Postgres, NativeUuid::bind);
