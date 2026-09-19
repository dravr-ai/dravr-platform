// ABOUTME: PostgreSQL-backed RouteSummaryRepository, emitted from the shared implementation in repositories/route_summaries.rs
// ABOUTME: Ids bind as native uuid and the two JSON blobs as jsonb, both through the same binds SQLite uses
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, UserId};
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::route_summaries::{
    impl_route_summary_repository, json_column_value, route_summary_from_row,
    RouteSummaryRepository, GET_ROUTE_SUMMARY_SQL, UPSERT_ROUTE_SUMMARY_SQL,
};

impl_route_summary_repository!(PostgresDatabase);
