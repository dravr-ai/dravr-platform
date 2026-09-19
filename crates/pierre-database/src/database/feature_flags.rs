// ABOUTME: SQLite-backed FeatureFlagsRepository, emitted from the shared implementation in repositories/feature_flags.rs
// ABOUTME: The id columns are TEXT here, so uuids bind as hyphenated text and updated_by reads back with no cast

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::feature_flags::FeatureKey;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::feature_flags::{
    feature_flag_columns, flags_from_rows, impl_feature_flags_repository, list_tenant_defaults_sql,
    list_user_overrides_sql, FeatureFlagRow, FeatureFlagsRepository, CLEAR_TENANT_DEFAULT_SQL,
    CLEAR_USER_OVERRIDE_SQL, SET_TENANT_DEFAULT_SQL, SET_USER_OVERRIDE_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_feature_flags_repository!(Database, TextUuid::bind, "");
