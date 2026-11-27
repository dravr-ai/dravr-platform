// ABOUTME: SQLite-backed AthletePlaceRepository, emitted from the shared implementation in repositories/athlete_places.rs
// ABOUTME: Every id column is TEXT here, as on Postgres, so the binds carry no cast
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_arpenteur::HomeDeclaration;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{CachedRowKey, StoredHome, TenantId};

use crate::database::Database;
use crate::repositories::activity_cache::sql::{ASSIGN_PLACE_SQL, CLEAR_DANGLING_PLACES_SQL};
use crate::repositories::athlete_places::{
    athlete_place_from_row, cached_rows_stamp_from_row, due_users_from_rows, home_columns,
    impl_athlete_place_repository, integer_column, log_skipped_due_users, stored_home_from_row,
    AthletePlace, AthletePlaceRepository, CachedRowsStamp, PlaceNameSource, PlaceRun,
    CACHED_ROWS_STAMP_SQL, CLEAR_PLACE_NAME_SQL, DELETE_UNNAMED_PLACES_SQL, DUE_PLACE_USERS_SQL,
    GET_PLACE_SQL, HOME_DECLARATION_SQL, INSERT_ATHLETE_PLACE_SQL, LIST_PLACES_SQL, NAME_PLACE_SQL,
    RECORD_FAILED_PLACE_RUN_SQL, STORE_HOME_DECLARATION_SQL, UPSERT_CLUSTERED_PLACE_SQL,
    UPSERT_PLACE_RUN_SQL,
};

impl_athlete_place_repository!(Database);
