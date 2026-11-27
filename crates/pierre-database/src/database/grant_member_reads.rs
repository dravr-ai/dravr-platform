// ABOUTME: SQLite-backed GrantMemberReadRepository, emitted from the shared body in repositories/
// ABOUTME: user_id is a TEXT column here, so the shared statements bind it as hyphenated text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::grant_member_reads::{
    impl_grant_member_read_repository, read_at_from_ms, GrantMemberReadRepository,
    GRANT_MEMBER_READ_AT_SQL, RECORD_GRANT_MEMBER_READ_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_grant_member_read_repository!(Database, TextUuid);
