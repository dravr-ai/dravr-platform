// ABOUTME: PostgreSQL-backed AthleteGearRepository, emitted from the shared body in repositories/athlete_gear.rs
// ABOUTME: Postgres stores the ledger's ids as native uuid columns, so the shell passes its row type and the native uuid codec

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{
    shoe_sport_slug, ActivityGearKey, AthleteGear, GearAttribution, GearDeclaration,
    MAX_GEAR_PER_ATHLETE,
};
use sqlx::postgres::PgRow;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::athlete_gear::{
    attribution_from_row, gear_error, gear_from_row, impl_athlete_gear_repository,
    AthleteGearRepository, GearDeletion, COUNT_GEAR_SQL, DELETE_ATTRIBUTION_SQL, DELETE_GEAR_SQL,
    GET_GEAR_SQL, HAND_OVER_ATTRIBUTION_SQL, INSERT_DEFAULT_ATTRIBUTION_SQL, INSERT_GEAR_SQL,
    LIST_ATTRIBUTIONS_SQL, LIST_GEAR_SQL, RESTORE_GEAR_SQL, RETIRE_GEAR_SQL, UNPIN_SPORT_SQL,
    UPDATE_ATTRIBUTION_DISTANCE_SQL, UPDATE_GEAR_SQL, UPSERT_DECLARED_ATTRIBUTION_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_athlete_gear_repository!(PostgresDatabase, PgRow, NativeUuid);
