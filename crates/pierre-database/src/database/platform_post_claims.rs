// ABOUTME: SQLite-backed PlatformPostClaimRepository, emitted from the shared body in repositories/
// ABOUTME: tenant_id and user_id are TEXT columns here, so the shared statements bind them as text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::platform_post_claims::{
    impl_platform_post_claim_repository, PlatformPostClaimRepository, CLAIM_PLATFORM_POST_ONCE_SQL,
    CLAIM_PLATFORM_POST_SINCE_SQL, MARK_PLATFORM_POST_POSTED_SQL, RELEASE_PLATFORM_POST_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_platform_post_claim_repository!(Database, TextUuid);
