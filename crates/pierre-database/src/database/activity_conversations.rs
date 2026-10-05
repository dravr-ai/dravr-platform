// ABOUTME: SQLite-backed ActivityConversationRepository, emitted from the shared implementation in repositories/activity_conversations.rs
// ABOUTME: Every id column is TEXT here, so the binds carry no cast
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use pierre_core::transport::TransportPolicy;
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::activity_conversations::{
    impl_activity_conversation_repository, ActivityConversationLink,
    ActivityConversationRepository, GET_ACTIVITY_CONVERSATION_SQL, LINK_ACTIVITY_CONVERSATION_SQL,
    LIST_ACTIVITY_CONVERSATION_LINKS_SQL, UNLINK_ACTIVITY_CONVERSATION_SQL,
};

impl_activity_conversation_repository!(Database);
