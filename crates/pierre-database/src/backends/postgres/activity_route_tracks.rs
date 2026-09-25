// ABOUTME: PostgreSQL-backed ActivityRouteTrackRepository, emitted from the shared implementation in repositories/activity_route_tracks.rs
// ABOUTME: Every id column is TEXT here, as on SQLite, so the binds carry no cast
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::activity_route_tracks::{
    impl_activity_route_track_repository, route_track_columns, route_track_from_row,
    ActivityRouteTrackRepository, StoredRouteTrack, GET_ROUTE_TRACK_SQL, UPSERT_ROUTE_TRACK_SQL,
};

impl_activity_route_track_repository!(PostgresDatabase);
