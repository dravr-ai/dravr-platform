// ABOUTME: PostgreSQL-backed ActivityConversationRepository, emitted from the shared implementation in repositories/activity_conversations.rs
// ABOUTME: The link's ids are TEXT here, as on SQLite; the conversation's uuid owner is compared as text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use sqlx::Row;
use uuid::Uuid;

use super::PostgresDatabase;
use crate::repositories::activity_conversations::{
    impl_activity_conversation_repository, ActivityConversationRepository,
    GET_ACTIVITY_CONVERSATION_SQL, LINK_ACTIVITY_CONVERSATION_SQL,
    UNLINK_ACTIVITY_CONVERSATION_SQL,
};

impl_activity_conversation_repository!(PostgresDatabase);
