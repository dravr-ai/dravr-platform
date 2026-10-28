// ABOUTME: PostgreSQL-backed ActivityStreamRepository, emitted from the shared implementation in repositories/activity_streams.rs
// ABOUTME: user_id is uuid here (a foreign key to users), so its bound string is cast to ::uuid
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, TimeSeriesData};
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::activity_streams::{
    impl_activity_stream_repository, streams_from_row, streams_to_json, ActivityStreamRepository,
    ActivityStreamsWrite, ActivityWithoutStreams, DELETE_ACTIVITY_STREAMS_PG_SQL,
    GET_ACTIVITY_STREAMS_PG_SQL, LIST_ACTIVITIES_WITHOUT_STREAMS_PG_SQL, STREAMS_FORMAT_VERSION,
    UPSERT_ACTIVITY_STREAMS_PG_SQL,
};

impl_activity_stream_repository!(
    PostgresDatabase,
    GET_ACTIVITY_STREAMS_PG_SQL,
    UPSERT_ACTIVITY_STREAMS_PG_SQL,
    DELETE_ACTIVITY_STREAMS_PG_SQL,
    LIST_ACTIVITIES_WITHOUT_STREAMS_PG_SQL
);
