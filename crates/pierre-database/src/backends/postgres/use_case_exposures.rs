// ABOUTME: PostgreSQL-backed UseCaseExposureRepository, emitted from the shared body in repositories/
// ABOUTME: user_id and tenant_id are native uuid columns here, so the shared statements bind them without a textual round-trip
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::use_case_exposures::{
    exposure_from_row, impl_use_case_exposure_repository, ExposureRow, UseCaseExposure,
    UseCaseExposureRepository, RECORD_USE_CASE_SHOWN_SQL, RECORD_USE_CASE_TAPPED_SQL,
    USE_CASE_EXPOSURES_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_use_case_exposure_repository!(PostgresDatabase, NativeUuid);
