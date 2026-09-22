// ABOUTME: SQLite-backed StravaSeatReclaimWarningRepository, emitted from the shared body in repositories/
// ABOUTME: user_id and tenant_id are TEXT columns here, so the shared statements bind them as hyphenated text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{StravaSeatReclaimWarning, TenantId};
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::strava_seat_reclaim_warnings::{
    impl_strava_seat_reclaim_warning_repository, warned_at_from_ms,
    StravaSeatReclaimWarningRepository, CLEAR_SEAT_RECLAIM_WARNING_SQL,
    GET_SEAT_RECLAIM_WARNING_SQL, RECORD_SEAT_RECLAIM_WARNING_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_strava_seat_reclaim_warning_repository!(Database, TextUuid);
