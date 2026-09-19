// ABOUTME: SQLite-backed RosterRepository, emitted from the shared implementation in repositories/roster.rs
// ABOUTME: Every id column is TEXT here, so uuids bind as hyphenated text and read back with no cast

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{CoachAthleteAssignment, TenantId};
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::roster::{
    assignment_columns, assignment_from_row, impl_roster_repository, list_athletes_for_coach_sql,
    list_coaches_for_athlete_sql, RosterRepository, ASSIGN_ATHLETE_SQL,
    COUNT_ACTIVE_ASSIGNMENTS_SQL, REVOKE_ASSIGNMENT_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_roster_repository!(Database, TextUuid::bind, "");
