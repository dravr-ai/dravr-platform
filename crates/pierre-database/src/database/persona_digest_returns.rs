// ABOUTME: SQLite-backed PersonaDigestReturnRepository, emitted from the shared body in repositories/
// ABOUTME: user_id, tenant_id and batch_id are TEXT columns here, so the shared statements bind them as text
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::TenantId;
use uuid::Uuid;

use crate::database::Database;
use crate::repositories::persona_digest_returns::{
    impl_persona_digest_return_repository, returned_at_from_ms, PersonaDigestReturnRepository,
    CLAIM_PERSONA_DIGEST_ITEM_SQL, LAST_PERSONA_DIGEST_AT_SQL,
    PERSONA_DIGEST_ITEMS_RETURNED_SINCE_SQL, RELEASE_PERSONA_DIGEST_BATCH_SQL,
};
use crate::repositories::uuid_columns::TextUuid;

impl_persona_digest_return_repository!(Database, TextUuid);
