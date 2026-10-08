// ABOUTME: SQLite-backed UseCaseExposureRepository, emitted from the shared body in repositories/
// ABOUTME: user_id and tenant_id are TEXT columns here, so the shared statements bind them as hyphenated text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::use_case_exposures::{
    exposure_from_row, impl_use_case_exposure_repository, ExposureRow, UseCaseExposure,
    UseCaseExposureRepository, RECORD_USE_CASE_SHOWN_SQL, RECORD_USE_CASE_TAPPED_SQL,
    USE_CASE_EXPOSURES_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_use_case_exposure_repository!(Database, TextUuid);
