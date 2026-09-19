// ABOUTME: SQLite-backed RouteSummaryRepository, emitted from the shared implementation in repositories/route_summaries.rs
// ABOUTME: Ids bind as hyphenated text and the two JSON blobs as TEXT, both through the same binds Postgres uses
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, UserId};
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::route_summaries::{
    impl_route_summary_repository, json_column_value, route_summary_from_row,
    RouteSummaryRepository, GET_ROUTE_SUMMARY_SQL, UPSERT_ROUTE_SUMMARY_SQL,
};

impl_route_summary_repository!(Database);
