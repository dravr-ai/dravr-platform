// ABOUTME: PostgreSQL-backed GrantMemberReadRepository, emitted from the shared body in repositories/
// ABOUTME: user_id is a native uuid column here, bound without a textual round-trip
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_tronc::dates::Timestamp;
use pierre_core::errors::{AppError, AppResult};
use uuid::Uuid;

use crate::backends::postgres::PostgresDatabase;
use crate::repositories::grant_member_reads::{
    impl_grant_member_read_repository, read_at_from_ms, GrantMemberReadRepository,
    GRANT_MEMBER_READ_AT_SQL, RECORD_GRANT_MEMBER_READ_SQL,
};
use crate::repositories::uuid_columns::NativeUuid;

impl_grant_member_read_repository!(PostgresDatabase, NativeUuid);
