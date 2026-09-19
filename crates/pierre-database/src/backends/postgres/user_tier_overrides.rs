// ABOUTME: PostgreSQL-backed UserTierOverrideRepository, emitted from the shared body in repositories/user_tier_overrides.rs
// ABOUTME: Postgres stores user_id and set_by as native uuid columns, so the shell passes the native uuid codec
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::user_tier_overrides::{
    impl_user_tier_override_repository, tier_from_column, tier_override_column_error,
    UserTierOverride, UserTierOverrideRepository, DELETE_TIER_OVERRIDE_SQL, GET_TIER_OVERRIDE_SQL,
    UPSERT_TIER_OVERRIDE_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_user_tier_override_repository!(PostgresDatabase, NativeUuid);
