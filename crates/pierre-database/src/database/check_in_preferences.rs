// ABOUTME: SQLite-backed CheckInPreferenceRepository, emitted from the shared body in repositories/
// ABOUTME: tenant_id and user_id are TEXT columns here, so the shared statements bind them as hyphenated text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{CheckInCadence, TenantId};
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::check_in_preferences::{
    cadence_from_column, impl_check_in_preference_repository, CheckInPreferenceRepository,
    CheckInSubscriber, CHECK_IN_CADENCE_SQL, CHECK_IN_SUBSCRIBERS_SQL, SET_CHECK_IN_CADENCE_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_check_in_preference_repository!(Database, TextUuid);
