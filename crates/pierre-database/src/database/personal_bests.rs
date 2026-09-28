// ABOUTME: SQLite-backed PersonalBestRepository, emitted from the shared body in repositories/
// ABOUTME: user_id and tenant_id are TEXT columns here, so the shared statements bind them as hyphenated text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::personal_bests::{
    impl_personal_best_repository, PersonalBest, PersonalBestRepository, PersonalBestSeed,
    PersonalBestSeedCandidate, GET_PERSONAL_BEST_SEED_SQL, IS_ACTIVITY_SCANNED_SQL,
    LIST_PERSONAL_BESTS_SQL, LIST_PERSONAL_BEST_SEED_CANDIDATES_SQL, RECORD_ACTIVITY_SCAN_SQL,
    RECORD_PERSONAL_BEST_SQL, SAVE_PERSONAL_BEST_SEED_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_personal_best_repository!(Database, TextUuid);
