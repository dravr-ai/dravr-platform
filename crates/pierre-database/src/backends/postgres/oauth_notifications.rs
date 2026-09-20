// ABOUTME: PostgreSQL-backed NotificationRepository, emitted from the shared implementation in repositories/notifications.rs
// ABOUTME: user_id is a native uuid column here, so the shared statements bind the uuid as itself
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::OAuthNotification;
use tracing::debug;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::notifications::{
    all_oauth_notifications_sql, impl_notification_repository, oauth_notification_from_row,
    NotificationRepository, MARK_ALL_OAUTH_NOTIFICATIONS_READ_SQL,
    MARK_OAUTH_NOTIFICATION_READ_SQL, STORE_OAUTH_NOTIFICATION_SQL, UNREAD_OAUTH_NOTIFICATIONS_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_notification_repository!(PostgresDatabase, NativeUuid);
