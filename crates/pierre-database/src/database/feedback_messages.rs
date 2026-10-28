// ABOUTME: SQLite-backed FeedbackMessageRepository, emitted from the shared body in repositories/
// ABOUTME: TEXT ids through the TextUuid codec; the statements are the ones Postgres runs
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::feedback_messages::{
    about_from_columns, impl_feedback_message_repository, instant_from_millis, FeedbackMessage,
    FeedbackMessageRepository, NewFeedbackMessage, INSERT_FEEDBACK_MESSAGE_SQL,
    LIST_FEEDBACK_MESSAGES_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_feedback_message_repository!(Database, TextUuid);
