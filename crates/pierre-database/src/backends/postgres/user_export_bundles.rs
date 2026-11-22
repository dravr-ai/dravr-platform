// ABOUTME: PostgreSQL-backed UserExportBundleRepository, emitted from the shared implementation in repositories/user_export_bundles.rs
// ABOUTME: Every id column is TEXT here, so the binds carry no cast
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::user_export_bundles::{
    count, impl_user_export_bundle_repository, stored, BundleContents, BundleStatus, ExportBundle,
    UserExportBundleRepository, BUILDING_SQL, DELETE_SQL, EXPIRED_SQL, FINISH_FAILED_SQL,
    FINISH_READY_SQL, GET_SQL, HEARTBEAT_SQL, INSERT_BUILDING_SQL, LATEST_SQL, STALE_BUILDING_SQL,
    SWEEP_BATCH,
};

impl_user_export_bundle_repository!(PostgresDatabase, PgRow);
