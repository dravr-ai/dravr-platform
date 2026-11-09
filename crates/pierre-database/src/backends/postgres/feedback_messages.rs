// ABOUTME: PostgreSQL-backed FeedbackMessageRepository, emitted from the shared body in repositories/
// ABOUTME: Native uuid ids through the NativeUuid codec; the statements are the ones SQLite runs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use sqlx::postgres::PgRow;
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::feedback_messages::{
    about_from_columns, image_from_columns, impl_feedback_message_repository, instant_from_millis,
    FeedbackAuthor, FeedbackMessage, FeedbackMessageRepository, NewFeedbackMessage,
    COUNT_UNREAD_TEAM_REPLIES_SQL, GET_FEEDBACK_MESSAGE_SQL, INSERT_FEEDBACK_MESSAGE_SQL,
    LIST_FEEDBACK_MESSAGES_SQL, MARK_TEAM_REPLIES_READ_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_feedback_message_repository!(PostgresDatabase, NativeUuid, PgRow);
