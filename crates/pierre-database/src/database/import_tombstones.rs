// ABOUTME: SQLite-backed ImportTombstoneRepository, emitted from the shared body in repositories/import_tombstones.rs
// ABOUTME: A deleted account's provider accounts and email, by hash, refused until explicitly cleared
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};

use crate::database::Database;
use crate::repositories::import_tombstones::{
    impl_import_tombstone_repository, tombstone_from_row, ImportTombstone,
    ImportTombstoneRepository, TombstoneKind, CLAIM_TOMBSTONE_SQL, CLEAR_TOMBSTONE_SQL,
    FIND_LIVE_TOMBSTONE_SQL, GET_TOMBSTONE_SQL, LIST_ALL_TOMBSTONES_SQL, LIST_LIVE_TOMBSTONES_SQL,
};

impl_import_tombstone_repository!(Database);
