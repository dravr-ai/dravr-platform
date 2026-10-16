// ABOUTME: SQLite-backed UserInvitationRepository, emitted from the shared implementation in repositories/user_invitations.rs
// ABOUTME: invited_by is a TEXT column here, so the shared statements bind the uuid as hyphenated text

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{normalize_email, UserInvitation};
use sqlx::Row;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::link_token_consume::consume_link_token;
use crate::repositories::user_invitations::{
    impl_user_invitation_repository, invitation_from_row, COUNT_RECENT_INVITATIONS_SQL,
    INSERT_INVITATION_SQL, INVITATION_LINK_TOKEN, LIST_LIVE_INVITATIONS_SQL,
    REVOKE_LIVE_INVITATIONS_SQL,
};
use crate::repositories::uuid_columns::TextUuid;
use crate::repositories::UserInvitationRepository;

impl_user_invitation_repository!(Database, TextUuid);
