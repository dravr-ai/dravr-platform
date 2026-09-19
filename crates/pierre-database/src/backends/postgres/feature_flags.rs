// ABOUTME: PostgreSQL-backed FeatureFlagsRepository, emitted from the shared implementation in repositories/feature_flags.rs
// ABOUTME: The id columns are native uuids here, so uuids bind unwrapped and updated_by reads back through a ::text cast

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::feature_flags::FeatureKey;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::feature_flags::{
    feature_flag_columns, flags_from_rows, impl_feature_flags_repository, list_tenant_defaults_sql,
    list_user_overrides_sql, FeatureFlagRow, FeatureFlagsRepository, CLEAR_TENANT_DEFAULT_SQL,
    CLEAR_USER_OVERRIDE_SQL, SET_TENANT_DEFAULT_SQL, SET_USER_OVERRIDE_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_feature_flags_repository!(PostgresDatabase, NativeUuid::bind, "::text");
