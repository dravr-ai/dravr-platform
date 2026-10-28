// ABOUTME: SQLite-backed ActivityStreamRepository, emitted from the shared implementation in repositories/activity_streams.rs
// ABOUTME: Every id column is TEXT here, so the binds carry no cast
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{TenantId, TimeSeriesData};
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::activity_streams::{
    impl_activity_stream_repository, streams_from_row, streams_to_json, ActivityStreamRepository,
    ActivityStreamsWrite, ActivityWithoutStreams, DELETE_ACTIVITY_STREAMS_SQL,
    DELETE_PROVIDER_ACTIVITY_STREAMS_SQL, GET_ACTIVITY_STREAMS_SQL,
    LIST_ACTIVITIES_WITHOUT_CURRENT_STREAMS_SQL, STREAMS_FORMAT_VERSION,
    UPSERT_ACTIVITY_STREAMS_SQL,
};

impl_activity_stream_repository!(
    Database,
    GET_ACTIVITY_STREAMS_SQL,
    UPSERT_ACTIVITY_STREAMS_SQL,
    DELETE_ACTIVITY_STREAMS_SQL,
    DELETE_PROVIDER_ACTIVITY_STREAMS_SQL,
    LIST_ACTIVITIES_WITHOUT_CURRENT_STREAMS_SQL
);
