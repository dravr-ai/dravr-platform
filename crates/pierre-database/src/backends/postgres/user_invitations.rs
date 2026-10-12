// ABOUTME: PostgreSQL-backed UserInvitationRepository, emitted from the shared implementation in repositories/user_invitations.rs
// ABOUTME: invited_by is a native uuid column here, so the shared statements bind the uuid as itself

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{normalize_email, UserInvitation};
use sqlx::Row;
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::user_invitations::{
    impl_user_invitation_repository, invalid_invitation, invitation_from_row,
    COUNT_RECENT_INVITATIONS_SQL, INSERT_INVITATION_SQL, INVITATION_MAX_ATTEMPTS,
    LIST_LIVE_INVITATIONS_SQL, LOAD_INVITATION_SQL, LOCK_INVITATION_SQL,
    RECORD_INVITATION_ATTEMPT_SQL, REDEEM_INVITATION_SQL, REVOKE_LIVE_INVITATIONS_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;
use crate::repositories::UserInvitationRepository;

impl_user_invitation_repository!(PostgresDatabase, NativeUuid);
